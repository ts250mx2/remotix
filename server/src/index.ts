import { serve } from '@hono/node-server';
import type { Server } from 'node:http';
import { app } from './app.js';
import { env } from './env.js';
import { pool } from './db/index.js';
import { applyMigrations } from './db/migrate.js';
import { attachSignaling } from './ws/signaling.js';
import { attachChat } from './chat/hub.js';
import { attachDeviceHub } from './devices/hub.js';
import { purgeExpired } from './auth/session.js';

async function main() {
  // Esquema (idempotente) antes de aceptar tráfico.
  await applyMigrations(pool);
  console.log(`[db]   MySQL conectado a ${env.mysql.host}:${env.mysql.port}/${env.mysql.database}`);

  const server = serve({ fetch: app.fetch, port: env.port }, (info) => {
    console.log(`[http] remotix-server listening on http://localhost:${info.port}`);
    console.log(`[http] portal API on http://localhost:${info.port}/api`);
    console.log(`[ws]   signaling on ws://localhost:${info.port}/ws/signal`);
  });

  // serve() devuelve un union (http/http2); en runtime es siempre http.Server.
  attachSignaling(server as unknown as Server);
  attachChat(server as unknown as Server);
  attachDeviceHub(server as unknown as Server);

  // Limpieza periódica de sesiones expiradas.
  setInterval(() => { void purgeExpired(); }, 1000 * 60 * 60).unref();
}

main().catch((err) => {
  console.error('[fatal] no se pudo arrancar el servidor:', err);
  process.exit(1);
});
