import { WebSocket } from 'ws';

const BASE = 'http://localhost:8080';
const WS_CHAT = BASE.replace('http', 'ws') + '/ws/chat';
const WS_SIGNAL = BASE.replace('http', 'ws') + '/ws/signal';
let fail = 0;
const check = (c, m) => { console.log(c ? '  OK  ' : ' FAIL ', m); if (!c) fail++; };
const open = (ws) => new Promise((r) => ws.once('open', r));
function track(ws) { ws._buf = []; ws.on('message', (d) => ws._buf.push(JSON.parse(d.toString()))); }
const waitFor = (ws, pred, ms = 4000) => new Promise((res, rej) => {
  const hit = ws._buf.find(pred); if (hit) return res(hit);
  const to = setTimeout(() => rej(new Error('timeout; buf=' + JSON.stringify(ws._buf))), ms);
  const on = (d) => { const m = JSON.parse(d.toString()); if (pred(m)) { clearTimeout(to); ws.off('message', on); res(m); } };
  ws.on('message', on);
});
async function register(email) {
  const res = await fetch(BASE + '/api/auth/register', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ email, password: 'password123', name: email.split('@')[0] }) });
  const sc = res.headers.getSetCookie ? res.headers.getSetCookie() : [res.headers.get('set-cookie')];
  return { cookie: (sc.find((c) => c && c.startsWith('remotix_session=')) || '').split(';')[0], user: (await res.json()).user };
}
const authed = (cookie) => (p, o = {}) => fetch(BASE + p, { ...o, headers: { ...(o.headers || {}), cookie, ...(o.body ? { 'content-type': 'application/json' } : {}) } });

async function main() {
  const t = Date.now();
  const admin = await register(`adm_${t}@msp.com`);
  const fa = authed(admin.cookie);
  const proj = (await (await fa('/api/projects', { method: 'POST', body: JSON.stringify({ name: 'Remote Co' }) })).json()).project;
  const join = await (await fetch(BASE + '/api/agent/join', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ projectId: proj.id, name: 'PC-Soporte' }) })).json();

  // PC conectado al chat (presencia).
  const pcChat = new WebSocket(WS_CHAT); track(pcChat); await open(pcChat);
  pcChat.send(JSON.stringify({ type: 'auth', equipoId: join.equipoId, agentSecret: join.agentSecret }));
  await waitFor(pcChat, (m) => m.type === 'ready');

  // Técnico lanza remoto → recibe code y el PC recibe la invitación.
  const launch = await (await fa(`/api/chat/remote/${join.equipoId}`, { method: 'POST' })).json();
  check(/^[0-9A-Z]{6}$/.test(launch.code || ''), `técnico lanza remoto → code ${launch.code}`);
  const inv = await waitFor(pcChat, (m) => m.type === 'remote-invite');
  check(inv.code === launch.code, 'el PC recibe la invitación con el mismo código');

  // Gate de rol: un usuario normal NO puede lanzar.
  const usr = await register(`usr_${t}@cli.com`);
  await fa(`/api/projects/${proj.id}/members`, { method: 'POST', body: JSON.stringify({ principalId: usr.user.id, role: 'usuario' }) });
  const denied = await authed(usr.cookie)(`/api/chat/remote/${join.equipoId}`, { method: 'POST' });
  check(denied.status === 403, 'usuario (no técnico) recibe 403 al intentar lanzar');

  // PC offline → 409.
  // (lo dejamos: el PC sigue online; probamos un equipo inexistente)
  const off = await fa(`/api/chat/remote/eq_${'x'.repeat(22)}`, { method: 'POST' });
  check(off.status === 404, 'PC inexistente → 404');

  // Emparejado por señalización con el código reservado.
  const op = new WebSocket(WS_SIGNAL); track(op); await open(op);
  op.send(JSON.stringify({ t: 'join', code: launch.code }));
  await waitFor(op, (m) => m.t === 'waiting');
  check(true, 'operador en espera (sala reservada, PC aún no comparte)');

  const host = new WebSocket(WS_SIGNAL); track(host); await open(host);
  host.send(JSON.stringify({ t: 'host', code: launch.code, name: 'PC-Soporte', mode: 'share', caps: [] }));
  await waitFor(host, (m) => m.t === 'hosted');
  const joined = await waitFor(op, (m) => m.t === 'joined');
  check(joined.mode === 'share', 'operador recibe joined al aceptar el PC');
  await waitFor(host, (m) => m.t === 'peer-joined');
  check(true, 'PC (host) recibe peer-joined → listos para WebRTC');

  pcChat.close(); op.close(); host.close();
  console.log(fail === 0 ? '\nTODOS LOS CHECKS DE REMOTO-DESDE-CHAT OK' : `\n${fail} FALLARON`);
  process.exit(fail ? 1 : 0);
}
main().catch((e) => { console.error('ERROR:', e.message); process.exit(1); });
