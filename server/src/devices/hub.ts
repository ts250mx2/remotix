import type { IncomingMessage, Server } from 'node:http';
import { eq } from 'drizzle-orm';
import { WebSocketServer, type WebSocket } from 'ws';
import { db, tables } from '../db/index.js';
import { verifySecret } from '../auth/password.js';

// Dispositivos Lite desatendidos en línea (deviceId → socket).
const online = new Map<string, WebSocket>();

export const deviceHub = {
  isOnline(deviceId: string): boolean {
    const ws = online.get(deviceId);
    return !!ws && ws.readyState === ws.OPEN;
  },
  sendToDevice(deviceId: string, payload: unknown): boolean {
    const ws = online.get(deviceId);
    if (ws && ws.readyState === ws.OPEN) {
      ws.send(JSON.stringify(payload));
      return true;
    }
    return false;
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

    ws.on('message', async (data) => {
      let msg: Record<string, unknown>;
      try { msg = JSON.parse(data.toString()); } catch { return; }

      if (msg.type === 'hello' && !deviceId) {
        const dev = (await db.select().from(tables.devices).where(eq(tables.devices.id, String(msg.deviceId ?? ''))))[0];
        if (dev && (await verifySecret(String(msg.secret ?? ''), dev.secretHash))) {
          deviceId = dev.id;
          online.set(deviceId, ws);
          // El agente reporta su versión en el hello: la guardamos para saber qué
          // PC tiene qué versión instalada (visible en el panel).
          const version = typeof msg.version === 'string' ? msg.version.slice(0, 32) : null;
          await db.update(tables.devices)
            .set({ lastSeenAt: new Date(), ...(version ? { agentVersion: version } : {}) })
            .where(eq(tables.devices.id, deviceId));
          ws.send(JSON.stringify({ type: 'ready', accessKey: dev.accessKey, name: dev.name }));
        } else {
          ws.send(JSON.stringify({ type: 'error', code: 'auth_failed' }));
        }
      }
    });

    const gone = () => { if (deviceId && online.get(deviceId) === ws) online.delete(deviceId); };
    ws.on('close', gone);
    ws.on('error', gone);
  });

  console.log('[ws] device hub activo en /ws/device');
}
