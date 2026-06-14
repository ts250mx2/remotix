import { sqliteTable, text, integer, primaryKey, index } from 'drizzle-orm/sqlite-core';

export const users = sqliteTable('users', {
  id: text('id').primaryKey(),                  // us_xxx
  email: text('email').notNull().unique(),
  passwordHash: text('password_hash').notNull(),
  name: text('name').notNull(),
  createdAt: integer('created_at', { mode: 'timestamp_ms' }).notNull(),
});

export const groups = sqliteTable('groups', {
  id: text('id').primaryKey(),                  // gp_xxx
  name: text('name').notNull(),
  createdAt: integer('created_at', { mode: 'timestamp_ms' }).notNull(),
});

export const groupMembers = sqliteTable(
  'group_members',
  {
    groupId: text('group_id').notNull().references(() => groups.id, { onDelete: 'cascade' }),
    userId: text('user_id').notNull().references(() => users.id, { onDelete: 'cascade' }),
  },
  (t) => ({ pk: primaryKey({ columns: [t.groupId, t.userId] }) }),
);

export const projects = sqliteTable('projects', {
  id: text('id').primaryKey(),                  // py_xxx
  name: text('name').notNull(),
  ownerId: text('owner_id').notNull().references(() => users.id),
  createdAt: integer('created_at', { mode: 'timestamp_ms' }).notNull(),
});

// principal_id puede ser un us_* o un gp_*
// Roles dentro de una empresa (proyecto):
//   admin   = administra la empresa (MSP)
//   tecnico = personal de soporte del MSP; puede controlar PCs remotamente
//   usuario = empleado del cliente; chatea y pide soporte, NO controla
//   operator= alias heredado (equivale a tecnico)
export const projectMembers = sqliteTable(
  'project_members',
  {
    projectId: text('project_id').notNull().references(() => projects.id, { onDelete: 'cascade' }),
    principalId: text('principal_id').notNull(),
    role: text('role', { enum: ['admin', 'tecnico', 'usuario', 'operator'] }).notNull().default('usuario'),
  },
  (t) => ({
    pk: primaryKey({ columns: [t.projectId, t.principalId] }),
    principalIdx: index('project_members_principal_idx').on(t.principalId),
  }),
);

// ---- Chat tipo Slack (persistente) ----

// Canal de chat dentro de una empresa (proyecto).
export const channels = sqliteTable(
  'channels',
  {
    id: text('id').primaryKey(),                 // ch_xxx
    empresaId: text('empresa_id').notNull().references(() => projects.id, { onDelete: 'cascade' }),
    name: text('name').notNull(),
    kind: text('kind', { enum: ['channel', 'dm', 'support'] }).notNull().default('channel'),
    createdAt: integer('created_at', { mode: 'timestamp_ms' }).notNull(),
  },
  (t) => ({ empresaIdx: index('channels_empresa_idx').on(t.empresaId) }),
);

// Miembro de un canal: puede ser un usuario (us_*) o un PC (eq_*).
export const channelMembers = sqliteTable(
  'channel_members',
  {
    channelId: text('channel_id').notNull().references(() => channels.id, { onDelete: 'cascade' }),
    memberId: text('member_id').notNull(),       // us_* o eq_*
    createdAt: integer('created_at', { mode: 'timestamp_ms' }).notNull(),
  },
  (t) => ({
    pk: primaryKey({ columns: [t.channelId, t.memberId] }),
    memberIdx: index('channel_members_member_idx').on(t.memberId),
  }),
);

// Mensaje persistente.
export const messages = sqliteTable(
  'messages',
  {
    id: text('id').primaryKey(),                 // msg_xxx
    channelId: text('channel_id').notNull().references(() => channels.id, { onDelete: 'cascade' }),
    senderId: text('sender_id').notNull(),       // us_*, eq_* o 'system'
    senderKind: text('sender_kind', { enum: ['user', 'pc', 'system'] }).notNull(),
    body: text('body').notNull(),
    // Adjunto opcional (archivo guardado en disco como uploads/<id>).
    attachmentName: text('attachment_name'),
    attachmentSize: integer('attachment_size'),
    attachmentMime: text('attachment_mime'),
    createdAt: integer('created_at', { mode: 'timestamp_ms' }).notNull(),
  },
  (t) => ({ channelIdx: index('messages_channel_idx').on(t.channelId, t.createdAt) }),
);

export const equipos = sqliteTable(
  'equipos',
  {
    id: text('id').primaryKey(),                // eq_xxx
    projectId: text('project_id').notNull().references(() => projects.id, { onDelete: 'cascade' }),
    name: text('name').notNull(),
    os: text('os'),                             // 'windows', 'linux', etc.
    hostname: text('hostname'),
    agentSecretHash: text('agent_secret_hash').notNull(),
    pinHash: text('pin_hash'),                  // null si pinMode='none'
    pinMode: text('pin_mode', { enum: ['none', 'required'] }).notNull().default('required'),
    // Usuario actualmente "casado" con este PC (sesión iniciada en el agente).
    // Un usuario solo puede estar en un PC a la vez (se limpia al moverse).
    currentUserId: text('current_user_id'),
    lastSeenAt: integer('last_seen_at', { mode: 'timestamp_ms' }),
    createdAt: integer('created_at', { mode: 'timestamp_ms' }).notNull(),
  },
  (t) => ({ projectIdx: index('equipos_project_idx').on(t.projectId) }),
);

// Dispositivos "desatendidos" estilo TeamViewer: clave fija permanente, sin
// proyecto. El usuario instala el Lite, obtiene una clave que NO cambia, y un
// técnico se conecta por esa clave cuando el equipo está en línea.
export const devices = sqliteTable('devices', {
  id: text('id').primaryKey(),                  // dv_xxx
  accessKey: text('access_key').notNull().unique(), // clave fija (9 chars)
  secretHash: text('secret_hash').notNull(),
  name: text('name').notNull(),
  lastSeenAt: integer('last_seen_at', { mode: 'timestamp_ms' }),
  createdAt: integer('created_at', { mode: 'timestamp_ms' }).notNull(),
});

export const enrollmentTokens = sqliteTable('enrollment_tokens', {
  token: text('token').primaryKey(),            // ~10 chars Base62
  projectId: text('project_id').notNull().references(() => projects.id, { onDelete: 'cascade' }),
  createdBy: text('created_by').notNull().references(() => users.id),
  expiresAt: integer('expires_at', { mode: 'timestamp_ms' }).notNull(),
  usedAt: integer('used_at', { mode: 'timestamp_ms' }),
  usedByEquipo: text('used_by_equipo'),
});

export const sessions = sqliteTable(
  'sessions',
  {
    token: text('token').primaryKey(),
    userId: text('user_id').notNull().references(() => users.id, { onDelete: 'cascade' }),
    expiresAt: integer('expires_at', { mode: 'timestamp_ms' }).notNull(),
    createdAt: integer('created_at', { mode: 'timestamp_ms' }).notNull(),
  },
  (t) => ({ userIdx: index('sessions_user_idx').on(t.userId) }),
);
