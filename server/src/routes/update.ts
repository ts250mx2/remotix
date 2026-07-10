import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { Hono } from 'hono';

// Manifiesto de la última versión publicada. `infra\build-app-installer.ps1`
// genera `server/public/remotix-latest.json` (canal por defecto: la APP, que se
// auto-actualiza sola). El canal `host` (instalación con servicio de Windows,
// RemotixHostSetup) usa `remotix-host-latest.json` para NUNCA recibir el
// instalador de la app por error; si ese manifiesto no existe, el host
// simplemente no se auto-actualiza.
interface Manifest {
  version: string;
  url: string;
  notes?: string;
  mandatory?: boolean;
}

const FILES: Record<string, { file: string; url: string }> = {
  app: { file: './public/remotix-latest.json', url: '/download/RemotixSetup.exe' },
  host: { file: './public/remotix-host-latest.json', url: '/download/RemotixHostSetup.exe' },
};

export function readManifest(channel: 'app' | 'host' = 'app'): Manifest {
  const { file, url } = FILES[channel];
  const path = resolve(process.cwd(), file);
  if (existsSync(path)) {
    try {
      // strip BOM (U+FEFF): PowerShell/algunos editores lo añaden y rompería JSON.parse.
      let raw = readFileSync(path, 'utf8');
      if (raw.charCodeAt(0) === 0xfeff) raw = raw.slice(1);
      const m = JSON.parse(raw) as Partial<Manifest>;
      if (m && typeof m.version === 'string') {
        return {
          version: m.version,
          url: typeof m.url === 'string' ? m.url : url,
          notes: typeof m.notes === 'string' ? m.notes : undefined,
          mandatory: !!m.mandatory,
        };
      }
    } catch {
      /* manifiesto corrupto: se trata como "sin versión" */
    }
  }
  // Sin manifiesto: 0.0.0 => ningún agente lo considerará más nuevo.
  return { version: '0.0.0', url };
}

/// Compara versiones "a.b.c" (mismo criterio que agent/src/update.rs).
export function versionIsNewer(latest: string, current: string): boolean {
  const parse = (s: string) =>
    s.split(/[.\-+]/).map((p) => {
      const digits = /^\d+/.exec(p)?.[0] ?? '0';
      return Number.parseInt(digits, 10);
    });
  const a = parse(latest);
  const b = parse(current);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] ?? 0;
    const y = b[i] ?? 0;
    if (x !== y) return x > y;
  }
  return false;
}

export const updateRoutes = new Hono().get('/latest', (c) =>
  c.json(readManifest(c.req.query('channel') === 'host' ? 'host' : 'app')),
);
