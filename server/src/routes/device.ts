import { Hono } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { z } from 'zod';
import { eq } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { newId, newAccessKey, newAgentSecret } from '../ids.js';
import { hashSecret } from '../auth/password.js';
import { deviceHub } from '../devices/hub.js';
import { reserveRemoteSession } from '../ws/signaling.js';

const registerSchema = z.object({
  name: z.string().min(1).max(120),
  os: z.string().max(64).optional(),
  hostname: z.string().max(255).optional(),
  machineId: z.string().max(80).optional(),
});
const connectSchema = z.object({ accessKey: z.string().min(6).max(40) });

function normalizeKey(k: string): string {
  return k.replace(/[^0-9A-Za-z]/g, '').toUpperCase();
}

export const deviceRoutes = new Hono()
  // El exe se registra y guarda (deviceId, accessKey, secret). Si manda un
  // `machineId` (Windows MachineGuid) y ya existe un device para esa máquina,
  // se REUTILIZA (misma clave fija) y solo se renueva el secreto — así reinstalar
  // no duplica el equipo. La accessKey es permanente y no cambia.
  .post('/register', zValidator('json', registerSchema), async (c) => {
    const { name, os, hostname, machineId } = c.req.valid('json');
    const secret = newAgentSecret();
    const secretHash = await hashSecret(secret);

    if (machineId) {
      const existing = (await db.select().from(tables.devices).where(eq(tables.devices.machineId, machineId)))[0];
      if (existing) {
        await db.update(tables.devices)
          .set({ secretHash, name, os, hostname, lastSeenAt: new Date() })
          .where(eq(tables.devices.id, existing.id));
        return c.json({ deviceId: existing.id, accessKey: existing.accessKey, secret, name });
      }
    }

    const id = newId('dv');
    const accessKey = newAccessKey();
    await db.insert(tables.devices).values({ id, accessKey, secretHash, name, os, hostname, machineId, createdAt: new Date() });
    return c.json({ deviceId: id, accessKey, secret, name }, 201);
  })

  // Conectarse por la clave fija (estilo TeamViewer): la CLAVE es la credencial,
  // NO requiere iniciar sesión. Si el equipo está en línea, reserva una sala y le
  // ordena compartir; devolvemos el código para la consola/visor.
  .post('/connect', zValidator('json', connectSchema), async (c) => {
    const key = normalizeKey(c.req.valid('json').accessKey);
    const dev = (await db.select().from(tables.devices).where(eq(tables.devices.accessKey, key)))[0];
    if (!dev) return c.json({ error: 'not_found' }, 404);
    if (!deviceHub.isOnline(dev.id)) return c.json({ error: 'offline' }, 409);
    const code = reserveRemoteSession({ name: dev.name });
    if (!deviceHub.sendToDevice(dev.id, { type: 'start', code })) {
      return c.json({ error: 'offline' }, 409);
    }
    return c.json({ code, name: dev.name });
  });
