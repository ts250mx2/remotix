import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type { Hono } from 'hono';
import { serveStatic } from '@hono/node-server/serve-static';

/**
 * Sirve la consola web ya compilada (web/dist) desde el propio servidor, de
 * modo que todo (API + WS + SPA) viva en un único puerto. Esto es lo que hace
 * que la instalación sea "un solo proceso, una sola URL".
 *
 * En desarrollo se suele usar Vite (puerto 5173) con proxy hacia este server;
 * si web/dist no existe, simplemente no monta nada y avisa.
 */
/** Sirve binarios descargables (p. ej. el instalador del agente) desde public/.
 * Coloca el .exe firmado en server/public/ y se ofrece en /download/<archivo>. */
export function attachDownloads(app: Hono, publicRoot = './public'): void {
  const root = resolve(process.cwd(), publicRoot);
  if (!existsSync(root)) {
    console.log(`[download] ${root} no existe — sin binarios para descargar.`);
    return;
  }
  app.use('/download/*', serveStatic({ root: publicRoot, rewriteRequestPath: (p) => p.replace(/^\/download/, '') }));
  console.log(`[download] binarios servidos desde ${root} en /download/*`);
}

export function attachWebStatic(app: Hono, webRoot = '../web/dist'): void {
  const root = resolve(process.cwd(), webRoot);
  const indexPath = resolve(root, 'index.html');

  if (!existsSync(indexPath)) {
    console.log(`[web] ${root} no compilado — sirviendo solo API/WS.`);
    console.log('[web] Ejecuta "npm run build" en web/ (o usa Vite en dev).');
    return;
  }

  const indexHtml = readFileSync(indexPath, 'utf8');
  const isAppRequest = (path: string): boolean =>
    !path.startsWith('/api') && !path.startsWith('/ws') && !path.startsWith('/download') && path !== '/health';

  // 1) Archivos reales (assets con hash, favicon, etc.).
  app.use('*', async (c, next) => {
    if (!isAppRequest(c.req.path)) return next();
    return serveStatic({ root: webRoot })(c, next);
  });

  // 2) Fallback SPA: cualquier ruta de navegación devuelve index.html.
  app.get('*', (c, next) => {
    if (!isAppRequest(c.req.path)) return next();
    return c.html(indexHtml);
  });

  console.log(`[web] consola servida desde ${root}`);
}
