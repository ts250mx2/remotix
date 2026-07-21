import { Hono } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { z } from 'zod';
import { and, eq } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { requireUser } from '../auth/middleware.js';
import { userCanAccessDevice, listAccessibleDevices } from '../access.js';
import { deviceHub } from '../devices/hub.js';
import { reserveRemoteSession } from '../ws/signaling.js';

const patchSchema = z.object({ name: z.string().min(1).max(120) });
const claimSchema = z.object({ accessKey: z.string().min(6).max(40) });
const accessSchema = z.object({
  principalId: z.string().regex(/^(us|gp)_[0-9a-zA-Z]{22}$/),
});

function normalizeKey(k: string): string {
  return k.replace(/[^0-9A-Za-z]/g, '').toUpperCase();
}

// Carga el device y exige que el usuario sea el dueño. Devuelve el device o null
// (la respuesta de error la pone el caller).
async function loadOwned(deviceId: string, userId: string) {
  const dev = (await db.select().from(tables.devices).where(eq(tables.devices.id, deviceId)))[0];
  if (!dev) return { dev: null, owned: false };
  return { dev, owned: dev.ownerId === userId };
}

export const deviceManageRoutes = new Hono()
  .use('*', requireUser)

  // Libreta: PCs accesibles por el usuario (dueño + grants), con estado online.
  .get('/', async (c) => {
    const user = c.get('user');
    const devices = await listAccessibleDevices(user.id);
    return c.json({ devices });
  })

  // Suscribir un device por su clave fija (self-service). Lo usa el exe al
  // loguearse y el portal ("Agregar PC por clave"). La clave YA es la credencial
  // para conectar, así que conocerla basta para añadir la PC a tu cuenta:
  //   · sin dueño        → el primero que la reclama queda como dueño (gestiona);
  //   · con otro dueño   → te suscribes (auto-acceso) y la ves en tu lista;
  // de modo que una misma PC puede estar en las cuentas de varios usuarios.
  .post('/claim', zValidator('json', claimSchema), async (c) => {
    const user = c.get('user');
    const key = normalizeKey(c.req.valid('json').accessKey);
    const dev = (await db.select().from(tables.devices).where(eq(tables.devices.accessKey, key)))[0];
    if (!dev) return c.json({ error: 'not_found' }, 404);
    if (!dev.ownerId) {
      await db.update(tables.devices).set({ ownerId: user.id }).where(eq(tables.devices.id, dev.id));
    } else if (dev.ownerId !== user.id) {
      await db.insert(tables.deviceAccess)
        .values({ deviceId: dev.id, principalId: user.id, createdAt: new Date() })
        .onDuplicateKeyUpdate({ set: { principalId: user.id } });
    }
    return c.json({ device: { id: dev.id, name: dev.name, accessKey: dev.accessKey } });
  })

  // Darse de baja de una PC compartida (quita TU acceso; no borra la PC). El
  // dueño no puede "desuscribirse": debe eliminarla (DELETE /:id).
  .delete('/:id/subscription', async (c) => {
    const user = c.get('user');
    await db.delete(tables.deviceAccess).where(and(
      eq(tables.deviceAccess.deviceId, c.req.param('id')),
      eq(tables.deviceAccess.principalId, user.id),
    ));
    return c.json({ ok: true });
  })

  // Renombrar (solo dueño).
  .patch('/:id', zValidator('json', patchSchema), async (c) => {
    const user = c.get('user');
    const { dev, owned } = await loadOwned(c.req.param('id'), user.id);
    if (!dev) return c.json({ error: 'not_found' }, 404);
    if (!owned) return c.json({ error: 'forbidden' }, 403);
    await db.update(tables.devices).set({ name: c.req.valid('json').name }).where(eq(tables.devices.id, dev.id));
    return c.json({ ok: true });
  })

  // Eliminar (solo dueño). Borra en cascada los grants.
  .delete('/:id', async (c) => {
    const user = c.get('user');
    const { dev, owned } = await loadOwned(c.req.param('id'), user.id);
    if (!dev) return c.json({ error: 'not_found' }, 404);
    if (!owned) return c.json({ error: 'forbidden' }, 403);
    await db.delete(tables.devices).where(eq(tables.devices.id, dev.id));
    return c.json({ ok: true });
  })

  // Listar accesos concedidos (solo dueño), resueltos a email/nombre de grupo.
  .get('/:id/access', async (c) => {
    const user = c.get('user');
    const { dev, owned } = await loadOwned(c.req.param('id'), user.id);
    if (!dev) return c.json({ error: 'not_found' }, 404);
    if (!owned) return c.json({ error: 'forbidden' }, 403);
    const rows = await db.select().from(tables.deviceAccess).where(eq(tables.deviceAccess.deviceId, dev.id));
    const grants = [];
    for (const r of rows) {
      if (r.principalId.startsWith('us_')) {
        const u = (await db.select({ email: tables.users.email, name: tables.users.name })
          .from(tables.users).where(eq(tables.users.id, r.principalId)))[0];
        grants.push({ principalId: r.principalId, kind: 'user', label: u?.email ?? r.principalId, name: u?.name });
      } else {
        const g = (await db.select({ name: tables.groups.name })
          .from(tables.groups).where(eq(tables.groups.id, r.principalId)))[0];
        grants.push({ principalId: r.principalId, kind: 'group', label: g?.name ?? r.principalId });
      }
    }
    return c.json({ owner: dev.ownerId, grants });
  })

  // Conceder acceso a un usuario (us_) o grupo (gp_) — solo dueño.
  .post('/:id/access', zValidator('json', accessSchema), async (c) => {
    const user = c.get('user');
    const { dev, owned } = await loadOwned(c.req.param('id'), user.id);
    if (!dev) return c.json({ error: 'not_found' }, 404);
    if (!owned) return c.json({ error: 'forbidden' }, 403);
    const { principalId } = c.req.valid('json');
    if (principalId.startsWith('us_')) {
      const u = await db.select().from(tables.users).where(eq(tables.users.id, principalId));
      if (u.length === 0) return c.json({ error: 'principal_not_found' }, 404);
    } else {
      const g = await db.select().from(tables.groups).where(eq(tables.groups.id, principalId));
      if (g.length === 0) return c.json({ error: 'principal_not_found' }, 404);
    }
    await db.insert(tables.deviceAccess)
      .values({ deviceId: dev.id, principalId, createdAt: new Date() })
      .onDuplicateKeyUpdate({ set: { principalId } });
    return c.json({ ok: true });
  })

  // Revocar acceso — solo dueño.
  .delete('/:id/access/:principalId', async (c) => {
    const user = c.get('user');
    const { dev, owned } = await loadOwned(c.req.param('id'), user.id);
    if (!dev) return c.json({ error: 'not_found' }, 404);
    if (!owned) return c.json({ error: 'forbidden' }, 403);
    await db.delete(tables.deviceAccess).where(and(
      eq(tables.deviceAccess.deviceId, dev.id),
      eq(tables.deviceAccess.principalId, c.req.param('principalId')),
    ));
    return c.json({ ok: true });
  })

  // Conectarse a un device por id (desde la libreta). Gate de acceso + reserva de
  // sala de señalización; ordena al equipo compartir y devuelve el código. Mismo
  // contrato de confirmación que /api/device/connect: si el equipo tiene activado
  // "pedir permiso", el flag viaja en el `start` y en la respuesta.
  .post('/:id/connect', async (c) => {
    const user = c.get('user');
    const deviceId = c.req.param('id');
    const role = await userCanAccessDevice(user.id, deviceId);
    if (!role) return c.json({ error: 'forbidden' }, 403);
    if (!deviceHub.isOnline(deviceId)) return c.json({ error: 'offline' }, 409);
    const dev = (await db.select().from(tables.devices).where(eq(tables.devices.id, deviceId)))[0];
    const requireConfirm = !!dev.requireConfirm;
    const code = reserveRemoteSession({ name: dev.name, deviceId: dev.id });
    if (!deviceHub.sendToDevice(deviceId, { type: 'start', code, confirm: requireConfirm })) {
      return c.json({ error: 'offline' }, 409);
    }
    return c.json({ code, name: dev.name, confirm: requireConfirm });
  });
