// Verifica: dedup de devices por machineId + conectar por clave SIN login.
import { WebSocket } from 'ws';
const BASE = process.env.SMOKE_BASE || 'http://localhost:8099';
let fail = 0;
const check = (c, m) => { console.log(c ? '  OK  ' : ' FAIL ', m); if (!c) fail++; };
const reg = (body) => fetch(BASE + '/api/device/register', {
  method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body),
}).then((r) => r.json());
function online(deviceId, secret) {
  return new Promise((res, rej) => {
    const ws = new WebSocket(BASE.replace(/^http/, 'ws') + '/ws/device');
    ws.on('open', () => ws.send(JSON.stringify({ type: 'hello', deviceId, secret })));
    ws.on('message', (d) => { const m = JSON.parse(d.toString()); m.type === 'ready' ? res(ws) : rej(new Error('auth')); });
    ws.on('error', rej);
  });
}

async function main() {
  const m1 = 'MID-' + Date.now();
  const a1 = await reg({ name: 'PC-A', machineId: m1 });
  const a2 = await reg({ name: 'PC-A', machineId: m1 });
  check(a1.accessKey === a2.accessKey && a1.deviceId === a2.deviceId, 'mismo machineId → MISMO device (no duplica), misma clave');
  check(a1.secret !== a2.secret, 'el secreto se renueva al re-registrar');

  const b = await reg({ name: 'PC-B', machineId: 'OTHER-' + Date.now() });
  check(b.accessKey !== a1.accessKey, 'otro machineId → otro device');

  const c1 = await reg({ name: 'PC-C' });
  const c2 = await reg({ name: 'PC-C' });
  check(c1.accessKey !== c2.accessKey, 'sin machineId → device nuevo cada vez (legacy)');

  // Conectar por clave SIN login (sin cookie).
  const ws = await online(a2.deviceId, a2.secret);
  const conn = await fetch(BASE + '/api/device/connect', {
    method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ accessKey: a1.accessKey }),
  });
  check(conn.status === 200 && !!(await conn.json()).code, 'conectar por clave SIN login → 200 + code');
  ws.close();

  console.log(fail === 0 ? '\nDEDUP + CONNECT-SIN-LOGIN OK' : `\n${fail} FALLARON`);
  process.exit(fail ? 1 : 0);
}
main().catch((e) => { console.error('ERROR:', e.message); process.exit(1); });
