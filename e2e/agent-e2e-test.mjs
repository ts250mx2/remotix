// Test E2E del MODO AGENTE: un operador WebRTC real (werift) se conecta al
// agente vía la señalización, completa ICE, recibe el vídeo H.264, y ejerce
// control (mueve el cursor) y transferencia de archivos (escribe en disco).
// Verificaciones observables: paquetes de vídeo, posición del cursor, archivo.

import { RTCPeerConnection, RTCRtpCodecParameters, RTCIceCandidate } from 'werift';
import { WebSocket } from 'ws';
import { execSync } from 'node:child_process';
import { existsSync, readFileSync, rmSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const code = process.argv[2];
if (!code) { console.error('uso: node agent-e2e-test.mjs <CODE>'); process.exit(2); }

const SCREEN_W = 1920, SCREEN_H = 1080; // del selftest del agente
const remotixDir = join(process.env.USERPROFILE ?? '.', 'Downloads', 'Remotix');
const testFileName = 'remotix-e2e-test.txt';
const testFileContent = 'Remotix E2E ' + 'X'.repeat(5000); // ~5 KB, varios chunks

const r = { joined: false, connected: false, videoPackets: 0, videoBytes: 0, controlOpen: false, filesOpen: false, cursorMoved: false, cursorAt: '', fileWritten: false };

function cursorPos() {
  try {
    const out = execSync(
      'powershell -NoProfile -Command "Add-Type -AssemblyName System.Windows.Forms; $p=[System.Windows.Forms.Cursor]::Position; \\"$($p.X),$($p.Y)\\""',
    ).toString().trim();
    const [x, y] = out.split(',').map(Number);
    return { x, y };
  } catch { return { x: -1, y: -1 }; }
}

function cleanupTestFiles() {
  try {
    if (!existsSync(remotixDir)) return;
    for (const f of readdirSync(remotixDir)) {
      if (f.startsWith('remotix-e2e-test')) rmSync(join(remotixDir, f), { force: true });
    }
  } catch {}
}

const origin = cursorPos();
cleanupTestFiles();

const ws = new WebSocket('ws://localhost:8080/ws/signal');
let pc, controlDc, filesDc, ranTests = false;

ws.on('open', () => ws.send(JSON.stringify({ t: 'join', code })));
ws.on('message', async (raw) => {
  const m = JSON.parse(raw.toString());
  if (m.t === 'joined') { r.joined = true; setupPc(); }
  else if (m.t === 'signal') { await onSignal(m.payload); }
  else if (m.t === 'error') { console.log('error del servidor:', m.code); finish(); }
});

function setupPc() {
  pc = new RTCPeerConnection({
    codecs: {
      video: [new RTCRtpCodecParameters({
        mimeType: 'video/H264', clockRate: 90000,
        rtcpFeedback: [{ type: 'nack' }, { type: 'nack', parameter: 'pli' }, { type: 'goog-remb' }],
        parameters: 'level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f',
      })],
      audio: [],
    },
    iceServers: [{ urls: 'stun:stun.l.google.com:19302' }],
  });

  pc.onIceCandidate.subscribe((c) => {
    if (c) ws.send(JSON.stringify({ t: 'signal', payload: { candidate: { candidate: c.candidate, sdpMid: c.sdpMid, sdpMLineIndex: c.sdpMLineIndex } } }));
  });
  pc.connectionStateChange.subscribe((s) => {
    console.log('  estado conexión:', s);
    if (s === 'connected') { r.connected = true; setTimeout(runActiveTests, 2500); }
  });
  pc.onTrack.subscribe((track) => {
    console.log('  track recibido:', track.kind);
    track.onReceiveRtp.subscribe((rtp) => { r.videoPackets++; r.videoBytes += rtp.payload.length; });
  });
  pc.onDataChannel.subscribe((dc) => {
    if (dc.label === 'control') { controlDc = dc; r.controlOpen = true; console.log('  canal control abierto'); }
    else if (dc.label === 'files') { filesDc = dc; r.filesOpen = true; console.log('  canal files abierto'); }
  });
}

async function onSignal(payload) {
  if (payload?.sdp) {
    await pc.setRemoteDescription(payload.sdp);
    const answer = await pc.createAnswer();
    await pc.setLocalDescription(answer);
    ws.send(JSON.stringify({ t: 'signal', payload: { sdp: { type: answer.type, sdp: answer.sdp } } }));
  } else if (payload?.candidate?.candidate) {
    try {
      await pc.addIceCandidate(new RTCIceCandidate({ candidate: payload.candidate.candidate, sdpMid: payload.candidate.sdpMid, sdpMLineIndex: payload.candidate.sdpMLineIndex }));
    } catch (e) { /* candidato no aplicable */ }
  }
}

async function runActiveTests() {
  if (ranTests) return; ranTests = true;

  // 1) Control: mover el cursor al centro.
  if (controlDc) {
    for (let i = 0; i < 5; i++) { controlDc.send(JSON.stringify({ k: 'move', x: 0.5, y: 0.5 })); await sleep(60); }
    await sleep(500);
    const p = cursorPos();
    r.cursorAt = `${p.x},${p.y}`;
    r.cursorMoved = Math.abs(p.x - SCREEN_W / 2) < 250 && Math.abs(p.y - SCREEN_H / 2) < 250;
  }

  // 2) Archivos: enviar un archivo de prueba (operador → agente → disco).
  if (filesDc) {
    filesDc.send(JSON.stringify({ f: 'begin', id: 1, name: testFileName, size: testFileContent.length, mime: 'text/plain' }));
    await sleep(50);
    filesDc.send(Buffer.from(testFileContent));
    await sleep(50);
    filesDc.send(JSON.stringify({ f: 'end', id: 1 }));
    await sleep(1200);
    const path = join(remotixDir, testFileName);
    r.fileWritten = existsSync(path) && readFileSync(path, 'utf8') === testFileContent;
  }

  // Restaurar el cursor a su posición original.
  if (controlDc && origin.x >= 0) {
    controlDc.send(JSON.stringify({ k: 'move', x: origin.x / SCREEN_W, y: origin.y / SCREEN_H }));
  }

  await sleep(500);
  finish();
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function finish() {
  console.log('\n=== RESULTADOS E2E (modo agente) ===');
  console.log('  join + señalización :', r.joined ? 'OK' : 'FALLO');
  console.log('  WebRTC conectado    :', r.connected ? 'OK' : 'FALLO');
  console.log(`  vídeo H.264 recibido: ${r.videoPackets} paquetes, ${(r.videoBytes / 1024).toFixed(0)} KB ${r.videoPackets > 0 ? 'OK' : 'FALLO'}`);
  console.log('  canal control       :', r.controlOpen ? 'OK' : 'FALLO');
  console.log(`  control mueve cursor: ${r.cursorMoved ? 'OK' : 'FALLO'} (cursor en ${r.cursorAt}, esperado ~960,540)`);
  console.log('  canal files         :', r.filesOpen ? 'OK' : 'FALLO');
  console.log('  archivo en disco    :', r.fileWritten ? 'OK' : 'FALLO');
  const ok = r.joined && r.connected && r.videoPackets > 0 && r.controlOpen && r.cursorMoved && r.filesOpen && r.fileWritten;
  console.log('\n' + (ok ? 'TODOS LOS CHECKS E2E OK' : 'ALGUNOS CHECKS FALLARON'));
  cleanupTestFiles();
  try { pc?.close(); } catch {}
  try { ws.close(); } catch {}
  process.exit(ok ? 0 : 1);
}

setTimeout(() => { console.log('\n(timeout global)'); finish(); }, 20000);
