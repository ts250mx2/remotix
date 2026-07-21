import type { Pool } from 'mysql2/promise';

// DDL idempotente (CREATE TABLE IF NOT EXISTS) para MySQL/InnoDB. Para producción
// seria es preferible drizzle-kit migrations, pero esto basta para instalación
// fresca y arranque. Orden = dependencias de FK (tablas referidas primero).
// Las columnas de fecha son DATETIME(3) (ms) y el pool va en UTC.
const STATEMENTS: string[] = [
  `CREATE TABLE IF NOT EXISTS users (
    id VARCHAR(40) PRIMARY KEY,
    email VARCHAR(255) NOT NULL UNIQUE,
    password_hash VARCHAR(255) NOT NULL,
    name VARCHAR(255) NOT NULL,
    created_at DATETIME(3) NOT NULL
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,

  `CREATE TABLE IF NOT EXISTS \`groups\` (
    id VARCHAR(40) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    created_at DATETIME(3) NOT NULL
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,

  `CREATE TABLE IF NOT EXISTS group_members (
    group_id VARCHAR(40) NOT NULL,
    user_id  VARCHAR(40) NOT NULL,
    PRIMARY KEY (group_id, user_id),
    FOREIGN KEY (group_id) REFERENCES \`groups\`(id) ON DELETE CASCADE,
    FOREIGN KEY (user_id)  REFERENCES users(id)  ON DELETE CASCADE
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,

  `CREATE TABLE IF NOT EXISTS devices (
    id VARCHAR(40) PRIMARY KEY,
    access_key VARCHAR(16) NOT NULL UNIQUE,
    secret_hash VARCHAR(255) NOT NULL,
    name VARCHAR(255) NOT NULL,
    owner_id VARCHAR(40),
    machine_id VARCHAR(80),
    os VARCHAR(64),
    hostname VARCHAR(255),
    agent_version VARCHAR(32),
    require_confirm TINYINT(1) NOT NULL DEFAULT 0,
    last_seen_at DATETIME(3),
    created_at DATETIME(3) NOT NULL,
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE SET NULL
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,

  // Idempotente para bases ya creadas (se ignora si la columna/índice existen).
  `ALTER TABLE devices ADD COLUMN machine_id VARCHAR(80)`,
  `ALTER TABLE devices ADD COLUMN agent_version VARCHAR(32)`,
  `ALTER TABLE devices ADD COLUMN require_confirm TINYINT(1) NOT NULL DEFAULT 0`,
  `CREATE INDEX devices_machine_idx ON devices(machine_id)`,

  `CREATE TABLE IF NOT EXISTS device_access (
    device_id VARCHAR(40) NOT NULL,
    principal_id VARCHAR(40) NOT NULL,
    created_at DATETIME(3) NOT NULL,
    PRIMARY KEY (device_id, principal_id),
    FOREIGN KEY (device_id) REFERENCES devices(id) ON DELETE CASCADE
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,

  `CREATE INDEX device_access_principal_idx ON device_access(principal_id)`,

  `CREATE TABLE IF NOT EXISTS sessions (
    token VARCHAR(64) PRIMARY KEY,
    user_id VARCHAR(40) NOT NULL,
    expires_at DATETIME(3) NOT NULL,
    created_at DATETIME(3) NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,

  `CREATE INDEX sessions_user_idx ON sessions(user_id)`,

  // ---- Tablas heredadas (MSP/chat) en desuso pero aún referidas por código ----

  `CREATE TABLE IF NOT EXISTS projects (
    id VARCHAR(40) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    owner_id VARCHAR(40) NOT NULL,
    created_at DATETIME(3) NOT NULL,
    FOREIGN KEY (owner_id) REFERENCES users(id)
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,

  `CREATE TABLE IF NOT EXISTS project_members (
    project_id   VARCHAR(40) NOT NULL,
    principal_id VARCHAR(40) NOT NULL,
    role         VARCHAR(16) NOT NULL DEFAULT 'usuario',
    PRIMARY KEY (project_id, principal_id),
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,

  `CREATE INDEX project_members_principal_idx ON project_members(principal_id)`,

  `CREATE TABLE IF NOT EXISTS equipos (
    id VARCHAR(40) PRIMARY KEY,
    project_id VARCHAR(40) NOT NULL,
    name VARCHAR(255) NOT NULL,
    os VARCHAR(64),
    hostname VARCHAR(255),
    agent_secret_hash VARCHAR(255) NOT NULL,
    pin_hash VARCHAR(255),
    pin_mode VARCHAR(16) NOT NULL DEFAULT 'required',
    current_user_id VARCHAR(40),
    last_seen_at DATETIME(3),
    created_at DATETIME(3) NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,

  `CREATE INDEX equipos_project_idx ON equipos(project_id)`,

  `CREATE TABLE IF NOT EXISTS channels (
    id VARCHAR(40) PRIMARY KEY,
    empresa_id VARCHAR(40) NOT NULL,
    name VARCHAR(255) NOT NULL,
    kind VARCHAR(16) NOT NULL DEFAULT 'channel',
    created_at DATETIME(3) NOT NULL,
    FOREIGN KEY (empresa_id) REFERENCES projects(id) ON DELETE CASCADE
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,

  `CREATE INDEX channels_empresa_idx ON channels(empresa_id)`,

  `CREATE TABLE IF NOT EXISTS channel_members (
    channel_id VARCHAR(40) NOT NULL,
    member_id  VARCHAR(40) NOT NULL,
    created_at DATETIME(3) NOT NULL,
    PRIMARY KEY (channel_id, member_id),
    FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,

  `CREATE INDEX channel_members_member_idx ON channel_members(member_id)`,

  `CREATE TABLE IF NOT EXISTS messages (
    id VARCHAR(40) PRIMARY KEY,
    channel_id VARCHAR(40) NOT NULL,
    sender_id VARCHAR(40) NOT NULL,
    sender_kind VARCHAR(16) NOT NULL,
    body TEXT NOT NULL,
    attachment_name VARCHAR(255),
    attachment_size INT,
    attachment_mime VARCHAR(255),
    created_at DATETIME(3) NOT NULL,
    FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,

  `CREATE INDEX messages_channel_idx ON messages(channel_id, created_at)`,

  `CREATE TABLE IF NOT EXISTS enrollment_tokens (
    token VARCHAR(32) PRIMARY KEY,
    project_id VARCHAR(40) NOT NULL,
    created_by VARCHAR(40) NOT NULL,
    expires_at DATETIME(3) NOT NULL,
    used_at DATETIME(3),
    used_by_equipo VARCHAR(40),
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE,
    FOREIGN KEY (created_by) REFERENCES users(id)
  ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,
];

// MySQL no soporta "CREATE INDEX IF NOT EXISTS" ni "ADD COLUMN IF NOT EXISTS" de
// forma portable; tratamos como idempotentes ignorando "ya existe / duplicado".
function isAlreadyExists(err: unknown): boolean {
  const s = String((err as { message?: string })?.message ?? err);
  return /Duplicate key name|already exists|Duplicate column name|errno: 1061|errno: 1060|errno: 1050/i.test(s);
}

export async function applyMigrations(pool: Pool): Promise<void> {
  for (const sql of STATEMENTS) {
    try {
      await pool.query(sql);
    } catch (err) {
      if (!isAlreadyExists(err)) throw err;
    }
  }
}
