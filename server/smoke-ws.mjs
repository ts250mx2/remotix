import { WebSocket } from 'ws';

const URL = 'ws://localhost:8080/ws/signal';
const log = (...a) => console.log(...a);
let failures = 0;
const check = (cond, msg) => { if (cond) log('  OK  ', msg); else { failures++; log(' FAIL ', msg); } };
const wait = (ws, pred, ms = 2000) =>
  new Promise((res, rej) => {
    const to = setTimeout(() => rej(new Error('timeout esperando ' + pred.name)), ms);
    const on = (raw) => {
      const m = JSON.parse(raw.toString());
      if (pred(m)) { clearTimeout(to); ws.off('message', on); res(m); }
    };
    ws.on('message', on);
  });

const open = (ws) => new Promise((r) => ws.once('open', r));

async function main() {
  // 1) Código inválido → error not_found
  const bad = new WebSocket(URL);
  await open(bad);
  bad.send(JSON.stringify({ t: 'join', code: 'ZZZZZZ' }));
  const e = await wait(bad, (m) => m.t === 'error');
  check(e.code === 'not_found', `join con código inválido → error ${e.code}`);
  bad.close();

  // 2) Cliente hace host
  const client = new WebSocket(URL);
  await open(client);
  client.send(JSON.stringify({ t: 'host', name: 'María', issue: 'No abre el correo' }));
  const hosted = await wait(client, (m) => m.t === 'hosted');
  check(typeof hosted.code === 'string' && hosted.code.length === 6, `host → código de 6 chars (${hosted.code})`);

  // 3) Operador se une
  const op = new WebSocket(URL);
  await open(op);
  const clientPeerJoined = wait(client, (m) => m.t === 'peer-joined');
  op.send(JSON.stringify({ t: 'join', code: hosted.code }));
  const joined = await wait(op, (m) => m.t === 'joined');
  check(joined.name === 'María' && joined.issue === 'No abre el correo', 'operador recibe name/issue del cliente');
  await clientPeerJoined;
  check(true, 'cliente recibe peer-joined');

  // 4) Segundo operador → busy
  const op2 = new WebSocket(URL);
  await open(op2);
  op2.send(JSON.stringify({ t: 'join', code: hosted.code }));
  const busy = await wait(op2, (m) => m.t === 'error');
  check(busy.code === 'busy', `segundo operador → error ${busy.code}`);
  op2.close();

  // 5) Chat cliente → operador
  const opGetsChat = wait(op, (m) => m.t === 'chat');
  client.send(JSON.stringify({ t: 'chat', text: 'Hola, no me carga' }));
  const chat = await opGetsChat;
  check(chat.text === 'Hola, no me carga' && chat.from === 'client', 'chat cliente→operador con from=client');

  // 6) Signal (SDP/ICE) operador → cliente
  const clientGetsSignal = wait(client, (m) => m.t === 'signal');
  op.send(JSON.stringify({ t: 'signal', payload: { sdp: { type: 'answer', sdp: 'v=0...' } } }));
  const sig = await clientGetsSignal;
  check(sig.payload?.sdp?.type === 'answer', 'signal operador→cliente relayed');

  // 7) Operador se desconecta → cliente recibe peer-left
  const clientPeerLeft = wait(client, (m) => m.t === 'peer-left');
  op.close();
  await clientPeerLeft;
  check(true, 'cliente recibe peer-left al irse el operador');

  client.close();
  log(failures === 0 ? '\nTODOS LOS CHECKS OK' : `\n${failures} CHECK(S) FALLARON`);
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err) => { console.error('ERROR:', err.message); process.exit(1); });
