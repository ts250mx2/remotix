export type ApiError = { error: string; message?: string };

export class HttpError extends Error {
  constructor(public status: number, public payload: ApiError) {
    super(payload.error);
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(path, {
    method,
    credentials: 'include',
    headers: body !== undefined ? { 'content-type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  const json = text ? JSON.parse(text) : null;
  if (!res.ok) throw new HttpError(res.status, json ?? { error: 'unknown' });
  return json as T;
}

export const api = {
  get:  <T>(path: string)               => request<T>('GET',    path),
  post: <T>(path: string, body?: unknown) => request<T>('POST',   path, body),
  patch:<T>(path: string, body?: unknown) => request<T>('PATCH',  path, body),
  del:  <T>(path: string)               => request<T>('DELETE', path),
};

// ---- Tipos compartidos ----
export interface User { id: string; email: string; name: string; }
export interface Project {
  id: string; name: string; ownerId: string; createdAt?: number; isOwner?: boolean;
}
export interface Equipo {
  id: string; projectId: string; name: string; os: string | null; hostname: string | null;
  pinMode: 'none' | 'required'; lastSeenAt: number | null; createdAt: number;
}
// PC en el modelo TeamViewer: dueño + accesos + estado online.
export interface Device {
  id: string; accessKey: string; name: string; ownerId: string | null;
  os: string | null; hostname: string | null;
  agentVersion: string | null;
  lastSeenAt: string | null; createdAt: string;
  role: 'owner' | 'granted'; online: boolean;
  /** Comentario personal del usuario sobre esta PC (cada usuario ve el suyo). */
  note: string | null;
}
export interface DeviceGrant {
  principalId: string; kind: 'user' | 'group'; label: string; name?: string;
}
export interface PairingCode {
  pairingCode: string; projectId: string; expiresAt: string;
}
