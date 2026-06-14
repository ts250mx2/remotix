import { Hono } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { z } from 'zod';
import { and, eq, isNull } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { requireUser } from '../auth/middleware.js';
import { userHasProjectAccess } from './projects.js';
import { newId, newAgentSecret, newPairingCode, newPin } from '../ids.js';
import { hashSecret, verifySecret, verifyPassword } from '../auth/password.js';
import { ensureGeneralChannel, addToPublicChannels, bindUserToPc, unbindPc } from '../chat/service.js';

const ENROLL_TTL_MS = 1000 * 60 * 60; // 1 hora

const createTokenSchema = z.object({
  projectId: z.string().regex(/^py_[0-9a-zA-Z]{22}$/),
});

const enrollSchema = z.object({
  projectId: z.string().regex(/^py_[0-9a-zA-Z]{22}$/),
  pairingCode: z.string().min(8).max(16),
  name: z.string().min(1).max(120),
  hostname: z.string().max(255).optional(),
  os: z.string().max(60).optional(),
});

// Rutas autenticadas (admin del proyecto crea un pairing code).
export const pairingAuthedRoutes = new Hono()
  .use('*', requireUser)
  .post('/', zValidator('json', createTokenSchema), async (c) => {
    const user = c.get('user');
    const { projectId } = c.req.valid('json');
    const role = await userHasProjectAccess(user.id, projectId);
    if (role !== 'admin') return c.json({ error: 'forbidden' }, 403);

    const token = newPairingCode();
    const expiresAt = new Date(Date.now() + ENROLL_TTL_MS);
    await db.insert(tables.enrollmentTokens).values({
      token, projectId, createdBy: user.id, expiresAt,
    });
    return c.json({ pairingCode: token, projectId, expiresAt: expiresAt.toISOString() }, 201);
  });

const joinSchema = z.object({
  projectId: z.string().regex(/^py_[0-9a-zA-Z]{22}$/),
  name: z.string().min(1).max(120),
  hostname: z.string().max(255).optional(),
  os: z.string().max(60).optional(),
});

const bindSchema = z.object({
  equipoId: z.string().regex(/^eq_[0-9a-zA-Z]{22}$/),
  agentSecret: z.string().min(10),
  email: z.string().email().max(254),
  password: z.string().min(1).max(128),
});

const unbindSchema = z.object({
  equipoId: z.string().regex(/^eq_[0-9a-zA-Z]{22}$/),
  agentSecret: z.string().min(10),
});

async function verifyAgent(equipoId: string, agentSecret: string) {
  const eqp = (await db.select().from(tables.equipos).where(eq(tables.equipos.id, equipoId)))[0];
  if (!eqp) return null;
  return (await verifySecret(agentSecret, eqp.agentSecretHash)) ? eqp : null;
}

// Ruta pública usada por el AGENTE/cliente al instalarse: SOLO requiere el UUID
// del proyecto (que actúa como secreto de unión, ~131 bits de entropía).
export const pairingPublicRoutes = new Hono()
  // Login en el agente: "casa" un usuario con este PC (uno a la vez).
  .post('/bind', zValidator('json', bindSchema), async (c) => {
    const { equipoId, agentSecret, email, password } = c.req.valid('json');
    const eqp = await verifyAgent(equipoId, agentSecret);
    if (!eqp) return c.json({ error: 'agent_unauthorized' }, 401);
    const user = (await db.select().from(tables.users).where(eq(tables.users.email, email.toLowerCase())))[0]
      ?? (await db.select().from(tables.users).where(eq(tables.users.email, email)))[0];
    if (!user || !(await verifyPassword(password, user.passwordHash))) {
      return c.json({ error: 'invalid_credentials' }, 401);
    }
    await bindUserToPc(user.id, equipoId);
    return c.json({ userId: user.id, name: user.name });
  })

  .post('/unbind', zValidator('json', unbindSchema), async (c) => {
    const { equipoId, agentSecret } = c.req.valid('json');
    const eqp = await verifyAgent(equipoId, agentSecret);
    if (!eqp) return c.json({ error: 'agent_unauthorized' }, 401);
    await unbindPc(equipoId);
    return c.json({ ok: true });
  })

  .post('/join', zValidator('json', joinSchema), async (c) => {
    const { projectId, name, hostname, os } = c.req.valid('json');
    const project = (await db.select().from(tables.projects).where(eq(tables.projects.id, projectId)))[0];
    if (!project) return c.json({ error: 'invalid_project' }, 400);

    const equipoId = newId('eq');
    const agentSecret = newAgentSecret();
    const agentSecretHash = await hashSecret(agentSecret);

    await db.insert(tables.equipos).values({
      id: equipoId,
      projectId,
      name,
      os,
      hostname,
      agentSecretHash,
      pinHash: null,
      pinMode: 'none',
      createdAt: new Date(),
    });

    // El PC entra al chat de la empresa (canal general + públicos).
    await ensureGeneralChannel(projectId);
    await addToPublicChannels(projectId, equipoId);

    // El cliente guarda (equipoId, agentSecret) para autenticarse en el chat
    // y en la señalización. El nombre del PC es configurable.
    return c.json({ equipoId, projectId, agentSecret, name }, 201);
  })

  .post('/enroll', zValidator('json', enrollSchema), async (c) => {
    const { projectId, pairingCode, name, hostname, os } = c.req.valid('json');

    const tokenRow = (await db.select().from(tables.enrollmentTokens)
      .where(and(
        eq(tables.enrollmentTokens.token, pairingCode),
        eq(tables.enrollmentTokens.projectId, projectId),
        isNull(tables.enrollmentTokens.usedAt),
      )))[0];

    if (!tokenRow) return c.json({ error: 'invalid_or_used_pairing_code' }, 400);
    if (tokenRow.expiresAt.getTime() < Date.now()) {
      return c.json({ error: 'pairing_code_expired' }, 400);
    }

    const equipoId = newId('eq');
    const agentSecret = newAgentSecret();
    const pin = newPin();
    const [agentSecretHash, pinHash] = await Promise.all([
      hashSecret(agentSecret),
      hashSecret(pin),
    ]);

    await db.transaction(async (tx) => {
      await tx.insert(tables.equipos).values({
        id: equipoId,
        projectId,
        name,
        os,
        hostname,
        agentSecretHash,
        pinHash,
        pinMode: 'required',
        createdAt: new Date(),
      });
      await tx.update(tables.enrollmentTokens).set({
        usedAt: new Date(),
        usedByEquipo: equipoId,
      }).where(eq(tables.enrollmentTokens.token, pairingCode));
    });

    // El agente debe guardar (equipoId, agentSecret) localmente. El PIN se
    // muestra al usuario final (operator) — el agente puede mostrarlo en su
    // bandeja del sistema.
    return c.json({
      equipoId,
      projectId,
      agentSecret,
      pin,
    }, 201);
  });
