import { Hono } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { z } from 'zod';
import { eq } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { requireUser } from '../auth/middleware.js';
import { userHasProjectAccess } from './projects.js';
import { newPin } from '../ids.js';
import { hashSecret } from '../auth/password.js';

const updateSchema = z.object({
  name: z.string().min(1).max(120).optional(),
  pinMode: z.enum(['none', 'required']).optional(),
});

export const equipoRoutes = new Hono()
  .use('*', requireUser)

  // Listar equipos visibles para el usuario, opcionalmente filtrados por proyecto.
  .get('/', async (c) => {
    const user = c.get('user');
    const projectId = c.req.query('project_id');
    if (!projectId) return c.json({ error: 'project_id_required' }, 400);
    const role = await userHasProjectAccess(user.id, projectId);
    if (!role) return c.json({ error: 'not_found' }, 404);
    const rows = await db.select({
      id: tables.equipos.id,
      projectId: tables.equipos.projectId,
      name: tables.equipos.name,
      os: tables.equipos.os,
      hostname: tables.equipos.hostname,
      pinMode: tables.equipos.pinMode,
      lastSeenAt: tables.equipos.lastSeenAt,
      createdAt: tables.equipos.createdAt,
    }).from(tables.equipos).where(eq(tables.equipos.projectId, projectId));
    return c.json({ equipos: rows });
  })

  .get('/:id', async (c) => {
    const user = c.get('user');
    const id = c.req.param('id');
    const eq_ = (await db.select().from(tables.equipos).where(eq(tables.equipos.id, id)))[0];
    if (!eq_) return c.json({ error: 'not_found' }, 404);
    const role = await userHasProjectAccess(user.id, eq_.projectId);
    if (!role) return c.json({ error: 'not_found' }, 404);
    const { agentSecretHash, pinHash, ...safe } = eq_;
    return c.json({ equipo: safe, role });
  })

  .patch('/:id', zValidator('json', updateSchema), async (c) => {
    const user = c.get('user');
    const id = c.req.param('id');
    const eq_ = (await db.select().from(tables.equipos).where(eq(tables.equipos.id, id)))[0];
    if (!eq_) return c.json({ error: 'not_found' }, 404);
    const role = await userHasProjectAccess(user.id, eq_.projectId);
    if (role !== 'admin') return c.json({ error: 'forbidden' }, 403);
    const body = c.req.valid('json');
    const updates: Partial<typeof tables.equipos.$inferInsert> = {};
    if (body.name !== undefined) updates.name = body.name;
    if (body.pinMode !== undefined) {
      updates.pinMode = body.pinMode;
      if (body.pinMode === 'none') updates.pinHash = null;
    }
    if (Object.keys(updates).length > 0) {
      await db.update(tables.equipos).set(updates).where(eq(tables.equipos.id, id));
    }
    return c.json({ ok: true });
  })

  // Regenerar el PIN de conexión. Devuelve el PIN claro UNA SOLA VEZ.
  .post('/:id/pin', async (c) => {
    const user = c.get('user');
    const id = c.req.param('id');
    const eq_ = (await db.select().from(tables.equipos).where(eq(tables.equipos.id, id)))[0];
    if (!eq_) return c.json({ error: 'not_found' }, 404);
    const role = await userHasProjectAccess(user.id, eq_.projectId);
    if (role !== 'admin') return c.json({ error: 'forbidden' }, 403);
    const pin = newPin();
    const pinHash = await hashSecret(pin);
    await db.update(tables.equipos)
      .set({ pinHash, pinMode: 'required' })
      .where(eq(tables.equipos.id, id));
    return c.json({ pin });
  })

  .delete('/:id', async (c) => {
    const user = c.get('user');
    const id = c.req.param('id');
    const eq_ = (await db.select().from(tables.equipos).where(eq(tables.equipos.id, id)))[0];
    if (!eq_) return c.json({ error: 'not_found' }, 404);
    const role = await userHasProjectAccess(user.id, eq_.projectId);
    if (role !== 'admin') return c.json({ error: 'forbidden' }, 403);
    await db.delete(tables.equipos).where(eq(tables.equipos.id, id));
    return c.json({ ok: true });
  });
