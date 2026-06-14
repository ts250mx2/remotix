import { Hono } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { z } from 'zod';
import { and, eq } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { requireUser } from '../auth/middleware.js';
import { newId } from '../ids.js';

const createSchema = z.object({ name: z.string().min(1).max(120) });
const addMemberSchema = z.object({
  userId: z.string().regex(/^us_[0-9a-zA-Z]{22}$/),
});

export const groupRoutes = new Hono()
  .use('*', requireUser)

  .get('/', async (c) => {
    const rows = await db.select().from(tables.groups);
    return c.json({ groups: rows });
  })

  .post('/', zValidator('json', createSchema), async (c) => {
    const { name } = c.req.valid('json');
    const id = newId('gp');
    await db.insert(tables.groups).values({ id, name, createdAt: new Date() });
    return c.json({ group: { id, name } }, 201);
  })

  .get('/:id/members', async (c) => {
    const groupId = c.req.param('id');
    const rows = await db.select({
      userId: tables.groupMembers.userId,
      email: tables.users.email,
      name: tables.users.name,
    })
      .from(tables.groupMembers)
      .leftJoin(tables.users, eq(tables.groupMembers.userId, tables.users.id))
      .where(eq(tables.groupMembers.groupId, groupId));
    return c.json({ members: rows });
  })

  .post('/:id/members', zValidator('json', addMemberSchema), async (c) => {
    const groupId = c.req.param('id');
    const { userId } = c.req.valid('json');
    const u = await db.select().from(tables.users).where(eq(tables.users.id, userId));
    if (u.length === 0) return c.json({ error: 'user_not_found' }, 404);
    await db.insert(tables.groupMembers).values({ groupId, userId }).onConflictDoNothing();
    return c.json({ ok: true });
  })

  .delete('/:id/members/:userId', async (c) => {
    const groupId = c.req.param('id');
    const userId = c.req.param('userId');
    await db.delete(tables.groupMembers).where(and(
      eq(tables.groupMembers.groupId, groupId),
      eq(tables.groupMembers.userId, userId),
    ));
    return c.json({ ok: true });
  });
