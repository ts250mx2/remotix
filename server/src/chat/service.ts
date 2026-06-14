import { and, desc, eq, inArray, lt } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { newId } from '../ids.js';

export type SenderKind = 'user' | 'pc' | 'system';
export type ChannelKind = 'channel' | 'dm' | 'support';

export interface Attachment {
  name: string;
  size: number;
  mime: string;
}

export interface MessageDTO {
  id: string;
  channelId: string;
  senderId: string;
  senderKind: SenderKind;
  body: string;
  attachment?: Attachment;
  createdAt: number;
}

export interface ChannelDTO {
  id: string;
  empresaId: string;
  name: string;
  kind: ChannelKind;
  createdAt: number;
}

/** IDs de empresa (proyecto) a las que pertenece un principal (usuario o PC). */
export async function empresaIdsForPrincipal(kind: 'user' | 'pc', id: string): Promise<string[]> {
  if (kind === 'pc') {
    const rows = await db.select({ projectId: tables.equipos.projectId })
      .from(tables.equipos).where(eq(tables.equipos.id, id));
    return rows.map((r) => r.projectId);
  }
  // Usuario: proyectos donde es owner o miembro (directo).
  const owned = await db.select({ id: tables.projects.id })
    .from(tables.projects).where(eq(tables.projects.ownerId, id));
  const memberships = await db.select({ projectId: tables.projectMembers.projectId })
    .from(tables.projectMembers).where(eq(tables.projectMembers.principalId, id));
  return Array.from(new Set([...owned.map((r) => r.id), ...memberships.map((r) => r.projectId)]));
}

export async function isChannelMember(channelId: string, principalId: string): Promise<boolean> {
  const rows = await db.select().from(tables.channelMembers)
    .where(and(eq(tables.channelMembers.channelId, channelId), eq(tables.channelMembers.memberId, principalId)));
  return rows.length > 0;
}

export async function channelMemberIds(channelId: string): Promise<string[]> {
  const rows = await db.select({ memberId: tables.channelMembers.memberId })
    .from(tables.channelMembers).where(eq(tables.channelMembers.channelId, channelId));
  return rows.map((r) => r.memberId);
}

export async function getChannel(channelId: string): Promise<ChannelDTO | null> {
  const row = (await db.select().from(tables.channels).where(eq(tables.channels.id, channelId)))[0];
  if (!row) return null;
  return { id: row.id, empresaId: row.empresaId, name: row.name, kind: row.kind, createdAt: row.createdAt.getTime() };
}

/** Canales de una empresa de los que el principal es miembro. */
export async function channelsForPrincipal(empresaId: string, principalId: string): Promise<ChannelDTO[]> {
  const rows = await db.select({
    id: tables.channels.id,
    empresaId: tables.channels.empresaId,
    name: tables.channels.name,
    kind: tables.channels.kind,
    createdAt: tables.channels.createdAt,
  })
    .from(tables.channels)
    .innerJoin(tables.channelMembers, eq(tables.channelMembers.channelId, tables.channels.id))
    .where(and(eq(tables.channels.empresaId, empresaId), eq(tables.channelMembers.memberId, principalId)));
  return rows.map((r) => ({ ...r, createdAt: r.createdAt.getTime() }));
}

/** Canales (de varias empresas) de los que el principal es miembro. */
export async function channelsForPrincipalIn(empresaIds: string[], principalId: string): Promise<ChannelDTO[]> {
  const out: ChannelDTO[] = [];
  for (const e of empresaIds) out.push(...(await channelsForPrincipal(e, principalId)));
  return out;
}

/** Garantiza un canal 'general' con todos los miembros de la empresa. */
export async function ensureGeneralChannel(empresaId: string): Promise<void> {
  const existing = await db.select({ id: tables.channels.id })
    .from(tables.channels).where(eq(tables.channels.empresaId, empresaId)).limit(1);
  if (existing.length > 0) return;
  const members = await empresaMemberIds(empresaId);
  await createChannel(empresaId, 'general', 'channel', members);
}

/** Añade un principal (p. ej. un PC recién unido) a todos los canales públicos. */
export async function addToPublicChannels(empresaId: string, memberId: string): Promise<void> {
  const chans = await db.select({ id: tables.channels.id })
    .from(tables.channels)
    .where(and(eq(tables.channels.empresaId, empresaId), eq(tables.channels.kind, 'channel')));
  for (const ch of chans) await addChannelMembers(ch.id, [memberId]);
}

export async function addChannelMembers(channelId: string, memberIds: string[]): Promise<void> {
  if (memberIds.length === 0) return;
  const now = new Date();
  for (const memberId of memberIds) {
    await db.insert(tables.channelMembers).values({ channelId, memberId, createdAt: now }).onDuplicateKeyUpdate({ set: { memberId } });
  }
}

/** IDs de todos los miembros (usuarios + PCs) de una empresa. */
export async function empresaMemberIds(empresaId: string): Promise<string[]> {
  const proj = (await db.select().from(tables.projects).where(eq(tables.projects.id, empresaId)))[0];
  const users = await db.select({ principalId: tables.projectMembers.principalId })
    .from(tables.projectMembers).where(eq(tables.projectMembers.projectId, empresaId));
  const pcs = await db.select({ id: tables.equipos.id })
    .from(tables.equipos).where(eq(tables.equipos.projectId, empresaId));
  const ids = new Set<string>([...users.map((u) => u.principalId), ...pcs.map((p) => p.id)]);
  if (proj) ids.add(proj.ownerId);
  return Array.from(ids).filter((id) => id.startsWith('us_') || id.startsWith('eq_'));
}

export async function createChannel(
  empresaId: string,
  name: string,
  kind: ChannelKind,
  memberIds: string[],
): Promise<ChannelDTO> {
  const id = newId('ch');
  const createdAt = new Date();
  await db.insert(tables.channels).values({ id, empresaId, name, kind, createdAt });
  await addChannelMembers(id, memberIds);
  return { id, empresaId, name, kind, createdAt: createdAt.getTime() };
}

export async function persistMessage(
  channelId: string,
  senderId: string,
  senderKind: SenderKind,
  body: string,
  attachment?: Attachment,
): Promise<MessageDTO> {
  const id = newId('msg');
  const createdAt = new Date();
  await db.insert(tables.messages).values({
    id, channelId, senderId, senderKind, body, createdAt,
    attachmentName: attachment?.name ?? null,
    attachmentSize: attachment?.size ?? null,
    attachmentMime: attachment?.mime ?? null,
  });
  return { id, channelId, senderId, senderKind, body, attachment, createdAt: createdAt.getTime() };
}

function rowToMessage(r: typeof tables.messages.$inferSelect): MessageDTO {
  return {
    id: r.id, channelId: r.channelId, senderId: r.senderId, senderKind: r.senderKind, body: r.body,
    attachment: r.attachmentName ? { name: r.attachmentName, size: r.attachmentSize ?? 0, mime: r.attachmentMime ?? 'application/octet-stream' } : undefined,
    createdAt: r.createdAt.getTime(),
  };
}

export async function getMessageById(id: string): Promise<MessageDTO | null> {
  const row = (await db.select().from(tables.messages).where(eq(tables.messages.id, id)))[0];
  return row ? rowToMessage(row) : null;
}

export async function getMessages(channelId: string, limit = 50, before?: number): Promise<MessageDTO[]> {
  const conds = [eq(tables.messages.channelId, channelId)];
  if (before) conds.push(lt(tables.messages.createdAt, new Date(before)));
  const rows = await db.select().from(tables.messages)
    .where(and(...conds))
    .orderBy(desc(tables.messages.createdAt))
    .limit(Math.min(limit, 200));
  return rows.map(rowToMessage).reverse();
}

export interface RosterEntry {
  id: string;
  kind: 'user' | 'pc';
  name: string;
  role?: string;       // solo usuarios
  online: boolean;
  currentUserId?: string | null; // solo PCs
}

export async function empresaRoster(empresaId: string, isOnline: (id: string) => boolean): Promise<RosterEntry[]> {
  const out: RosterEntry[] = [];

  const members = await db.select().from(tables.projectMembers)
    .where(eq(tables.projectMembers.projectId, empresaId));
  const userIds = members.map((m) => m.principalId).filter((id) => id.startsWith('us_'));
  const proj = (await db.select().from(tables.projects).where(eq(tables.projects.id, empresaId)))[0];
  if (proj && !userIds.includes(proj.ownerId)) userIds.push(proj.ownerId);

  if (userIds.length > 0) {
    const users = await db.select().from(tables.users).where(inArray(tables.users.id, userIds));
    for (const u of users) {
      const role = proj?.ownerId === u.id ? 'admin' : members.find((m) => m.principalId === u.id)?.role ?? 'usuario';
      out.push({ id: u.id, kind: 'user', name: u.name, role, online: isOnline(u.id) });
    }
  }

  const pcs = await db.select().from(tables.equipos).where(eq(tables.equipos.projectId, empresaId));
  for (const pc of pcs) {
    out.push({ id: pc.id, kind: 'pc', name: pc.name, online: isOnline(pc.id), currentUserId: pc.currentUserId ?? null });
  }
  return out;
}

/** Casa un usuario con un PC (login en el agente). Un usuario solo en un PC. */
export async function bindUserToPc(userId: string, equipoId: string): Promise<void> {
  await db.update(tables.equipos).set({ currentUserId: null }).where(eq(tables.equipos.currentUserId, userId));
  await db.update(tables.equipos).set({ currentUserId: userId }).where(eq(tables.equipos.id, equipoId));
}

export async function unbindPc(equipoId: string): Promise<void> {
  await db.update(tables.equipos).set({ currentUserId: null }).where(eq(tables.equipos.id, equipoId));
}

/** ¿El usuario puede controlar remotamente PCs de esta empresa? (admin o técnico) */
export async function canRemoteControl(userId: string, empresaId: string): Promise<boolean> {
  const proj = (await db.select().from(tables.projects).where(eq(tables.projects.id, empresaId)))[0];
  if (proj?.ownerId === userId) return true;
  const m = (await db.select().from(tables.projectMembers)
    .where(and(eq(tables.projectMembers.projectId, empresaId), eq(tables.projectMembers.principalId, userId))))[0];
  return m?.role === 'admin' || m?.role === 'tecnico' || m?.role === 'operator';
}
