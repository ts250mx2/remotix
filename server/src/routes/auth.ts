import { Hono, type Context } from 'hono';
import { setCookie, deleteCookie } from 'hono/cookie';
import { zValidator } from '@hono/zod-validator';
import { z } from 'zod';
import { eq } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { hashPassword, verifyPassword } from '../auth/password.js';
import { createSession, deleteSession, COOKIE_NAME } from '../auth/session.js';
import { requireUser } from '../auth/middleware.js';
import { newId } from '../ids.js';
import { env } from '../env.js';

const registerSchema = z.object({
  email: z.string().email().max(254),
  password: z.string().min(8).max(128),
  name: z.string().min(1).max(100),
});

const loginSchema = z.object({
  email: z.string().email().max(254),
  password: z.string().min(1).max(128),
});

function sessionCookie(c: Context, token: string, expires: Date) {
  setCookie(c, COOKIE_NAME, token, {
    httpOnly: true,
    sameSite: 'Lax',
    secure: !env.isDev,
    path: '/',
    expires,
  });
}

export const authRoutes = new Hono()
  .post('/register', zValidator('json', registerSchema), async (c) => {
    const { email, password, name } = c.req.valid('json');
    const existing = await db.select().from(tables.users).where(eq(tables.users.email, email));
    if (existing.length > 0) return c.json({ error: 'email_taken' }, 409);

    const id = newId('us');
    const passwordHash = await hashPassword(password);
    await db.insert(tables.users).values({
      id,
      email,
      passwordHash,
      name,
      createdAt: new Date(),
    });

    const session = await createSession(id);
    sessionCookie(c, session.token, session.expiresAt);
    return c.json({ user: { id, email, name } }, 201);
  })

  .post('/login', zValidator('json', loginSchema), async (c) => {
    const { email, password } = c.req.valid('json');
    const rows = await db.select().from(tables.users).where(eq(tables.users.email, email));
    const user = rows[0];
    if (!user) return c.json({ error: 'invalid_credentials' }, 401);
    const ok = await verifyPassword(password, user.passwordHash);
    if (!ok) return c.json({ error: 'invalid_credentials' }, 401);

    const session = await createSession(user.id);
    sessionCookie(c, session.token, session.expiresAt);
    return c.json({ user: { id: user.id, email: user.email, name: user.name } });
  })

  .post('/logout', async (c) => {
    const cookie = c.req.header('cookie') ?? '';
    const match = cookie.split(';').map((s) => s.trim()).find((s) => s.startsWith(`${COOKIE_NAME}=`));
    if (match) {
      const token = match.slice(COOKIE_NAME.length + 1);
      await deleteSession(token);
    }
    deleteCookie(c, COOKIE_NAME, { path: '/' });
    return c.json({ ok: true });
  })

  .get('/me', requireUser, (c) => {
    const user = c.get('user');
    return c.json({ user });
  });
