import { WebSocket } from 'ws';

const BASE = 'http://localhost:8080';
let failures = 0;
const check = (c, m) => { console.log(c ? '  OK  ' : ' FAIL ', m); if (!c) failures++; };
const open = (ws) => new Promise((r) => ws.once('open', r));
// Buffer de mensajes para evitar carreras: empezamos a escuchar al crear el ws.
function track(ws) { ws._buf = []; ws.on('message', (d) => ws._buf.push(JSON.parse(d.toString()))); }
const wsWait = (ws, pred, ms = 3000) =>
  new Promise((res, rej) => {
    const hit = ws._buf.find(pred);
    if (hit) return res(hit);
    const to = setTimeout(() => rej(new Error('timeout; buffer=' + JSON.stringify(ws._buf))), ms);
    const on = (d) => { const m = JSON.parse(d.toString()); if (pred(m)) { clearTimeout(to); ws.off('message', on); res(m); } };
    ws.on('message', on);
  });

async function register(email) {
  const res = await fetch(BASE + '/api/auth/register', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email, password: 'password123', name: email.split('@')[0] }),
  });
  const setCookie = res.headers.getSetCookie ? res.headers.getSetCookie() : [res.headers.get('set-cookie')];
  const cookie = (setCookie.find((c) => c && c.startsWith('remotix_session=')) || '').split(';')[0];
  return { cookie, user: (await res.json()).user };
}
const authed = (cookie) => (path, opts = {}) =>
  fetch(BASE + path, { ...opts, headers: { ...(opts.headers || {}), cookie, ...(opts.body ? { 'content-type': 'application/json' } : {}) } });

async function main() {
  const t = Date.now();
  const A = await register(`a_${t}@t.com`);
  const B = await register(`b_${t}@t.com`);
  check(!!A.user && !!B.user, 'dos usuarios registrados');
  const fa = authed(A.cookie);

  const projResp = await fa('/api/projects', { method: 'POST', body: JSON.stringify({ name: 'Cliente Demo' }) });
  const proj = await projResp.json();
  console.log('  DEBUG project:', projResp.status, JSON.stringify(proj));
  const empresaId = proj.project.id;
  const memResp = await fa(`/api/projects/${empresaId}/members`, { method: 'POST', body: JSON.stringify({ principalId: B.user.id, role: 'tecnico' }) });
  console.log('  DEBUG add member:', memResp.status, await memResp.text());

  const chRes = await fa(`/api/chat/channels?empresa_id=${empresaId}`);
  const ch = await chRes.json();
  if (!ch.channels) console.log('  DEBUG channels:', chRes.status, JSON.stringify(ch));
  check((ch.channels?.length ?? 0) >= 1, `canal general autocreado (${ch.channels?.length})`);
  const general = ch.channels[0];

  const wsUrl = BASE.replace('http', 'ws') + '/ws/chat';
  const wsA = new WebSocket(wsUrl, { headers: { cookie: A.cookie } });
  const wsB = new WebSocket(wsUrl, { headers: { cookie: B.cookie } });
  track(wsA); track(wsB);
  await Promise.all([open(wsA), open(wsB)]);
  await Promise.all([wsWait(wsA, (m) => m.type === 'ready'), wsWait(wsB, (m) => m.type === 'ready')]);
  check(true, 'ambos autenticados por cookie (ready)');

  const bGet = wsWait(wsB, (m) => m.type === 'message');
  wsA.send(JSON.stringify({ type: 'message', channelId: general.id, body: 'hola equipo' }));
  const got = await bGet;
  check(got.message?.body === 'hola equipo' && got.message.senderId === A.user.id, 'mensaje WS A→B en tiempo real');

  const bGet2 = wsWait(wsB, (m) => m.type === 'message' && m.message.body === 'desde REST');
  await fa(`/api/chat/channels/${general.id}/messages`, { method: 'POST', body: JSON.stringify({ body: 'desde REST' }) });
  await bGet2;
  check(true, 'mensaje REST→WS en tiempo real');

  const hist = await (await fa(`/api/chat/channels/${general.id}/messages`)).json();
  check((hist.messages?.length ?? 0) >= 2 && hist.messages.some((m) => m.body === 'hola equipo'), `historial persistente (${hist.messages?.length} msgs)`);

  const roster = await (await fa(`/api/chat/empresas/${empresaId}/roster`)).json();
  const aEntry = roster.roster?.find((r) => r.id === A.user.id);
  check(aEntry?.online === true, `roster con presencia (${roster.roster?.length} entradas, A online=${aEntry?.online})`);

  wsA.close(); wsB.close();
  console.log(failures === 0 ? '\nTODOS LOS CHECKS DE CHAT OK' : `\n${failures} CHECK(S) FALLARON`);
  process.exit(failures === 0 ? 0 : 1);
}
main().catch((e) => { console.error('ERROR:', e.message); process.exit(1); });
