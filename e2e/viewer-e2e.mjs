// E2E del visor nativo (loopback en una máquina):
//   1) remotix-lite (host) registra un device y comparte esta pantalla.
//   2) Un usuario lo reclama y reserva sesión (API).
//   3) remotix console <code> (visor headless) decodifica el stream H.264 real
//      y reporta frames. Aserta que llegan frames.
//
// Requiere el server corriendo en :8099 contra MySQL.
import { spawn } from 'node:child_process';
import { rmSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const BASE = process.env.E2E_BASE || 'http://localhost:8099';
const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..');
const LITE = path.join(ROOT, 'agent', 'target', 'debug', 'remotix-lite.exe');
const REMOTIX = path.join(ROOT, 'agent', 'target', 'debug', 'remotix.exe');
const liteJson = path.join(process.env.APPDATA || '.', 'Remotix', 'lite.json');

const env = { ...process.env, REMOTIX_SERVER: BASE, RUST_LOG: 'warn,remotix_agent=info' };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function lineWaiter(proc, tag) {
  const waiters = [];
  let buf = '';
  const feed = (d) => {
    buf += d.toString();
    let i;
    while ((i = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, i).trim();
      buf = buf.slice(i + 1);
      if (line) { console.log(`[${tag}] ${line}`); for (const w of waiters.slice()) if (w.re.test(line)) { waiters.splice(waiters.indexOf(w), 1); w.resolve(line); } }
    }
  };
  proc.stdout.on('data', feed);
  proc.stderr.on('data', feed);
  return (re, timeoutMs) => new Promise((resolve, reject) => {
    const w = { re, resolve };
    waiters.push(w);
    setTimeout(() => { const k = waiters.indexOf(w); if (k >= 0) { waiters.splice(k, 1); reject(new Error(`timeout esperando ${re}`)); } }, timeoutMs);
  });
}

async function main() {
  try { rmSync(liteJson, { force: true }); } catch {}

  // 1) Host (lite) — registra y comparte esta pantalla.
  console.log('Arrancando host (remotix-lite console)…');
  const lite = spawn(LITE, ['console'], { env });
  const liteLine = lineWaiter(lite, 'host');
  const codeLine = await liteLine(/^CODE\s+(\S+)/, 30000);
  const accessKey = codeLine.split(/\s+/)[1];
  console.log('Clave del host:', accessKey);
  await liteLine(/En línea/, 20000); // presencia en /ws/device

  // 2) Usuario reclama el device y reserva sesión (API).
  const t = Date.now();
  const reg = await fetch(`${BASE}/api/auth/register`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ email: `viewer_${t}@e2e.com`, password: 'password123', name: 'Visor E2E' }) });
  const cookie = (reg.headers.getSetCookie?.() || [reg.headers.get('set-cookie')]).find((c) => c && c.startsWith('remotix_session=')).split(';')[0];
  const authed = (p, o = {}) => fetch(BASE + p, { ...o, headers: { ...(o.headers || {}), cookie, ...(o.body ? { 'content-type': 'application/json' } : {}) } });

  const claim = await (await authed('/api/devices/claim', { method: 'POST', body: JSON.stringify({ accessKey }) })).json();
  const deviceId = claim.device.id;
  console.log('Device reclamado:', deviceId);

  const conn = await authed(`/api/devices/${deviceId}/connect`, { method: 'POST' });
  if (!conn.ok) throw new Error(`connect falló: ${conn.status}`);
  const { code } = await conn.json();
  console.log('Sesión reservada, code:', code);

  // 3) Visor headless — decodifica el stream real.
  console.log('Arrancando visor (remotix console)…');
  const viewer = spawn(REMOTIX, ['console', code], { env });
  const viewerLine = lineWaiter(viewer, 'visor');
  let frames = 0;
  viewer.stdout.on('data', (d) => { const m = d.toString().match(/FRAME \d+x\d+/g); if (m) frames += m.length; });

  try { await viewerLine(/FRAME \d+x\d+/, 25000); } catch (e) { console.error('NO llegaron frames:', e.message); }
  await sleep(4000); // recoge unos cuantos más

  lite.kill(); viewer.kill();
  await sleep(500);

  if (frames > 0) { console.log(`\n✅ VISOR E2E OK — ${frames} muestras de frame decodificadas`); process.exit(0); }
  else { console.log('\n❌ VISOR E2E FALLÓ — no se decodificó ningún frame'); process.exit(1); }
}

main().catch((e) => { console.error('ERROR:', e.message); process.exit(1); });
