import { Hono } from 'hono';
import { eq } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { requireUser } from '../auth/middleware.js';

// Directorio mínimo de usuarios (para añadir a proyectos/grupos).
// Por ahora cualquier usuario autenticado puede ver el directorio.
// En el futuro: restringir a admin del proyecto / tenant.
export const userRoutes = new Hono()
  .use('*', requireUser)
  .get('/', async (c) => {
    const rows = await db.select({
      id: tables.users.id,
      email: tables.users.email,
      name: tables.users.name,
    }).from(tables.users);
    return c.json({ users: rows });
  })
  // Buscar un usuario por email (para asignarlo como técnico a un proyecto).
  .get('/lookup', async (c) => {
    const email = (c.req.query('email') ?? '').trim().toLowerCase();
    if (!email) return c.json({ error: 'email_required' }, 400);
    const u = (await db.select({ id: tables.users.id, email: tables.users.email, name: tables.users.name })
      .from(tables.users).where(eq(tables.users.email, email)))[0];
    if (!u) return c.json({ error: 'not_found' }, 404);
    return c.json({ user: u });
  });
