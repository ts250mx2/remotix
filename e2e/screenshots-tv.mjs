// Capturas del admin web (modelo TeamViewer): Mis PCs, detalle/accesos, grupos,
// usuarios, operador y login. Siembra datos vía API y navega con Playwright.
import { chromium, request } from 'playwright';
import { WebSocket } from 'ws';
import { mkdirSync } from 'node:fs';

const BASE = process.env.SHOT_BASE || 'http://localhost:8099';
const OUT = process.argv[2] || './shots-tv';
mkdirSync(OUT, { recursive: true });

function deviceOnline(deviceId, secret) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(BASE.replace(/^http/, 'ws') + '/ws/device');
    ws.on('open', () => ws.send(JSON.stringify({ type: 'hello', deviceId, secret })));
    ws.on('message', (d) => { const m = JSON.parse(d.toString()); m.type === 'ready' ? resolve(ws) : reject(new Error('auth')); });
    ws.on('error', reject);
  });
}

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1280, height: 860 } });
const page = await ctx.newPage();
const t = Date.now();

// Ana = dueña (su cookie queda en el contexto del navegador).
await page.request.post(`${BASE}/api/auth/register`, { data: { email: `ana_${t}@demo.com`, password: 'password123', name: 'Ana Torres' } });

// Dos PCs registrados (como haría el exe), reclamados por Ana.
const dev1 = await (await page.request.post(`${BASE}/api/device/register`, { data: { name: 'PC-Oficina', os: 'windows', hostname: 'OFI-01' } })).json();
const dev2 = await (await page.request.post(`${BASE}/api/device/register`, { data: { name: 'Laptop-Gerencia', os: 'windows', hostname: 'GER-LT' } })).json();
await page.request.post(`${BASE}/api/devices/claim`, { data: { accessKey: dev1.accessKey } });
await page.request.post(`${BASE}/api/devices/claim`, { data: { accessKey: dev2.accessKey } });

// Carlos (otro usuario) + grupo, ambos con acceso a PC-Oficina.
const tecCtx = await request.newContext();
const carlos = (await (await tecCtx.post(`${BASE}/api/auth/register`, { data: { email: `carlos_${t}@demo.com`, password: 'password123', name: 'Carlos Ruiz' } })).json()).user;
await tecCtx.dispose();
const grp = (await (await page.request.post(`${BASE}/api/groups`, { data: { name: 'Soporte Norte' } })).json()).group;
await page.request.post(`${BASE}/api/groups/${grp.id}/members`, { data: { userId: carlos.id } });
await page.request.post(`${BASE}/api/devices/${dev1.deviceId}/access`, { data: { principalId: carlos.id } });
await page.request.post(`${BASE}/api/devices/${dev1.deviceId}/access`, { data: { principalId: grp.id } });

// PC-Oficina en línea (para ver estado verde + botón Conectar habilitado).
const ws = await deviceOnline(dev1.deviceId, dev1.secret);

async function shot(file, ms = 700) { await page.waitForTimeout(ms); await page.screenshot({ path: `${OUT}/${file}` }); console.log('shot', file); }

await page.goto(`${BASE}/`, { waitUntil: 'domcontentloaded' });
await shot('01-mis-pcs.png', 1000);

await page.goto(`${BASE}/devices/${dev1.deviceId}`, { waitUntil: 'domcontentloaded' });
await shot('02-detalle-accesos.png', 1000);

await page.goto(`${BASE}/groups`, { waitUntil: 'domcontentloaded' });
await page.click('.link-row, .group-list li').catch(() => {});
await shot('03-grupos.png');

await page.goto(`${BASE}/users`, { waitUntil: 'domcontentloaded' });
await shot('04-usuarios.png');

await page.goto(`${BASE}/operador`, { waitUntil: 'domcontentloaded' });
await shot('05-operador.png');

// Login (contexto nuevo, sin sesión).
const anon = await browser.newContext({ viewport: { width: 1280, height: 860 } });
const ap = await anon.newPage();
await ap.goto(`${BASE}/login`, { waitUntil: 'domcontentloaded' });
await ap.waitForTimeout(600);
await ap.screenshot({ path: `${OUT}/06-login.png` });
console.log('shot 06-login.png');

ws.close();
await browser.close();
console.log('OK');
