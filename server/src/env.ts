function list(value: string | undefined, fallback: string): string[] {
  return (value ?? fallback).split(',').map((s) => s.trim()).filter(Boolean);
}

export const env = {
  port: Number(process.env.PORT ?? 8080),
  sessionSecret: process.env.SESSION_SECRET ?? 'dev-only-change-me',
  isDev: process.env.NODE_ENV !== 'production',

  // Base de datos MySQL (servidor existente; credenciales por entorno).
  mysql: {
    host: process.env.MYSQL_HOST ?? '127.0.0.1',
    port: Number(process.env.MYSQL_PORT ?? 3306),
    user: process.env.MYSQL_USER ?? 'remotix',
    password: process.env.MYSQL_PASSWORD ?? '',
    database: process.env.MYSQL_DATABASE ?? 'remotix',
    connectionLimit: Number(process.env.MYSQL_POOL ?? 10),
  },

  // WebRTC ICE / NAT traversal.
  stunUrls: list(process.env.STUN_URLS, 'stun:stun.l.google.com:19302,stun:stun1.l.google.com:19302'),
  turnHost: process.env.TURN_HOST ?? '',        // ej: turn.midominio.com (vacío = sin TURN)
  turnSecret: process.env.TURN_SECRET ?? '',    // shared secret con coturn (use-auth-secret)
  turnPort: Number(process.env.TURN_PORT ?? 3478),
  turnsPort: Number(process.env.TURNS_PORT ?? 5349),
  // Anunciar turns: (TURN sobre TLS) SOLO si coturn tiene certificado y el
  // puerto está abierto; anunciarlo cerrado hace que cada cliente pierda ~5 s
  // intentándolo en cada conexión.
  turnsEnabled: process.env.TURNS_ENABLED === '1',
  turnTtl: Number(process.env.TURN_TTL ?? 3600), // segundos de validez de la credencial efímera
};

if (!env.isDev && env.sessionSecret === 'dev-only-change-me') {
  throw new Error('SESSION_SECRET must be set in production');
}
