import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { Hono } from 'hono';

// Manifiesto de la última versión del agente. Lo genera `infra\build-installer.ps1`
// al publicar el instalador en `server/public/remotix-latest.json`. El servicio
// de cada equipo consulta este endpoint y, si su versión es más antigua, se
// auto-actualiza (silencioso, cuando no hay sesión activa).
interface Manifest {
  version: string;
  url: string;
  notes?: string;
  mandatory?: boolean;
}

function readManifest(): Manifest {
  const path = resolve(process.cwd(), './public/remotix-latest.json');
  if (existsSync(path)) {
    try {
      // strip BOM (U+FEFF): PowerShell/algunos editores lo añaden y rompería JSON.parse.
      let raw = readFileSync(path, 'utf8');
      if (raw.charCodeAt(0) === 0xfeff) raw = raw.slice(1);
      const m = JSON.parse(raw) as Partial<Manifest>;
      if (m && typeof m.version === 'string') {
        return {
          version: m.version,
          url: typeof m.url === 'string' ? m.url : '/download/RemotixSetup.exe',
          notes: typeof m.notes === 'string' ? m.notes : undefined,
          mandatory: !!m.mandatory,
        };
      }
    } catch {
      /* manifiesto corrupto: se trata como "sin versión" */
    }
  }
  // Sin manifiesto: 0.0.0 => ningún agente lo considerará más nuevo.
  return { version: '0.0.0', url: '/download/RemotixSetup.exe' };
}

export const updateRoutes = new Hono().get('/latest', (c) => c.json(readManifest()));
