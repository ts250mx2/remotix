import { Hono } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { z } from 'zod';
import { eq } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { newId, newAccessKey, newAgentSecret } from '../ids.js';
import { hashSecret } from '../auth/password.js';
import { deviceHub } from '../devices/hub.js';
import { reserveRemoteSession } from '../ws/signaling.js';

const registerSchema = z.object({ name: z.string().min(1).max(120) });
const connectSchema = z.object({ accessKey: z.string().min(6).max(40) });

function normalizeKey(k: string): string {
  return k.replace(/[^0-9A-Za-z]/g, '').toUpperCase();
}

export const deviceRoutes = new Hono()
  // El Lite se registra UNA vez y guarda (deviceId, accessKey, secret).
  // La accessKey es FIJA y permanente; no cambia entre arranques.
  .post('/register', zValidator('json', registerSchema), async (c) => {
    const { name } = c.req.valid('json');
    const id = newId('dv');
    const accessKey = newAccessKey();
    const secret = newAgentSecret();
    const secretHash = await hashSecret(secret);
    await db.insert(tables.devices).values({ id, accessKey, secretHash, name, createdAt: new Date() });
    return c.json({ deviceId: id, accessKey, secret, name }, 201);
  })

  // El técnico se conecta por la clave: si el equipo está en línea, se reserva
  // una sala y se le ordena compartir; devolvemos el código para la consola.
  .post('/connect', zValidator('json', connectSchema), async (c) => {
    const key = normalizeKey(c.req.valid('json').accessKey);
    const dev = (await db.select().from(tables.devices).where(eq(tables.devices.accessKey, key)))[0];
    if (!dev) return c.json({ error: 'not_found' }, 404);
    if (!deviceHub.isOnline(dev.id)) return c.json({ error: 'offline' }, 409);
    const code = reserveRemoteSession({ name: dev.name });
    deviceHub.sendToDevice(dev.id, { type: 'start', code });
    return c.json({ code, name: dev.name });
  });
