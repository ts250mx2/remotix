import { Hono } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { z } from 'zod';
import { eq } from 'drizzle-orm';
import { getCookie } from 'hono/cookie';
import { db, tables } from '../db/index.js';
import { newId, newAccessKey, newAgentSecret } from '../ids.js';
import { hashSecret } from '../auth/password.js';
import { getSession, COOKIE_NAME } from '../auth/session.js';
import { userCanAccessDevice } from '../access.js';
import { deviceHub } from '../devices/hub.js';
import { reserveRemoteSession } from '../ws/signaling.js';

const registerSchema = z.object({
  name: z.string().min(1).max(120),
  os: z.string().max(64).optional(),
  hostname: z.string().max(255).optional(),
});
const connectSchema = z.object({ accessKey: z.string().min(6).max(40) });

function normalizeKey(k: string): string {
  return k.replace(/[^0-9A-Za-z]/g, '').toUpperCase();
}

export const deviceRoutes = new Hono()
  // El exe se registra UNA vez (sin login) y guarda (deviceId, accessKey, secret).
  // La accessKey es FIJA y permanente; queda sin dueño hasta que un usuario la
  // reclama (al loguearse en el exe, vía POST /api/devices/claim).
  .post('/register', zValidator('json', registerSchema), async (c) => {
    const { name, os, hostname } = c.req.valid('json');
    const id = newId('dv');
    const accessKey = newAccessKey();
    const secret = newAgentSecret();
    const secretHash = await hashSecret(secret);
    await db.insert(tables.devices).values({ id, accessKey, secretHash, name, os, hostname, createdAt: new Date() });
    return c.json({ deviceId: id, accessKey, secret, name }, 201);
  })

  // Conectarse por la clave fija (consola del operador), estilo TeamViewer:
  //   - Equipo SIN dueño → solo con la clave, SIN necesidad de login.
  //   - Equipo CON dueño → requiere usuario logueado con acceso (dueño/grant/grupo).
  .post('/connect', zValidator('json', connectSchema), async (c) => {
    const key = normalizeKey(c.req.valid('json').accessKey);
    const dev = (await db.select().from(tables.devices).where(eq(tables.devices.accessKey, key)))[0];
    if (!dev) return c.json({ error: 'not_found' }, 404);
    if (dev.ownerId) {
      const token = getCookie(c, COOKIE_NAME);
      const session = token ? await getSession(token) : null;
      if (!session) return c.json({ error: 'unauthenticated' }, 401);
      const role = await userCanAccessDevice(session.userId, dev.id);
      if (!role) return c.json({ error: 'forbidden' }, 403);
    }
    if (!deviceHub.isOnline(dev.id)) return c.json({ error: 'offline' }, 409);
    const code = reserveRemoteSession({ name: dev.name });
    deviceHub.sendToDevice(dev.id, { type: 'start', code });
    return c.json({ code, name: dev.name });
  });
