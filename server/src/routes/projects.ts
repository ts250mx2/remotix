import { Hono } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { z } from 'zod';
import { and, eq, inArray } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { requireUser } from '../auth/middleware.js';
import { newId } from '../ids.js';

const createSchema = z.object({
  name: z.string().min(1).max(120),
});

const addMemberSchema = z.object({
  principalId: z.string().regex(/^(us|gp)_[0-9a-zA-Z]{22}$/),
  role: z.enum(['admin', 'tecnico', 'usuario', 'operator']).optional(),
});

// Devuelve los IDs de proyectos a los que el usuario tiene acceso (owner,
// miembro directo, o miembro vía un grupo del que forma parte).
async function visibleProjectIds(userId: string): Promise<string[]> {
  const owned = await db.select({ id: tables.projects.id })
    .from(tables.projects)
    .where(eq(tables.projects.ownerId, userId));

  const groupRows = await db.select({ groupId: tables.groupMembers.groupId })
    .from(tables.groupMembers)
    .where(eq(tables.groupMembers.userId, userId));
  const groupIds = groupRows.map((r) => r.groupId);

  const principals = [userId, ...groupIds];
  const memberships = await db.select({ projectId: tables.projectMembers.projectId })
    .from(tables.projectMembers)
    .where(inArray(tables.projectMembers.principalId, principals));

  return Array.from(new Set([...owned.map((r) => r.id), ...memberships.map((r) => r.projectId)]));
}

export async function userHasProjectAccess(userId: string, projectId: string): Promise<'admin' | 'operator' | null> {
  const proj = (await db.select().from(tables.projects).where(eq(tables.projects.id, projectId)))[0];
  if (!proj) return null;
  if (proj.ownerId === userId) return 'admin';

  const groupRows = await db.select({ groupId: tables.groupMembers.groupId })
    .from(tables.groupMembers)
    .where(eq(tables.groupMembers.userId, userId));
  const principals = [userId, ...groupRows.map((r) => r.groupId)];

  const memberships = await db.select()
    .from(tables.projectMembers)
    .where(and(
      eq(tables.projectMembers.projectId, projectId),
      inArray(tables.projectMembers.principalId, principals),
    ));

  if (memberships.length === 0) return null;
  // Si alguna membresía es 'admin', gana admin.
  return memberships.some((m) => m.role === 'admin') ? 'admin' : 'operator';
}

export const projectRoutes = new Hono()
  .use('*', requireUser)

  .get('/', async (c) => {
    const user = c.get('user');
    const ids = await visibleProjectIds(user.id);
    if (ids.length === 0) return c.json({ projects: [] });
    const rows = await db.select().from(tables.projects).where(inArray(tables.projects.id, ids));
    return c.json({
      projects: rows.map((p) => ({
        id: p.id,
        name: p.name,
        ownerId: p.ownerId,
        createdAt: p.createdAt,
        isOwner: p.ownerId === user.id,
      })),
    });
  })

  .post('/', zValidator('json', createSchema), async (c) => {
    const user = c.get('user');
    const { name } = c.req.valid('json');
    const id = newId('py');
    await db.insert(tables.projects).values({
      id, name, ownerId: user.id, createdAt: new Date(),
    });
    return c.json({ project: { id, name, ownerId: user.id } }, 201);
  })

  .get('/:id', async (c) => {
    const user = c.get('user');
    const projectId = c.req.param('id');
    const role = await userHasProjectAccess(user.id, projectId);
    if (!role) return c.json({ error: 'not_found' }, 404);
    const proj = (await db.select().from(tables.projects).where(eq(tables.projects.id, projectId)))[0];
    return c.json({
      project: { id: proj!.id, name: proj!.name, ownerId: proj!.ownerId, createdAt: proj!.createdAt },
      role,
    });
  })

  .get('/:id/members', async (c) => {
    const user = c.get('user');
    const projectId = c.req.param('id');
    const role = await userHasProjectAccess(user.id, projectId);
    if (!role) return c.json({ error: 'not_found' }, 404);
    const members = await db.select().from(tables.projectMembers)
      .where(eq(tables.projectMembers.projectId, projectId));
    return c.json({ members });
  })

  .post('/:id/members', zValidator('json', addMemberSchema), async (c) => {
    const user = c.get('user');
    const projectId = c.req.param('id');
    const role = await userHasProjectAccess(user.id, projectId);
    if (role !== 'admin') return c.json({ error: 'forbidden' }, 403);
    const { principalId, role: memberRole = 'operator' } = c.req.valid('json');

    if (principalId.startsWith('us_')) {
      const u = await db.select().from(tables.users).where(eq(tables.users.id, principalId));
      if (u.length === 0) return c.json({ error: 'principal_not_found' }, 404);
    } else {
      const g = await db.select().from(tables.groups).where(eq(tables.groups.id, principalId));
      if (g.length === 0) return c.json({ error: 'principal_not_found' }, 404);
    }

    await db.insert(tables.projectMembers)
      .values({ projectId, principalId, role: memberRole })
      .onDuplicateKeyUpdate({ set: { role: memberRole } });
    return c.json({ ok: true });
  })

  .delete('/:id/members/:principalId', async (c) => {
    const user = c.get('user');
    const projectId = c.req.param('id');
    const principalId = c.req.param('principalId');
    const role = await userHasProjectAccess(user.id, projectId);
    if (role !== 'admin') return c.json({ error: 'forbidden' }, 403);
    await db.delete(tables.projectMembers).where(and(
      eq(tables.projectMembers.projectId, projectId),
      eq(tables.projectMembers.principalId, principalId),
    ));
    return c.json({ ok: true });
  });
