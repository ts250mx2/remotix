import type Database from 'better-sqlite3';

// SQL "Create table if not exists" idempotente. Para producción seria
// preferible drizzle-kit migrations, pero esto basta para dev / instalación
// fresca.
const DDL = `
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;

CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  name TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS groups (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS group_members (
  group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
  user_id  TEXT NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
  PRIMARY KEY (group_id, user_id)
);

CREATE TABLE IF NOT EXISTS projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  owner_id TEXT NOT NULL REFERENCES users(id),
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS project_members (
  project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  principal_id TEXT NOT NULL,
  role         TEXT NOT NULL DEFAULT 'operator',
  PRIMARY KEY (project_id, principal_id)
);
CREATE INDEX IF NOT EXISTS project_members_principal_idx ON project_members(principal_id);

CREATE TABLE IF NOT EXISTS equipos (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  os TEXT,
  hostname TEXT,
  agent_secret_hash TEXT NOT NULL,
  pin_hash TEXT,
  pin_mode TEXT NOT NULL DEFAULT 'required',
  current_user_id TEXT,
  last_seen_at INTEGER,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS equipos_project_idx ON equipos(project_id);

CREATE TABLE IF NOT EXISTS channels (
  id TEXT PRIMARY KEY,
  empresa_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'channel',
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS channels_empresa_idx ON channels(empresa_id);

CREATE TABLE IF NOT EXISTS channel_members (
  channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  member_id  TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (channel_id, member_id)
);
CREATE INDEX IF NOT EXISTS channel_members_member_idx ON channel_members(member_id);

CREATE TABLE IF NOT EXISTS messages (
  id TEXT PRIMARY KEY,
  channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  sender_id TEXT NOT NULL,
  sender_kind TEXT NOT NULL,
  body TEXT NOT NULL,
  attachment_name TEXT,
  attachment_size INTEGER,
  attachment_mime TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_channel_idx ON messages(channel_id, created_at);

CREATE TABLE IF NOT EXISTS devices (
  id TEXT PRIMARY KEY,
  access_key TEXT NOT NULL UNIQUE,
  secret_hash TEXT NOT NULL,
  name TEXT NOT NULL,
  last_seen_at INTEGER,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS enrollment_tokens (
  token TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  created_by TEXT NOT NULL REFERENCES users(id),
  expires_at INTEGER NOT NULL,
  used_at INTEGER,
  used_by_equipo TEXT
);

CREATE TABLE IF NOT EXISTS sessions (
  token TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_user_idx ON sessions(user_id);
`;

// ALTERs idempotentes para columnas añadidas a tablas ya existentes.
const ALTERS = [
  `ALTER TABLE equipos ADD COLUMN current_user_id TEXT`,
  `ALTER TABLE messages ADD COLUMN attachment_name TEXT`,
  `ALTER TABLE messages ADD COLUMN attachment_size INTEGER`,
  `ALTER TABLE messages ADD COLUMN attachment_mime TEXT`,
];

export function applyMigrations(db: Database.Database): void {
  db.exec(DDL);
  for (const sql of ALTERS) {
    try {
      db.exec(sql);
    } catch (err) {
      // "duplicate column name" → la columna ya existe; ignorar.
      if (!String(err).includes('duplicate column')) throw err;
    }
  }
}
