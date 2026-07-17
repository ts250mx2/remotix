import type { IncomingMessage, Server } from 'node:http';
import { randomInt } from 'node:crypto';
import { WebSocketServer, type WebSocket } from 'ws';
import { buildIceServers } from '../routes/turn.js';

/**
 * Señalización del helpdesk (soporte instantáneo, sin instalación).
 *
 * Modelo de "sala" efímera en memoria, identificada por un código corto que el
 * cliente dicta al técnico (estilo TeamViewer QuickSupport):
 *   - El CLIENTE (quien necesita ayuda) hace `host` → recibe un código.
 *   - El OPERADOR (técnico) hace `join` con ese código.
 * A partir de ahí el server solo retransmite mensajes `signal` (SDP/ICE de
 * WebRTC) y `chat` entre los dos peers. El vídeo de pantalla va P2P; el server
 * nunca lo ve.
 *
 * Protocolo (JSON por mensaje, campo `t` = tipo):
 *   cliente  → { t:'host', name?, issue? }      ← { t:'hosted', code }
 *   operador → { t:'join', code }               ← { t:'joined', name?, issue? }  | { t:'error', code }
 *   ambos    → { t:'signal', payload }          → al peer: { t:'signal', payload }
 *   ambos    → { t:'chat', text }               → al peer: { t:'chat', text, from, ts }
 *   ambos    → { t:'bye' }
 * Eventos del server:  { t:'peer-joined', ... } | { t:'peer-left' } | { t:'error', code }
 */

interface Session {
  code: string;
  client: WebSocket | null;   // peer que comparte pantalla (host)
  operator: WebSocket | null; // peer que ve (técnico)
  name?: string;
  issue?: string;
  mode: 'share' | 'agent';    // 'share' = pantalla de navegador (solo ver), 'agent' = control total
  caps: string[];             // capacidades del host, ej: ['control']
  createdAt: number;
}

interface ConnMeta {
  code: string;
  role: 'client' | 'operator';
}

const SESSIONS = new Map<string, Session>();
const CONNS = new WeakMap<WebSocket, ConnMeta>();
// Sockets vivos de /ws/signal (para el barrido de keepalive) y su estado de pong.
const ALL_CONNS = new Set<WebSocket>();
const alive = new WeakMap<WebSocket, boolean>();

// Alfabeto sin caracteres ambiguos (0/O, 1/I/L) para que sea fácil de dictar.
const CODE_ALPHABET = '23456789ABCDEFGHJKMNPQRSTUVWXYZ';
const CODE_LEN = 6;
const MAX_SESSIONS = 2000;
const MAX_MSG_BYTES = 64 * 1024;          // SDP/ICE caben de sobra; corta abusos.
const SESSION_TTL_MS = 1000 * 60 * 60 * 2; // 2 h sin actividad → barrida.

function genCode(): string {
  for (let attempt = 0; attempt < 20; attempt++) {
    let code = '';
    for (let i = 0; i < CODE_LEN; i++) {
      code += CODE_ALPHABET[randomInt(CODE_ALPHABET.length)];
    }
    if (!SESSIONS.has(code)) return code;
  }
  throw new Error('no se pudo generar un código único');
}

/** Reserva una sala de señalización con un código único para una sesión remota
 * lanzada desde el chat. El PC luego hace `host` con ese código y se adjunta. */
export function reserveRemoteSession(opts: { name?: string }): string {
  const code = genCode();
  SESSIONS.set(code, {
    code,
    client: null,
    operator: null,
    name: opts.name,
    mode: 'share',
    caps: [],
    createdAt: Date.now(),
  });
  return code;
}

function send(ws: WebSocket | null | undefined, msg: unknown): void {
  if (ws && ws.readyState === ws.OPEN) {
    ws.send(JSON.stringify(msg));
  }
}

function peerOf(session: Session, role: 'client' | 'operator'): WebSocket | null {
  return role === 'client' ? session.operator : session.client;
}

function dropSession(session: Session): void {
  SESSIONS.delete(session.code);
}

export function attachSignaling(server: Server): void {
  const wss = new WebSocketServer({ noServer: true, maxPayload: MAX_MSG_BYTES });

  server.on('upgrade', (req: IncomingMessage, socket, head) => {
    const url = new URL(req.url ?? '/', 'http://localhost');
    if (url.pathname !== '/ws/signal') return;
    wss.handleUpgrade(req, socket, head, (ws) => {
      wss.emit('connection', ws, req);
    });
  });

  wss.on('connection', (ws: WebSocket) => {
    alive.set(ws, true);
    ws.on('pong', () => alive.set(ws, true));
    ws.on('message', (data) => {
      let msg: Record<string, unknown>;
      try {
        msg = JSON.parse(data.toString());
      } catch {
        return send(ws, { t: 'error', code: 'bad_json' });
      }
      handleMessage(ws, msg);
    });

    ws.on('close', () => handleClose(ws));
    ws.on('error', () => handleClose(ws));

    ALL_CONNS.add(ws);
    ws.on('close', () => ALL_CONNS.delete(ws));
  });

  // Keepalive: mata sockets zombis (PC suspendida, red caída) para que el peer
  // reciba 'peer-left' en vez de esperar para siempre.
  setInterval(() => {
    for (const ws of ALL_CONNS) {
      if (alive.get(ws) === false) { ws.terminate(); continue; }
      alive.set(ws, false);
      ws.ping();
    }
  }, 30_000).unref();

  // Barrida periódica de salas abandonadas.
  setInterval(() => {
    const now = Date.now();
    for (const session of SESSIONS.values()) {
      const dead =
        (!session.client || session.client.readyState === session.client.CLOSED) &&
        (!session.operator || session.operator.readyState === session.operator.CLOSED);
      if (dead || now - session.createdAt > SESSION_TTL_MS) dropSession(session);
    }
  }, 1000 * 60 * 5).unref();

  console.log('[ws] helpdesk signaling activo en /ws/signal');
}

function handleMessage(ws: WebSocket, msg: Record<string, unknown>): void {
  const t = msg.t;

  // Mensajes previos a unirse a una sala.
  if (t === 'host') {
    if (CONNS.has(ws)) return send(ws, { t: 'error', code: 'already_in_session' });

    // Si el técnico lanzó la sesión desde el chat, viene un código reservado:
    // el PC se ADJUNTA a esa sala en vez de crear una nueva. Como el `start` se
    // difunde a TODOS los procesos del equipo, solo el primero se adjunta; a los
    // demás (o si la sala expiró) se les responde 'taken' para que se retiren —
    // antes caían al caso general y hospedaban una sala fantasma para siempre.
    const wanted = typeof msg.code === 'string' ? msg.code.trim().toUpperCase() : '';
    if (wanted) {
      const reserved = SESSIONS.get(wanted);
      if (!reserved) return send(ws, { t: 'error', code: 'taken' });
      // Solo un cliente VIVO bloquea la sala. Si el socket del intento anterior
      // quedó muerto (PC suspendida, sesión fallida sin cierre limpio), se permite
      // re-hospedar en su lugar en vez de responder 'taken' para siempre.
      const clientAlive = reserved.client && reserved.client.readyState === reserved.client.OPEN;
      if (clientAlive && reserved.client !== ws) return send(ws, { t: 'error', code: 'taken' });
      reserved.client = ws;
      if (typeof msg.name === 'string') reserved.name = msg.name.slice(0, 120);
      if (typeof msg.issue === 'string') reserved.issue = msg.issue.slice(0, 1000);
      reserved.mode = msg.mode === 'agent' ? 'agent' : 'share';
      reserved.caps = Array.isArray(msg.caps) ? msg.caps.filter((x): x is string => typeof x === 'string').slice(0, 16) : [];
      CONNS.set(ws, { code: wanted, role: 'client' });
      send(ws, { t: 'hosted', code: wanted, ...buildIceServers() });
      // Si el operador ya estaba esperando, ahora ambos quedan emparejados.
      if (reserved.operator && reserved.operator.readyState === reserved.operator.OPEN) {
        send(reserved.operator, { t: 'joined', name: reserved.name, issue: reserved.issue, mode: reserved.mode, caps: reserved.caps });
        send(ws, { t: 'peer-joined' });
      }
      return;
    }

    if (SESSIONS.size >= MAX_SESSIONS) return send(ws, { t: 'error', code: 'server_busy' });
    const code = genCode();
    const session: Session = {
      code,
      client: ws,
      operator: null,
      name: typeof msg.name === 'string' ? msg.name.slice(0, 120) : undefined,
      issue: typeof msg.issue === 'string' ? msg.issue.slice(0, 1000) : undefined,
      mode: msg.mode === 'agent' ? 'agent' : 'share',
      caps: Array.isArray(msg.caps) ? msg.caps.filter((x): x is string => typeof x === 'string').slice(0, 16) : [],
      createdAt: Date.now(),
    };
    SESSIONS.set(code, session);
    CONNS.set(ws, { code, role: 'client' });
    // Incluimos la config ICE para que el agente no necesite un cliente HTTP aparte.
    return send(ws, { t: 'hosted', code, ...buildIceServers() });
  }

  if (t === 'join') {
    if (CONNS.has(ws)) return send(ws, { t: 'error', code: 'already_in_session' });
    const code = typeof msg.code === 'string' ? msg.code.trim().toUpperCase() : '';
    const session = SESSIONS.get(code);
    if (!session) return send(ws, { t: 'error', code: 'not_found' });
    if (session.operator && session.operator.readyState === session.operator.OPEN && session.operator !== ws) {
      return send(ws, { t: 'error', code: 'busy' });
    }
    session.operator = ws;
    CONNS.set(ws, { code, role: 'operator' });
    if (!session.client) {
      // Sala reservada (lanzada desde el chat): el PC aún no acepta. Esperar.
      return send(ws, { t: 'waiting' });
    }
    send(ws, { t: 'joined', name: session.name, issue: session.issue, mode: session.mode, caps: session.caps });
    send(session.client, { t: 'peer-joined' });
    return;
  }

  // A partir de aquí el socket debe pertenecer a una sala.
  const meta = CONNS.get(ws);
  if (!meta) return send(ws, { t: 'error', code: 'not_in_session' });
  const session = SESSIONS.get(meta.code);
  if (!session) return send(ws, { t: 'error', code: 'session_gone' });
  const peer = peerOf(session, meta.role);

  if (t === 'signal') {
    return send(peer, { t: 'signal', payload: msg.payload });
  }

  if (t === 'chat') {
    const text = typeof msg.text === 'string' ? msg.text.slice(0, 4000) : '';
    if (!text) return;
    return send(peer, { t: 'chat', text, from: meta.role, ts: Date.now() });
  }

  if (t === 'bye') {
    return ws.close();
  }

  send(ws, { t: 'error', code: 'unknown_type' });
}

function handleClose(ws: WebSocket): void {
  const meta = CONNS.get(ws);
  if (!meta) return;
  CONNS.delete(ws);
  const session = SESSIONS.get(meta.code);
  if (!session) return;

  if (meta.role === 'client') {
    // El host se fue: la sala termina.
    send(session.operator, { t: 'peer-left' });
    dropSession(session);
  } else {
    // El operador se fue: liberamos su lugar, el cliente puede seguir esperando.
    session.operator = null;
    send(session.client, { t: 'peer-left' });
  }
}
