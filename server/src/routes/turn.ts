import { Hono } from 'hono';
import { createHmac } from 'node:crypto';
import { env } from '../env.js';

/**
 * Entrega la configuración de ICE (STUN + TURN) que usan tanto el agente como
 * la consola web para WebRTC. Si hay un TURN configurado (coturn con
 * `use-auth-secret`), genera credenciales EFÍMERAS firmadas con HMAC-SHA1:
 *   username   = <timestamp-de-expiración>
 *   credential = base64( HMAC-SHA1( turnSecret, username ) )
 * coturn revalida recomputando el HMAC, así que el secreto nunca viaja al
 * cliente y las credenciales caducan solas.
 *
 * Es público a propósito: el cliente anónimo de `/ayuda` también lo necesita.
 */
export interface RTCIceServerLike {
  urls: string | string[];
  username?: string;
  credential?: string;
}

/** Construye la lista de ICE servers (STUN + TURN efímero) reutilizable por el
 * endpoint HTTP y por la señalización (mensaje `hosted` del agente). */
export function buildIceServers(): { iceServers: RTCIceServerLike[]; ttl: number } {
  const iceServers: RTCIceServerLike[] = env.stunUrls.map((urls) => ({ urls }));

  if (env.turnHost && env.turnSecret) {
    const username = String(Math.floor(Date.now() / 1000) + env.turnTtl);
    const credential = createHmac('sha1', env.turnSecret).update(username).digest('base64');
    iceServers.push({
      urls: [
        `turn:${env.turnHost}:${env.turnPort}?transport=udp`,
        `turn:${env.turnHost}:${env.turnPort}?transport=tcp`,
        `turns:${env.turnHost}:${env.turnsPort}?transport=tcp`,
      ],
      username,
      credential,
    });
  }

  return { iceServers, ttl: env.turnTtl };
}

export const turnRoutes = new Hono().get('/', (c) => c.json(buildIceServers()));
