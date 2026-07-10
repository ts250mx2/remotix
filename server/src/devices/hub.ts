import type { IncomingMessage, Server } from 'node:http';
import { eq } from 'drizzle-orm';
import { WebSocketServer, type WebSocket } from 'ws';
import { db, tables } from '../db/index.js';
import { verifySecret } from '../auth/password.js';
import { readManifest, versionIsNewer } from '../routes/update.js';

// Dispositivos Lite desatendidos en línea (deviceId → sockets). Un mismo equipo
// puede tener VARIOS procesos conectados (la app con ventana + el ayudante del
// servicio): guardamos todos y el `start` se difunde a todos los vivos — el
// primero que se adjunta a la sala hospeda; el resto recibe 'taken' y se retira.
const online = new Map<string, Set<WebSocket>>();

// Keepalive: sin ping/pong un socket muerto (PC suspendida, NAT caído) queda
// OPEN para siempre y el equipo figura "en línea" sin estarlo; el `start` se
// perdería en el vacío y el operador se quedaría esperando indefinidamente.
const alive = new WeakMap<WebSocket, boolean>();
const PING_EVERY_MS = 30_000;

// Versión que reportó cada socket en su hello, para el push de actualización:
// al publicar una versión nueva, el servidor AVISA a los agentes conectados
// (mensaje {type:'update'}) en vez de esperar a que les toque sondear.
const reportedVersion = new WeakMap<WebSocket, string>();
const UPDATE_SWEEP_MS = 10 * 60_000;

function notifyIfOutdated(ws: WebSocket): void {
  const latest = readManifest('app');
  if (latest.version === '0.0.0') return;
  const current = reportedVersion.get(ws);
  // Agentes viejos no reportan versión: también se les avisa.
  if (!current || versionIsNewer(latest.version, current)) {
    if (ws.readyState === ws.OPEN) {
      ws.send(JSON.stringify({ type: 'update', version: latest.version, mandatory: !!latest.mandatory }));
    }
  }
}

export const deviceHub = {
  isOnline(deviceId: string): boolean {
    const set = online.get(deviceId);
    if (!set) return false;
    for (const ws of set) {
      if (ws.readyState === ws.OPEN) return true;
    }
    return false;
  },
  sendToDevice(deviceId: string, payload: unknown): boolean {
    const set = online.get(deviceId);
    if (!set) return false;
    const text = JSON.stringify(payload);
    let sent = false;
    for (const ws of set) {
      if (ws.readyState === ws.OPEN) {
        ws.send(text);
        sent = true;
      }
    }
    return sent;
  },
};

export function attachDeviceHub(server: Server): void {
  const wss = new WebSocketServer({ noServer: true, maxPayload: 16 * 1024 });

  server.on('upgrade', (req: IncomingMessage, socket, head) => {
    const url = new URL(req.url ?? '/', 'http://localhost');
    if (url.pathname !== '/ws/device') return;
    wss.handleUpgrade(req, socket, head, (ws) => wss.emit('connection', ws, req));
  });

  wss.on('connection', (ws: WebSocket) => {
    let deviceId: string | null = null;
    alive.set(ws, true);
    ws.on('pong', () => alive.set(ws, true));

    ws.on('message', async (data) => {
      let msg: Record<string, unknown>;
      try { msg = JSON.parse(data.toString()); } catch { return; }

      if (msg.type === 'hello' && !deviceId) {
        const dev = (await db.select().from(tables.devices).where(eq(tables.devices.id, String(msg.deviceId ?? ''))))[0];
        if (dev && (await verifySecret(String(msg.secret ?? ''), dev.secretHash))) {
          deviceId = dev.id;
          let set = online.get(deviceId);
          if (!set) { set = new Set(); online.set(deviceId, set); }
          set.add(ws);
          // El agente reporta su versión en el hello: la guardamos para saber qué
          // PC tiene qué versión instalada (visible en el panel).
          const version = typeof msg.version === 'string' ? msg.version.slice(0, 32) : null;
          if (version) reportedVersion.set(ws, version);
          await db.update(tables.devices)
            .set({ lastSeenAt: new Date(), ...(version ? { agentVersion: version } : {}) })
            .where(eq(tables.devices.id, deviceId));
          ws.send(JSON.stringify({ type: 'ready', accessKey: dev.accessKey, name: dev.name }));
          // Push de actualización: si este agente ya está desactualizado, que lo
          // sepa de inmediato (no dentro de 30 min cuando le toque sondear).
          notifyIfOutdated(ws);
        } else {
          ws.send(JSON.stringify({ type: 'error', code: 'auth_failed' }));
        }
      }
    });

    const gone = () => {
      if (!deviceId) return;
      const set = online.get(deviceId);
      if (!set) return;
      set.delete(ws);
      if (set.size === 0) online.delete(deviceId);
    };
    ws.on('close', gone);
    ws.on('error', gone);
  });

  // Barrido de keepalive: el que no respondió al ping anterior está muerto.
  setInterval(() => {
    for (const set of online.values()) {
      for (const ws of set) {
        if (alive.get(ws) === false) { ws.terminate(); continue; }
        alive.set(ws, false);
        ws.ping();
      }
    }
  }, PING_EVERY_MS).unref();

  // Barrido de actualización: cubre a los agentes con conexiones longevas que
  // ya estaban conectados cuando se publicó la versión nueva.
  setInterval(() => {
    for (const set of online.values()) {
      for (const ws of set) notifyIfOutdated(ws);
    }
  }, UPDATE_SWEEP_MS).unref();

  console.log('[ws] device hub activo en /ws/device');
}
