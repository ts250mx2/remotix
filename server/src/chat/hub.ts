import type { IncomingMessage, Server } from 'node:http';
import { eq } from 'drizzle-orm';
import { WebSocketServer, type WebSocket } from 'ws';
import { db, tables } from '../db/index.js';
import { getSession, COOKIE_NAME } from '../auth/session.js';
import { verifySecret } from '../auth/password.js';
import {
  channelMemberIds,
  channelsForPrincipalIn,
  empresaIdsForPrincipal,
  getMessages,
  isChannelMember,
  persistMessage,
  type Attachment,
  type MessageDTO,
  type SenderKind,
} from './service.js';

interface Principal {
  kind: 'user' | 'pc';
  id: string;
  name: string;
  empresaIds: string[];
}

const connections = new Map<string, Set<WebSocket>>(); // principalId → sockets
const meta = new WeakMap<WebSocket, Principal>();
const calls = new Map<string, Set<string>>(); // channelId → principalIds en la videollamada

function addConn(p: Principal, ws: WebSocket): boolean {
  let set = connections.get(p.id);
  const wasOffline = !set || set.size === 0;
  if (!set) { set = new Set(); connections.set(p.id, set); }
  set.add(ws);
  meta.set(ws, p);
  return wasOffline;
}

function removeConn(ws: WebSocket): { principal: Principal; nowOffline: boolean } | null {
  const p = meta.get(ws);
  if (!p) return null;
  meta.delete(ws);
  const set = connections.get(p.id);
  if (set) {
    set.delete(ws);
    if (set.size === 0) connections.delete(p.id);
  }
  return { principal: p, nowOffline: !connections.has(p.id) };
}

function send(ws: WebSocket, payload: unknown): void {
  if (ws.readyState === ws.OPEN) ws.send(JSON.stringify(payload));
}

export const chatHub = {
  isOnline(id: string): boolean {
    return (connections.get(id)?.size ?? 0) > 0;
  },

  /** Envía un payload a todas las conexiones de un principal (usuario o PC). */
  sendToPrincipal(id: string, payload: unknown): boolean {
    const set = connections.get(id);
    if (!set || set.size === 0) return false;
    for (const ws of set) send(ws, payload);
    return true;
  },

  /** Difunde a todos los miembros conectados de un canal. */
  async broadcastToChannel(channelId: string, payload: unknown): Promise<void> {
    const memberIds = await channelMemberIds(channelId);
    for (const id of memberIds) {
      const set = connections.get(id);
      if (set) for (const ws of set) send(ws, payload);
    }
  },

  /** Persiste un mensaje y lo difunde al canal. Usado por REST y por el WS. */
  async postAndBroadcast(channelId: string, senderId: string, senderKind: SenderKind, body: string, attachment?: Attachment): Promise<MessageDTO> {
    const msg = await persistMessage(channelId, senderId, senderKind, body, attachment);
    await this.broadcastToChannel(channelId, { type: 'message', message: msg });
    return msg;
  },
};

async function broadcastCallState(channelId: string): Promise<void> {
  const peers = Array.from(calls.get(channelId) ?? []);
  await chatHub.broadcastToChannel(channelId, { type: 'call-state', channelId, active: peers.length > 0, peers });
}

/** Saca a un principal de todas las videollamadas y avisa a los demás. */
async function leaveAllCalls(principalId: string): Promise<void> {
  for (const [channelId, peers] of calls) {
    if (peers.delete(principalId)) {
      for (const other of peers) chatHub.sendToPrincipal(other, { type: 'call-peer-left', channelId, peerId: principalId });
      if (peers.size === 0) calls.delete(channelId);
      await broadcastCallState(channelId);
    }
  }
}

function broadcastPresence(principal: Principal, online: boolean): void {
  const payload = { type: 'presence', id: principal.id, kind: principal.kind, online };
  for (const [, set] of connections) {
    for (const ws of set) {
      const p = meta.get(ws);
      if (p && p.id !== principal.id && p.empresaIds.some((e) => principal.empresaIds.includes(e))) {
        send(ws, payload);
      }
    }
  }
}

async function cookieUser(req: IncomingMessage): Promise<Principal | null> {
  const cookie = req.headers.cookie ?? '';
  const match = cookie.split(';').map((s) => s.trim()).find((s) => s.startsWith(`${COOKIE_NAME}=`));
  if (!match) return null;
  const token = match.slice(COOKIE_NAME.length + 1);
  const session = await getSession(token);
  if (!session) return null;
  const user = (await db.select().from(tables.users).where(eq(tables.users.id, session.userId)))[0];
  if (!user) return null;
  const empresaIds = await empresaIdsForPrincipal('user', user.id);
  return { kind: 'user', id: user.id, name: user.name, empresaIds };
}

async function authPc(equipoId: string, agentSecret: string): Promise<Principal | null> {
  const eqp = (await db.select().from(tables.equipos).where(eq(tables.equipos.id, equipoId)))[0];
  if (!eqp) return null;
  const ok = await verifySecret(agentSecret, eqp.agentSecretHash);
  if (!ok) return null;
  await db.update(tables.equipos).set({ lastSeenAt: new Date() }).where(eq(tables.equipos.id, equipoId));
  return { kind: 'pc', id: eqp.id, name: eqp.name, empresaIds: [eqp.projectId] };
}

async function register(ws: WebSocket, principal: Principal): Promise<void> {
  const wasOffline = addConn(principal, ws);
  const channels = await channelsForPrincipalIn(principal.empresaIds, principal.id);
  send(ws, {
    type: 'ready',
    self: { id: principal.id, kind: principal.kind, name: principal.name },
    empresaIds: principal.empresaIds,
    channels,
  });
  if (wasOffline) broadcastPresence(principal, true);
}

export function attachChat(server: Server): void {
  const wss = new WebSocketServer({ noServer: true, maxPayload: 256 * 1024 });

  server.on('upgrade', (req, socket, head) => {
    const url = new URL(req.url ?? '/', 'http://localhost');
    if (url.pathname !== '/ws/chat') return;
    wss.handleUpgrade(req, socket, head, (ws) => wss.emit('connection', ws, req));
  });

  wss.on('connection', async (ws: WebSocket, req: IncomingMessage) => {
    // Humanos: autenticados por cookie de sesión al instante.
    const user = await cookieUser(req);
    if (user) await register(ws, user);

    ws.on('message', async (data) => {
      let msg: Record<string, unknown>;
      try { msg = JSON.parse(data.toString()); } catch { return; }

      // PC: se autentica con un mensaje 'auth'.
      if (msg.type === 'auth' && !meta.has(ws)) {
        const principal = await authPc(String(msg.equipoId ?? ''), String(msg.agentSecret ?? ''));
        if (principal) await register(ws, principal);
        else send(ws, { type: 'error', code: 'auth_failed' });
        return;
      }

      const self = meta.get(ws);
      if (!self) return send(ws, { type: 'error', code: 'unauthenticated' });

      if (msg.type === 'message') {
        const channelId = String(msg.channelId ?? '');
        const body = typeof msg.body === 'string' ? msg.body.slice(0, 8000) : '';
        if (!channelId || !body) return;
        if (!(await isChannelMember(channelId, self.id))) return send(ws, { type: 'error', code: 'not_member' });
        await chatHub.postAndBroadcast(channelId, self.id, self.kind, body);
      } else if (msg.type === 'history') {
        const channelId = String(msg.channelId ?? '');
        if (!channelId || !(await isChannelMember(channelId, self.id))) return;
        const messages = await getMessages(channelId, 50);
        send(ws, { type: 'history', channelId, messages });
      } else if (msg.type === 'support') {
        const channelId = String(msg.channelId ?? '');
        if (!channelId || !(await isChannelMember(channelId, self.id))) return;
        await chatHub.postAndBroadcast(channelId, self.id, self.kind, '🆘 Solicito soporte remoto');
      } else if (msg.type === 'call-join') {
        const channelId = String(msg.channelId ?? '');
        if (!channelId || !(await isChannelMember(channelId, self.id))) return;
        let set = calls.get(channelId);
        if (!set) { set = new Set(); calls.set(channelId, set); }
        const existing = Array.from(set).filter((id) => id !== self.id);
        set.add(self.id);
        // Al que entra le damos los peers actuales (él ofertará a cada uno).
        send(ws, { type: 'call-peers', channelId, peers: existing });
        for (const peer of existing) chatHub.sendToPrincipal(peer, { type: 'call-peer-joined', channelId, peerId: self.id });
        await broadcastCallState(channelId);
      } else if (msg.type === 'call-leave') {
        const channelId = String(msg.channelId ?? '');
        const set = calls.get(channelId);
        if (set && set.delete(self.id)) {
          for (const peer of set) chatHub.sendToPrincipal(peer, { type: 'call-peer-left', channelId, peerId: self.id });
          if (set.size === 0) calls.delete(channelId);
          await broadcastCallState(channelId);
        }
      } else if (msg.type === 'call-signal') {
        const channelId = String(msg.channelId ?? '');
        const to = String(msg.to ?? '');
        const set = calls.get(channelId);
        if (set && set.has(self.id) && set.has(to)) {
          chatHub.sendToPrincipal(to, { type: 'call-signal', channelId, from: self.id, payload: msg.payload });
        }
      }
    });

    const onGone = () => {
      const r = removeConn(ws);
      if (r && r.nowOffline) {
        broadcastPresence(r.principal, false);
        void leaveAllCalls(r.principal.id);
      }
    };
    ws.on('close', onGone);
    ws.on('error', onGone);
  });

  console.log('[ws] chat hub activo en /ws/chat');
}
