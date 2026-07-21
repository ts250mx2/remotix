import { mysqlTable, varchar, text, int, datetime, index, primaryKey, boolean } from 'drizzle-orm/mysql-core';

// Timestamp con precisión de milisegundos que Drizzle mapea a `Date` en ambos
// sentidos (insert acepta Date, select devuelve Date), igual que hacía el modo
// `timestamp_ms` de SQLite. El pool mysql2 va en timezone 'Z' (UTC) para que no
// haya conversión de zona horaria. Mantener el código que hace `.getTime()` /
// `new Date()` sin cambios.
const ts = (name: string) => datetime(name, { mode: 'date', fsp: 3 });

export const users = mysqlTable('users', {
  id: varchar('id', { length: 40 }).primaryKey(),          // us_xxx
  email: varchar('email', { length: 255 }).notNull().unique(),
  passwordHash: varchar('password_hash', { length: 255 }).notNull(),
  name: varchar('name', { length: 255 }).notNull(),
  createdAt: ts('created_at').notNull(),
});

export const groups = mysqlTable('groups', {
  id: varchar('id', { length: 40 }).primaryKey(),          // gp_xxx
  name: varchar('name', { length: 255 }).notNull(),
  createdAt: ts('created_at').notNull(),
});

export const groupMembers = mysqlTable(
  'group_members',
  {
    groupId: varchar('group_id', { length: 40 }).notNull().references(() => groups.id, { onDelete: 'cascade' }),
    userId: varchar('user_id', { length: 40 }).notNull().references(() => users.id, { onDelete: 'cascade' }),
  },
  (t) => ({ pk: primaryKey({ columns: [t.groupId, t.userId] }) }),
);

// Dispositivos (PCs) estilo TeamViewer: clave fija permanente. Cada device tiene
// un dueño (usuario que lo reclamó) y se conceden accesos a otros usuarios/grupos
// vía `deviceAccess`. `ownerId` es null hasta que alguien lo reclama.
export const devices = mysqlTable(
  'devices',
  {
    id: varchar('id', { length: 40 }).primaryKey(),          // dv_xxx
    accessKey: varchar('access_key', { length: 16 }).notNull().unique(), // clave fija (9 chars)
    secretHash: varchar('secret_hash', { length: 255 }).notNull(),
    name: varchar('name', { length: 255 }).notNull(),
    ownerId: varchar('owner_id', { length: 40 }),            // us_*; null = sin reclamar
    // Identificador estable de la máquina (Windows MachineGuid): evita duplicar el
    // equipo al reinstalar — se reutiliza el mismo device si ya existe.
    machineId: varchar('machine_id', { length: 80 }),
    os: varchar('os', { length: 64 }),
    hostname: varchar('hostname', { length: 255 }),
    // Versión del agente instalada, reportada en el `hello` del WS. Permite ver
    // qué PC tiene qué versión y disparar la auto-actualización.
    agentVersion: varchar('agent_version', { length: 32 }),
    // Si el usuario del equipo debe aprobar cada conexión entrante (toggle en el
    // Lite). false = desatendido puro (por defecto, estilo TeamViewer).
    requireConfirm: boolean('require_confirm').notNull().default(false),
    lastSeenAt: ts('last_seen_at'),
    createdAt: ts('created_at').notNull(),
  },
  (t) => ({ machineIdx: index('devices_machine_idx').on(t.machineId) }),
);

// Acceso de un principal (us_* o gp_*) a un device. Mismo patrón polimórfico que
// project_members. Un usuario puede conectarse al device si es dueño, tiene una
// fila aquí, o pertenece a un grupo que la tiene.
export const deviceAccess = mysqlTable(
  'device_access',
  {
    deviceId: varchar('device_id', { length: 40 }).notNull().references(() => devices.id, { onDelete: 'cascade' }),
    principalId: varchar('principal_id', { length: 40 }).notNull(),   // us_* o gp_*
    createdAt: ts('created_at').notNull(),
  },
  (t) => ({
    pk: primaryKey({ columns: [t.deviceId, t.principalId] }),
    principalIdx: index('device_access_principal_idx').on(t.principalId),
  }),
);

export const sessions = mysqlTable(
  'sessions',
  {
    token: varchar('token', { length: 64 }).primaryKey(),
    userId: varchar('user_id', { length: 40 }).notNull().references(() => users.id, { onDelete: 'cascade' }),
    expiresAt: ts('expires_at').notNull(),
    createdAt: ts('created_at').notNull(),
  },
  (t) => ({ userIdx: index('sessions_user_idx').on(t.userId) }),
);

// ---------------------------------------------------------------------------
// Tablas heredadas del modelo MSP/chat (empresas + canales). Se conservan para
// no romper compilación durante el pivote; quedan en desuso y se retirarán.
// ---------------------------------------------------------------------------

export const projects = mysqlTable('projects', {
  id: varchar('id', { length: 40 }).primaryKey(),          // py_xxx
  name: varchar('name', { length: 255 }).notNull(),
  ownerId: varchar('owner_id', { length: 40 }).notNull().references(() => users.id),
  createdAt: ts('created_at').notNull(),
});

export const projectMembers = mysqlTable(
  'project_members',
  {
    projectId: varchar('project_id', { length: 40 }).notNull().references(() => projects.id, { onDelete: 'cascade' }),
    principalId: varchar('principal_id', { length: 40 }).notNull(),
    role: varchar('role', { length: 16, enum: ['admin', 'tecnico', 'usuario', 'operator'] }).notNull().default('usuario'),
  },
  (t) => ({
    pk: primaryKey({ columns: [t.projectId, t.principalId] }),
    principalIdx: index('project_members_principal_idx').on(t.principalId),
  }),
);

export const channels = mysqlTable(
  'channels',
  {
    id: varchar('id', { length: 40 }).primaryKey(),        // ch_xxx
    empresaId: varchar('empresa_id', { length: 40 }).notNull().references(() => projects.id, { onDelete: 'cascade' }),
    name: varchar('name', { length: 255 }).notNull(),
    kind: varchar('kind', { length: 16, enum: ['channel', 'dm', 'support'] }).notNull().default('channel'),
    createdAt: ts('created_at').notNull(),
  },
  (t) => ({ empresaIdx: index('channels_empresa_idx').on(t.empresaId) }),
);

export const channelMembers = mysqlTable(
  'channel_members',
  {
    channelId: varchar('channel_id', { length: 40 }).notNull().references(() => channels.id, { onDelete: 'cascade' }),
    memberId: varchar('member_id', { length: 40 }).notNull(),     // us_* o eq_*
    createdAt: ts('created_at').notNull(),
  },
  (t) => ({
    pk: primaryKey({ columns: [t.channelId, t.memberId] }),
    memberIdx: index('channel_members_member_idx').on(t.memberId),
  }),
);

export const messages = mysqlTable(
  'messages',
  {
    id: varchar('id', { length: 40 }).primaryKey(),        // msg_xxx
    channelId: varchar('channel_id', { length: 40 }).notNull().references(() => channels.id, { onDelete: 'cascade' }),
    senderId: varchar('sender_id', { length: 40 }).notNull(),     // us_*, eq_* o 'system'
    senderKind: varchar('sender_kind', { length: 16, enum: ['user', 'pc', 'system'] }).notNull(),
    body: text('body').notNull(),
    attachmentName: varchar('attachment_name', { length: 255 }),
    attachmentSize: int('attachment_size'),
    attachmentMime: varchar('attachment_mime', { length: 255 }),
    createdAt: ts('created_at').notNull(),
  },
  (t) => ({ channelIdx: index('messages_channel_idx').on(t.channelId, t.createdAt) }),
);

export const equipos = mysqlTable(
  'equipos',
  {
    id: varchar('id', { length: 40 }).primaryKey(),        // eq_xxx
    projectId: varchar('project_id', { length: 40 }).notNull().references(() => projects.id, { onDelete: 'cascade' }),
    name: varchar('name', { length: 255 }).notNull(),
    os: varchar('os', { length: 64 }),
    hostname: varchar('hostname', { length: 255 }),
    agentSecretHash: varchar('agent_secret_hash', { length: 255 }).notNull(),
    pinHash: varchar('pin_hash', { length: 255 }),
    pinMode: varchar('pin_mode', { length: 16, enum: ['none', 'required'] }).notNull().default('required'),
    currentUserId: varchar('current_user_id', { length: 40 }),
    lastSeenAt: ts('last_seen_at'),
    createdAt: ts('created_at').notNull(),
  },
  (t) => ({ projectIdx: index('equipos_project_idx').on(t.projectId) }),
);

export const enrollmentTokens = mysqlTable('enrollment_tokens', {
  token: varchar('token', { length: 32 }).primaryKey(),
  projectId: varchar('project_id', { length: 40 }).notNull().references(() => projects.id, { onDelete: 'cascade' }),
  createdBy: varchar('created_by', { length: 40 }).notNull().references(() => users.id),
  expiresAt: ts('expires_at').notNull(),
  usedAt: ts('used_at'),
  usedByEquipo: varchar('used_by_equipo', { length: 40 }),
});
