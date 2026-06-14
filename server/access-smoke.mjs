// Smoke de accesos (Fase 1, TeamViewer): dueño/grant/grupo + gate de /connect.
// Requiere el server corriendo en :8080 contra MySQL.
import { WebSocket } from 'ws';

const BASE = process.env.SMOKE_BASE || 'http://localhost:8080';
const WS_DEVICE = BASE.replace(/^http/, 'ws') + '/ws/device';
let fail = 0;
const check = (c, m) => { console.log(c ? '  OK  ' : ' FAIL ', m); if (!c) fail++; };

async function register(email) {
  const res = await fetch(BASE + '/api/auth/register', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email, password: 'password123', name: email.split('@')[0] }),
  });
  const sc = res.headers.getSetCookie ? res.headers.getSetCookie() : [res.headers.get('set-cookie')];
  return { cookie: (sc.find((c) => c && c.startsWith('remotix_session=')) || '').split(';')[0], user: (await res.json()).user };
}
const authed = (cookie) => (p, o = {}) => fetch(BASE + p, {
  ...o, headers: { ...(o.headers || {}), cookie, ...(o.body ? { 'content-type': 'application/json' } : {}) },
});

function deviceOnline(deviceId, secret) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(WS_DEVICE);
    ws.on('open', () => ws.send(JSON.stringify({ type: 'hello', deviceId, secret })));
    ws.on('message', (d) => {
      const m = JSON.parse(d.toString());
      if (m.type === 'ready') resolve(ws); else reject(new Error('device auth failed'));
    });
    ws.on('error', reject);
  });
}

async function main() {
  const t = Date.now();
  const A = await register(`owner_${t}@tv.com`);   // dueño
  const B = await register(`other_${t}@tv.com`);   // otro usuario (sin acceso)
  const C = await register(`group_${t}@tv.com`);   // miembro de grupo
  const fa = authed(A.cookie), fb = authed(B.cookie), fc = authed(C.cookie);

  // El exe (sin login) registra un device.
  const reg = await (await fetch(BASE + '/api/device/register', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name: 'PC-Oficina', os: 'windows', hostname: 'OFI-01' }),
  })).json();
  check(!!reg.deviceId && !!reg.accessKey && !!reg.secret, 'device registrado (id+clave+secret)');

  // A lo reclama → dueño.
  const claim = await fa('/api/devices/claim', { method: 'POST', body: JSON.stringify({ accessKey: reg.accessKey }) });
  check(claim.status === 200, 'A reclama el device (200)');

  // A lo ve en su libreta; B no.
  let la = (await (await fa('/api/devices')).json()).devices;
  check(la.some((d) => d.id === reg.deviceId && d.role === 'owner'), 'A ve el device como owner');
  let lb = (await (await fb('/api/devices')).json()).devices;
  check(!lb.some((d) => d.id === reg.deviceId), 'B NO ve el device');

  // Reclamar por un segundo usuario → 409.
  const claim2 = await fb('/api/devices/claim', { method: 'POST', body: JSON.stringify({ accessKey: reg.accessKey }) });
  check(claim2.status === 409, 'B no puede reclamar un device ya con dueño (409)');

  // Poner el device online (WS /ws/device).
  const ws = await deviceOnline(reg.deviceId, reg.secret);

  // Gate de /api/device/connect (por clave): A puede, B no.
  const cA = await fa('/api/device/connect', { method: 'POST', body: JSON.stringify({ accessKey: reg.accessKey }) });
  check(cA.status === 200 && !!(await cA.json()).code, 'A conecta por clave → 200 + code');
  const cB = await fb('/api/device/connect', { method: 'POST', body: JSON.stringify({ accessKey: reg.accessKey }) });
  check(cB.status === 403, 'B sin acceso → 403 (clave)');

  // Gate de /api/devices/:id/connect (por id).
  const cBid = await fb(`/api/devices/${reg.deviceId}/connect`, { method: 'POST' });
  check(cBid.status === 403, 'B sin acceso → 403 (id)');

  // Sin login → 401.
  const cAnon = await fetch(BASE + '/api/device/connect', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ accessKey: reg.accessKey }),
  });
  check(cAnon.status === 401, 'sin login → 401');

  // A concede acceso directo a B.
  const grant = await fa(`/api/devices/${reg.deviceId}/access`, { method: 'POST', body: JSON.stringify({ principalId: B.user.id }) });
  check(grant.status === 200, 'A concede acceso a B (200)');
  lb = (await (await fb('/api/devices')).json()).devices;
  check(lb.some((d) => d.id === reg.deviceId && d.role === 'granted'), 'B ahora ve el device (granted)');
  const cB2 = await fb('/api/device/connect', { method: 'POST', body: JSON.stringify({ accessKey: reg.accessKey }) });
  check(cB2.status === 200, 'B con grant → 200');

  // Revocar a B.
  await fa(`/api/devices/${reg.deviceId}/access/${B.user.id}`, { method: 'DELETE' });
  const cB3 = await fb('/api/device/connect', { method: 'POST', body: JSON.stringify({ accessKey: reg.accessKey }) });
  check(cB3.status === 403, 'tras revocar, B → 403');

  // Acceso vía grupo: grupo G con C dentro, grant del grupo al device.
  const g = (await (await fa('/api/groups', { method: 'POST', body: JSON.stringify({ name: `Soporte_${t}` }) })).json()).group;
  await fa(`/api/groups/${g.id}/members`, { method: 'POST', body: JSON.stringify({ userId: C.user.id }) });
  await fa(`/api/devices/${reg.deviceId}/access`, { method: 'POST', body: JSON.stringify({ principalId: g.id }) });
  let lc = (await (await fc('/api/devices')).json()).devices;
  check(lc.some((d) => d.id === reg.deviceId), 'C (miembro del grupo) ve el device');
  const cC = await fc('/api/device/connect', { method: 'POST', body: JSON.stringify({ accessKey: reg.accessKey }) });
  check(cC.status === 200, 'C vía grupo → 200');

  // Renombrar (solo dueño) y borrar.
  check((await fa(`/api/devices/${reg.deviceId}`, { method: 'PATCH', body: JSON.stringify({ name: 'PC-Renombrada' }) })).status === 200, 'A renombra (200)');
  check((await fb(`/api/devices/${reg.deviceId}`, { method: 'PATCH', body: JSON.stringify({ name: 'x' }) })).status === 403, 'B no puede renombrar (403)');

  ws.close();
  console.log(fail === 0 ? '\nTODOS LOS CHECKS DE ACCESO OK' : `\n${fail} FALLARON`);
  process.exit(fail ? 1 : 0);
}
main().catch((e) => { console.error('ERROR:', e.message); process.exit(1); });
