import { chromium, request } from 'playwright';
import { mkdirSync } from 'node:fs';

const BASE = 'http://localhost:8080';
const OUT = process.argv[2] || './shots';
mkdirSync(OUT, { recursive: true });

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1280, height: 860 } });
const page = await ctx.newPage();
const t = Date.now();

// Ana = admin/owner (cookie en el contexto del navegador).
await page.request.post(`${BASE}/api/auth/register`, { data: { email: `demo_${t}@msp.com`, password: 'password123', name: 'Ana Torres' } });
const proj = (await (await page.request.post(`${BASE}/api/projects`, { data: { name: 'Farmacia Central' } })).json()).project;
const empresaId = proj.id;
const general = (await (await page.request.get(`${BASE}/api/chat/channels?empresa_id=${empresaId}`)).json()).channels[0];
await page.request.post(`${BASE}/api/agent/join`, { data: { projectId: empresaId, name: 'PC-Caja-1', os: 'windows' } });
await page.request.post(`${BASE}/api/agent/join`, { data: { projectId: empresaId, name: 'PC-Recepción', os: 'windows' } });
await page.request.post(`${BASE}/api/chat/channels/${general.id}/messages`, { data: { body: 'Buenos días, el equipo de la caja 1 va muy lento desde ayer. 😟' } });
await page.request.post(`${BASE}/api/chat/channels/${general.id}/messages`, { data: { body: 'Lo reviso en un momento, ¿puedes darme acceso remoto? 👍' } });

// Técnico (Carlos) en un contexto aislado para no pisar la cookie de Ana.
const tecCtx = await request.newContext();
const tec = (await (await tecCtx.post(`${BASE}/api/auth/register`, { data: { email: `tec_${t}@msp.com`, password: 'password123', name: 'Carlos Ruiz' } })).json()).user;
await tecCtx.dispose();
await page.request.post(`${BASE}/api/projects/${empresaId}/members`, { data: { principalId: tec.id, role: 'tecnico' } });
const grp = (await (await page.request.post(`${BASE}/api/groups`, { data: { name: 'Soporte Norte' } })).json()).group;
await page.request.post(`${BASE}/api/groups/${grp.id}/members`, { data: { userId: tec.id } });
await page.request.post(`${BASE}/api/projects/${empresaId}/members`, { data: { principalId: grp.id, role: 'tecnico' } });

async function shot(file) { await page.screenshot({ path: `${OUT}/${file}` }); console.log('shot', file); }

await page.goto(`${BASE}/projects/${empresaId}`, { waitUntil: 'domcontentloaded' });
await page.waitForTimeout(1200);
await shot('project.png');

await page.goto(`${BASE}/groups`, { waitUntil: 'domcontentloaded' });
await page.waitForTimeout(600);
await page.click('.link-row').catch(() => {});
await page.waitForTimeout(500);
await shot('groups.png');

await page.goto(`${BASE}/chat`, { waitUntil: 'domcontentloaded' });
await page.waitForTimeout(1600);
await page.click('button[title="Emojis"]').catch(() => {});
await page.waitForTimeout(400);
await shot('chat-emoji.png');

await browser.close();
console.log('OK');
