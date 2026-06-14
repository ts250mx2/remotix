import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { Hono } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { z } from 'zod';
import { eq } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { requireUser } from '../auth/middleware.js';
import { userHasProjectAccess } from './projects.js';
import { chatHub } from '../chat/hub.js';
import { reserveRemoteSession } from '../ws/signaling.js';
import {
  canRemoteControl,
  channelsForPrincipal,
  createChannel,
  empresaMemberIds,
  empresaRoster,
  ensureGeneralChannel,
  getMessageById,
  getMessages,
  isChannelMember,
  persistMessage,
} from '../chat/service.js';

const UPLOAD_DIR = resolve(process.cwd(), 'uploads');
const MAX_UPLOAD = 50 * 1024 * 1024; // 50 MB

const createChannelSchema = z.object({
  empresaId: z.string().regex(/^py_[0-9a-zA-Z]{22}$/),
  name: z.string().min(1).max(80),
  kind: z.enum(['channel', 'dm', 'support']).optional(),
});

const postSchema = z.object({ body: z.string().min(1).max(8000) });

export const chatRoutes = new Hono()
  .use('*', requireUser)

  // Canales de una empresa visibles para el usuario.
  .get('/channels', async (c) => {
    const user = c.get('user');
    const empresaId = c.req.query('empresa_id');
    if (!empresaId) return c.json({ error: 'empresa_id_required' }, 400);
    if (!(await userHasProjectAccess(user.id, empresaId))) return c.json({ error: 'not_found' }, 404);
    await ensureGeneralChannel(empresaId);
    const channels = await channelsForPrincipal(empresaId, user.id);
    return c.json({ channels });
  })

  // Crear un canal (admin/técnico).
  .post('/channels', zValidator('json', createChannelSchema), async (c) => {
    const user = c.get('user');
    const { empresaId, name, kind = 'channel' } = c.req.valid('json');
    const role = await userHasProjectAccess(user.id, empresaId);
    if (role !== 'admin' && role !== 'operator') return c.json({ error: 'forbidden' }, 403);
    // Canal público: todos los miembros de la empresa. Otros: solo el creador.
    const members = kind === 'channel' ? await empresaMemberIds(empresaId) : [user.id];
    const channel = await createChannel(empresaId, name, kind, members);
    return c.json({ channel }, 201);
  })

  // Historial de mensajes de un canal.
  .get('/channels/:id/messages', async (c) => {
    const user = c.get('user');
    const channelId = c.req.param('id');
    if (!(await isChannelMember(channelId, user.id))) return c.json({ error: 'not_found' }, 404);
    const limit = Number(c.req.query('limit') ?? 50);
    const beforeRaw = c.req.query('before');
    const before = beforeRaw ? Number(beforeRaw) : undefined;
    const messages = await getMessages(channelId, limit, before);
    return c.json({ messages });
  })

  // Publicar un mensaje (persiste + difunde en tiempo real).
  .post('/channels/:id/messages', zValidator('json', postSchema), async (c) => {
    const user = c.get('user');
    const channelId = c.req.param('id');
    if (!(await isChannelMember(channelId, user.id))) return c.json({ error: 'not_found' }, 404);
    const { body } = c.req.valid('json');
    const message = await chatHub.postAndBroadcast(channelId, user.id, 'user', body);
    return c.json({ message }, 201);
  })

  // Subir un archivo a un canal (multipart) → crea un mensaje con adjunto.
  .post('/channels/:id/files', async (c) => {
    const user = c.get('user');
    const channelId = c.req.param('id');
    if (!(await isChannelMember(channelId, user.id))) return c.json({ error: 'not_found' }, 404);
    const form = await c.req.parseBody();
    const file = form['file'];
    if (!(file instanceof File)) return c.json({ error: 'file_required' }, 400);
    if (file.size > MAX_UPLOAD) return c.json({ error: 'file_too_large' }, 413);
    const buf = Buffer.from(await file.arrayBuffer());
    const caption = typeof form['body'] === 'string' ? form['body'] : '';

    const msg = await persistMessage(channelId, user.id, 'user', caption, {
      name: file.name || 'archivo',
      size: buf.length,
      mime: file.type || 'application/octet-stream',
    });
    await mkdir(UPLOAD_DIR, { recursive: true });
    await writeFile(resolve(UPLOAD_DIR, msg.id), buf);
    await chatHub.broadcastToChannel(channelId, { type: 'message', message: msg });
    return c.json({ message: msg }, 201);
  })

  // Descargar el adjunto de un mensaje (con control de acceso).
  .get('/files/:messageId', async (c) => {
    const user = c.get('user');
    const messageId = c.req.param('messageId');
    const msg = await getMessageById(messageId);
    if (!msg || !msg.attachment) return c.json({ error: 'not_found' }, 404);
    if (!(await isChannelMember(msg.channelId, user.id))) return c.json({ error: 'not_found' }, 404);
    let data: Buffer;
    try {
      data = await readFile(resolve(UPLOAD_DIR, msg.id));
    } catch {
      return c.json({ error: 'gone' }, 404);
    }
    const name = encodeURIComponent(msg.attachment.name);
    return new Response(new Uint8Array(data), {
      status: 200,
      headers: {
        'content-type': msg.attachment.mime,
        'content-disposition': `attachment; filename*=UTF-8''${name}`,
        'content-length': String(data.length),
      },
    });
  })

  // Lanzar acceso remoto a un PC desde el chat (SOLO técnicos/admin).
  .post('/remote/:equipoId', async (c) => {
    const user = c.get('user');
    const equipoId = c.req.param('equipoId');
    const pc = (await db.select().from(tables.equipos).where(eq(tables.equipos.id, equipoId)))[0];
    if (!pc) return c.json({ error: 'not_found' }, 404);
    if (!(await canRemoteControl(user.id, pc.projectId))) return c.json({ error: 'forbidden' }, 403);
    if (!chatHub.isOnline(equipoId)) return c.json({ error: 'pc_offline' }, 409);

    const code = reserveRemoteSession({ name: pc.name });
    chatHub.sendToPrincipal(equipoId, { type: 'remote-invite', code, from: user.name });
    return c.json({ code, equipoId });
  })

  // Roster de la empresa (PCs + usuarios, con presencia).
  .get('/empresas/:id/roster', async (c) => {
    const user = c.get('user');
    const empresaId = c.req.param('id');
    if (!(await userHasProjectAccess(user.id, empresaId))) return c.json({ error: 'not_found' }, 404);
    const roster = await empresaRoster(empresaId, (id) => chatHub.isOnline(id));
    return c.json({ roster });
  });
