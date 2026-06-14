import { WebSocket } from 'ws';

const code = process.argv[2];
if (!code) {
  console.error('uso: node agent-handshake-test.mjs <CODE>');
  process.exit(2);
}

const ws = new WebSocket('ws://localhost:8080/ws/signal');
let joined = false;
let gotOffer = false;
let offerHasH264 = false;
let gotCandidate = false;

function finish(ok, msg) {
  console.log(ok ? 'PASS' : 'FAIL', '-', msg);
  try { ws.close(); } catch {}
  process.exit(ok ? 0 : 1);
}

ws.on('open', () => ws.send(JSON.stringify({ t: 'join', code })));
ws.on('message', (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.t === 'joined') {
    joined = true;
    console.log(`  joined: mode=${m.mode} caps=${JSON.stringify(m.caps)} name=${m.name ?? '?'}`);
    if (m.mode !== 'agent') finish(false, 'la sesión no es modo agent');
  } else if (m.t === 'error') {
    finish(false, `error del servidor: ${m.code}`);
  } else if (m.t === 'signal') {
    if (m.payload?.sdp?.type === 'offer') {
      gotOffer = true;
      offerHasH264 = /H264/i.test(m.payload.sdp.sdp);
      console.log(`  offer recibido (${m.payload.sdp.sdp.length} chars), H264=${offerHasH264}`);
      // Respondemos con un answer mínimo no es posible sin pila WebRTC real;
      // este test valida solo la generación de offer + ICE del agente.
    }
    if (m.payload?.candidate?.candidate) {
      if (!gotCandidate) console.log(`  ICE candidate recibido: ${m.payload.candidate.candidate.slice(0, 50)}…`);
      gotCandidate = true;
    }
  }
  if (joined && gotOffer && offerHasH264 && gotCandidate) {
    finish(true, 'agente hizo host, generó offer H.264 y envió ICE candidates');
  }
});
ws.on('error', (e) => finish(false, `ws error: ${e.message}`));
setTimeout(
  () => finish(false, `timeout (joined=${joined} offer=${gotOffer} h264=${offerHasH264} cand=${gotCandidate})`),
  10000,
);
