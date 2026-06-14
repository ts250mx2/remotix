import Database from 'better-sqlite3';
import { drizzle } from 'drizzle-orm/better-sqlite3';
import { env } from '../env.js';
import { applyMigrations } from './migrate.js';
import * as schema from './schema.js';

const sqlite = new Database(env.dbPath);
applyMigrations(sqlite);

export const db = drizzle(sqlite, { schema });
export { sqlite };
export * as tables from './schema.js';
