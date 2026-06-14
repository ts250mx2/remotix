import { WebSocket } from 'ws';

const BASE = 'http://localhost:8080';
const WS = BASE.replace('http', 'ws') + '/ws/chat';
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
async function connectChat(cookie) { const ws = new WebSocket(WS, { headers: { cookie } }); track(ws); await open(ws); await waitFor(ws, (m) => m.type === 'ready'); return ws; }

async function main() {
  const t = Date.now();
  const a = await register(`ca_${t}@t.com`);
  const fa = authed(a.cookie);
  const proj = (await (await fa('/api/projects', { method: 'POST', body: JSON.stringify({ name: 'Call Co' }) })).json()).project;
  // dos usuarios más como miembros del mismo canal
  const b = await register(`cb_${t}@t.com`);
  const cc = await register(`cc_${t}@t.com`);
  await fa(`/api/projects/${proj.id}/members`, { method: 'POST', body: JSON.stringify({ principalId: b.user.id, role: 'tecnico' }) });
  await fa(`/api/projects/${proj.id}/members`, { method: 'POST', body: JSON.stringify({ principalId: cc.user.id, role: 'tecnico' }) });
  const general = (await (await fa(`/api/chat/channels?empresa_id=${proj.id}`)).json()).channels[0];

  const wsA = await connectChat(a.cookie);
  const wsB = await connectChat(b.cookie);
  const wsC = await connectChat(cc.cookie);

  // A inicia llamada.
  wsA.send(JSON.stringify({ type: 'call-join', channelId: general.id }));
  const aPeers = await waitFor(wsA, (m) => m.type === 'call-peers');
  check(aPeers.peers.length === 0, 'A se une: 0 peers existentes');

  // B se une → recibe a A como peer; A es notificado de B.
  wsB.send(JSON.stringify({ type: 'call-join', channelId: general.id }));
  const bPeers = await waitFor(wsB, (m) => m.type === 'call-peers');
  check(bPeers.peers.includes(a.user.id), 'B se une: ve a A como peer existente');
  await waitFor(wsA, (m) => m.type === 'call-peer-joined' && m.peerId === b.user.id);
  check(true, 'A es notificado de que B entró');

  // C se une → ve a A y B.
  wsC.send(JSON.stringify({ type: 'call-join', channelId: general.id }));
  const cPeers = await waitFor(wsC, (m) => m.type === 'call-peers');
  check(cPeers.peers.includes(a.user.id) && cPeers.peers.includes(b.user.id), 'C se une: ve a A y B (malla de 3)');

  // Relay de señal dirigida: B → A.
  const aGetsSignal = waitFor(wsA, (m) => m.type === 'call-signal' && m.from === b.user.id);
  wsB.send(JSON.stringify({ type: 'call-signal', channelId: general.id, to: a.user.id, payload: { sdp: { type: 'offer', sdp: 'x' } } }));
  const sig = await aGetsSignal;
  check(sig.payload?.sdp?.type === 'offer', 'señal dirigida B→A relayed');

  // call-state activo difundido al canal (esperamos el de 3 participantes).
  const st = await waitFor(wsC, (m) => m.type === 'call-state' && m.active && m.peers.length === 3);
  check(st.peers.length === 3, `call-state activo con 3 participantes`);

  // B sale → A y C son notificados.
  const aLeft = waitFor(wsA, (m) => m.type === 'call-peer-left' && m.peerId === b.user.id);
  wsB.send(JSON.stringify({ type: 'call-leave', channelId: general.id }));
  await aLeft;
  check(true, 'salida de B notificada a A');

  wsA.close(); wsB.close(); wsC.close();
  console.log(fail === 0 ? '\nTODOS LOS CHECKS DE LLAMADA (malla) OK' : `\n${fail} FALLARON`);
  process.exit(fail ? 1 : 0);
}
main().catch((e) => { console.error('ERROR:', e.message); process.exit(1); });
