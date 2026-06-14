import mysql from 'mysql2/promise';
import { drizzle } from 'drizzle-orm/mysql2';
import { env } from '../env.js';
import * as schema from './schema.js';

// Pool MySQL. timezone 'Z' = UTC: las columnas DATETIME(3) se leen/escriben en
// UTC sin conversión de zona, así los `Date` que viajan por Drizzle son exactos.
export const pool = mysql.createPool({
  host: env.mysql.host,
  port: env.mysql.port,
  user: env.mysql.user,
  password: env.mysql.password,
  database: env.mysql.database,
  timezone: 'Z',
  charset: 'utf8mb4',
  connectionLimit: env.mysql.connectionLimit,
  enableKeepAlive: true,
});

export const db = drizzle(pool, { schema, mode: 'default' });
export { schema };
export * as tables from './schema.js';
