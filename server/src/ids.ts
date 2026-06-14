import { customAlphabet } from 'nanoid';

// Base62: 0-9, a-z, A-Z. Case-sensitive.
const BASE62 = '0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ';

// 22 chars × log2(62) ≈ 131 bits de entropía. Más que suficiente para
// unicidad global sin coordinación (colisión despreciable).
const id22 = customAlphabet(BASE62, 22);

// Pairing code corto para enrollment (humano-legible-ish, 10 chars Base62).
const pairing10 = customAlphabet(BASE62, 10);

// PIN de equipo: 6 dígitos numéricos (estilo TeamViewer/2FA).
const pin6 = customAlphabet('0123456789', 6);

// Session token largo (32 chars Base62).
const sessionToken32 = customAlphabet(BASE62, 32);

// Agent secret largo (40 chars Base62).
const agentSecret40 = customAlphabet(BASE62, 40);

// Clave de acceso fija del Lite desatendido: 9 chars sin ambigüedades (fácil de
// dictar), mostrada agrupada como XXX-XXX-XXX.
const KEY_ALPHABET = '23456789ABCDEFGHJKMNPQRSTUVWXYZ';
const accessKey9 = customAlphabet(KEY_ALPHABET, 9);

export type IdPrefix = 'us' | 'gp' | 'py' | 'eq' | 'ch' | 'msg' | 'dv';

export function newId(prefix: IdPrefix): string {
  return `${prefix}_${id22()}`;
}

export function isId(prefix: IdPrefix, value: unknown): value is string {
  return typeof value === 'string' && value.startsWith(`${prefix}_`) && value.length === prefix.length + 1 + 22;
}

export const newPairingCode = pairing10;
export const newPin = pin6;
export const newSessionToken = sessionToken32;
export const newAgentSecret = agentSecret40;
export const newAccessKey = accessKey9;
