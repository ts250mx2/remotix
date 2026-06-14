import type { MiddlewareHandler } from 'hono';
import { getCookie } from 'hono/cookie';
import { eq } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { getSession, COOKIE_NAME } from './session.js';

export interface AuthUser {
  id: string;
  email: string;
  name: string;
}

declare module 'hono' {
  interface ContextVariableMap {
    user: AuthUser;
  }
}

export const requireUser: MiddlewareHandler = async (c, next) => {
  const token = getCookie(c, COOKIE_NAME);
  if (!token) return c.json({ error: 'unauthenticated' }, 401);
  const session = await getSession(token);
  if (!session) return c.json({ error: 'unauthenticated' }, 401);
  const rows = await db.select().from(tables.users).where(eq(tables.users.id, session.userId));
  const user = rows[0];
  if (!user) return c.json({ error: 'unauthenticated' }, 401);
  c.set('user', { id: user.id, email: user.email, name: user.name });
  await next();
};
