import { Hono } from 'hono';
import { cors } from 'hono/cors';
import { logger } from 'hono/logger';
import { authRoutes } from './routes/auth.js';
import { projectRoutes } from './routes/projects.js';
import { equipoRoutes } from './routes/equipos.js';
import { pairingAuthedRoutes, pairingPublicRoutes } from './routes/pairing.js';
import { userRoutes } from './routes/users.js';
import { groupRoutes } from './routes/groups.js';
import { turnRoutes } from './routes/turn.js';
import { chatRoutes } from './routes/chat.js';
import { deviceRoutes } from './routes/device.js';
import { deviceManageRoutes } from './routes/devices.js';
import { attachWebStatic, attachDownloads } from './static.js';
import { env } from './env.js';

export const app = new Hono();

app.use('*', logger());
app.use('/api/*', cors({
  origin: (origin) => {
    if (!origin) return origin;
    if (env.isDev && /^http:\/\/localhost:\d+$/.test(origin)) return origin;
    return origin;
  },
  credentials: true,
}));

app.get('/health', (c) => c.json({ ok: true }));

app
  .route('/api/auth', authRoutes)
  .route('/api/projects', projectRoutes)
  .route('/api/equipos', equipoRoutes)
  .route('/api/pairing', pairingAuthedRoutes)
  .route('/api/agent', pairingPublicRoutes)   // /api/agent/enroll
  .route('/api/users', userRoutes)
  .route('/api/groups', groupRoutes)
  .route('/api/turn-credentials', turnRoutes)
  .route('/api/chat', chatRoutes)
  .route('/api/device', deviceRoutes)
  .route('/api/devices', deviceManageRoutes);

// Descarga del agente (antes de la SPA, para que /download no caiga en el fallback).
attachDownloads(app);

// Sirve la SPA compilada (web/dist) en el mismo puerto. Debe ir DESPUÉS de las
// rutas /api para que la API siempre gane.
attachWebStatic(app);

app.notFound((c) => c.json({ error: 'not_found' }, 404));
app.onError((err, c) => {
  console.error('[error]', err);
  return c.json({ error: 'internal', message: env.isDev ? err.message : undefined }, 500);
});
