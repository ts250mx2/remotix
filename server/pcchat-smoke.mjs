import { WebSocket } from 'ws';

const BASE = 'http://localhost:8080';
let fail = 0;
const check = (c, m) => { console.log(c ? '  OK  ' : ' FAIL ', m); if (!c) fail++; };
const open = (ws) => new Promise((r) => ws.once('open', r));
function track(ws) { ws._buf = []; ws.on('message', (d) => ws._buf.push(JSON.parse(d.toString()))); }
const waitFor = (ws, pred, ms = 3000) =>
  new Promise((res, rej) => {
    const hit = ws._buf.find(pred);
    if (hit) return res(hit);
    const to = setTimeout(() => rej(new Error('timeout; buf=' + JSON.stringify(ws._buf))), ms);
    const on = (d) => { const m = JSON.parse(d.toString()); if (pred(m)) { clearTimeout(to); ws.off('message', on); res(m); } };
    ws.on('message', on);
  });

async function register(email) {
  const res = await fetch(BASE + '/api/auth/register', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email, password: 'password123', name: 'Tecnico' }),
  });
  const sc = res.headers.getSetCookie ? res.headers.getSetCookie() : [res.headers.get('set-cookie')];
  return { cookie: (sc.find((c) => c && c.startsWith('remotix_session=')) || '').split(';')[0], user: (await res.json()).user };
}
const wsUrl = BASE.replace('http', 'ws') + '/ws/chat';

async function main() {
  const admin = await register(`adm_${Date.now()}@msp.com`);
  const fa = (p, o = {}) => fetch(BASE + p, { ...o, headers: { ...(o.headers || {}), cookie: admin.cookie, ...(o.body ? { 'content-type': 'application/json' } : {}) } });
  const proj = (await (await fa('/api/projects', { method: 'POST', body: JSON.stringify({ name: 'PCChat Co' }) })).json()).project;

  // PC se une SOLO con el UUID.
  const join = await (await fetch(BASE + '/api/agent/join', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ projectId: proj.id, name: 'PC-Caja1' }),
  })).json();
  check(/^eq_/.test(join.equipoId), `PC unido por UUID (${join.equipoId})`);

  // Técnico (humano) conecta por cookie.
  const wsAdmin = new WebSocket(wsUrl, { headers: { cookie: admin.cookie } });
  track(wsAdmin); await open(wsAdmin);
  const adminReady = await waitFor(wsAdmin, (m) => m.type === 'ready');
  const general = adminReady.channels.find((c) => c.name === 'general');
  check(!!general, `técnico ve el canal general en ready (${adminReady.channels.length} canales)`);

  // PC conecta y se autentica como equipo (sin cookie).
  const wsPc = new WebSocket(wsUrl);
  track(wsPc); await open(wsPc);
  wsPc.send(JSON.stringify({ type: 'auth', equipoId: join.equipoId, agentSecret: join.agentSecret }));
  const pcReady = await waitFor(wsPc, (m) => m.type === 'ready');
  check(pcReady.self?.kind === 'pc' && pcReady.channels.some((c) => c.id === general.id), 'PC autenticado y ve el canal general');

  // PC escribe → el técnico lo recibe.
  const adminGets = waitFor(wsAdmin, (m) => m.type === 'message' && m.message.body === 'hola desde el PC');
  wsPc.send(JSON.stringify({ type: 'message', channelId: general.id, body: 'hola desde el PC' }));
  const got = await adminGets;
  check(got.message.senderKind === 'pc' && got.message.senderId === join.equipoId, 'mensaje del PC llega al técnico (senderKind=pc)');

  // PC pide soporte → el técnico recibe el SOS.
  const sos = waitFor(wsAdmin, (m) => m.type === 'message' && m.message.body.includes('soporte'));
  wsPc.send(JSON.stringify({ type: 'support', channelId: general.id }));
  await sos;
  check(true, 'el técnico recibe la solicitud de soporte del PC');

  // PC pide historial.
  wsPc.send(JSON.stringify({ type: 'history', channelId: general.id }));
  const hist = await waitFor(wsPc, (m) => m.type === 'history');
  check(hist.messages.length >= 2, `PC recibe historial por WS (${hist.messages.length} msgs)`);

  wsAdmin.close(); wsPc.close();
  console.log(fail === 0 ? '\nTODOS LOS CHECKS PC-CHAT OK' : `\n${fail} FALLARON`);
  process.exit(fail ? 1 : 0);
}
main().catch((e) => { console.error('ERROR:', e.message); process.exit(1); });
