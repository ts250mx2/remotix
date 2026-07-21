import { and, eq } from 'drizzle-orm';
import { db, tables } from './db/index.js';
import { deviceHub } from './devices/hub.js';

// Rol de un usuario sobre un device:
//   'owner'   = es el dueño (lo reclamó)
//   'granted' = tiene acceso por grant directo o por pertenecer a un grupo con grant
//   null      = sin acceso
export type DeviceRole = 'owner' | 'granted' | null;

export async function userCanAccessDevice(userId: string, deviceId: string): Promise<DeviceRole> {
  const dev = (await db.select().from(tables.devices).where(eq(tables.devices.id, deviceId)))[0];
  if (!dev) return null;
  if (dev.ownerId === userId) return 'owner';

  // Grant directo al usuario (principal_id = us_*).
  const direct = await db.select({ d: tables.deviceAccess.deviceId })
    .from(tables.deviceAccess)
    .where(and(eq(tables.deviceAccess.deviceId, deviceId), eq(tables.deviceAccess.principalId, userId)));
  if (direct.length) return 'granted';

  // Grant a un grupo del que el usuario es miembro (principal_id = gp_*).
  const viaGroup = await db.select({ d: tables.deviceAccess.deviceId })
    .from(tables.deviceAccess)
    .innerJoin(tables.groupMembers, eq(tables.groupMembers.groupId, tables.deviceAccess.principalId))
    .where(and(eq(tables.deviceAccess.deviceId, deviceId), eq(tables.groupMembers.userId, userId)));
  if (viaGroup.length) return 'granted';

  return null;
}

export interface AccessibleDevice {
  id: string;
  accessKey: string;
  name: string;
  ownerId: string | null;
  os: string | null;
  hostname: string | null;
  agentVersion: string | null;
  lastSeenAt: Date | null;
  createdAt: Date;
  role: 'owner' | 'granted';
  online: boolean;
  /// Comentario personal del usuario sobre esta PC (null = sin comentario).
  note: string | null;
}

// Devices visibles para un usuario: los que posee + los que le concedieron
// (directo o vía grupo). Incluye estado online del device hub.
export async function listAccessibleDevices(userId: string): Promise<AccessibleDevice[]> {
  const directIds = (await db.select({ id: tables.deviceAccess.deviceId })
    .from(tables.deviceAccess)
    .where(eq(tables.deviceAccess.principalId, userId))).map((r) => r.id);

  const groupIds = (await db.select({ id: tables.deviceAccess.deviceId })
    .from(tables.deviceAccess)
    .innerJoin(tables.groupMembers, eq(tables.groupMembers.groupId, tables.deviceAccess.principalId))
    .where(eq(tables.groupMembers.userId, userId))).map((r) => r.id);

  const granted = new Set<string>([...directIds, ...groupIds]);

  // Comentarios personales del usuario (uno por device, si lo escribió).
  const noteRows = await db.select({ deviceId: tables.deviceNotes.deviceId, note: tables.deviceNotes.note })
    .from(tables.deviceNotes)
    .where(eq(tables.deviceNotes.userId, userId));
  const notes = new Map(noteRows.map((r) => [r.deviceId, r.note]));

  const all = await db.select().from(tables.devices);
  return all
    .filter((d) => d.ownerId === userId || granted.has(d.id))
    .map((d) => ({
      id: d.id,
      accessKey: d.accessKey,
      name: d.name,
      ownerId: d.ownerId,
      os: d.os,
      hostname: d.hostname,
      agentVersion: d.agentVersion,
      lastSeenAt: d.lastSeenAt,
      createdAt: d.createdAt,
      role: (d.ownerId === userId ? 'owner' : 'granted') as 'owner' | 'granted',
      online: deviceHub.isOnline(d.id),
      note: notes.get(d.id) ?? null,
    }));
}
