import { eq, lt } from 'drizzle-orm';
import { db, tables } from '../db/index.js';
import { newSessionToken } from '../ids.js';

const SESSION_TTL_MS = 1000 * 60 * 60 * 24 * 14; // 14 días

export interface SessionRecord {
  token: string;
  userId: string;
  expiresAt: Date;
}

export async function createSession(userId: string): Promise<SessionRecord> {
  const token = newSessionToken();
  const now = new Date();
  const expiresAt = new Date(now.getTime() + SESSION_TTL_MS);
  await db.insert(tables.sessions).values({
    token,
    userId,
    expiresAt,
    createdAt: now,
  });
  return { token, userId, expiresAt };
}

export async function getSession(token: string): Promise<SessionRecord | null> {
  const rows = await db.select().from(tables.sessions).where(eq(tables.sessions.token, token));
  const row = rows[0];
  if (!row) return null;
  if (row.expiresAt.getTime() < Date.now()) {
    await db.delete(tables.sessions).where(eq(tables.sessions.token, token));
    return null;
  }
  return { token: row.token, userId: row.userId, expiresAt: row.expiresAt };
}

export async function deleteSession(token: string): Promise<void> {
  await db.delete(tables.sessions).where(eq(tables.sessions.token, token));
}

export async function purgeExpired(): Promise<void> {
  await db.delete(tables.sessions).where(lt(tables.sessions.expiresAt, new Date()));
}

export const COOKIE_NAME = 'remotix_session';
