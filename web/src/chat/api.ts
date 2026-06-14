import { api } from '../api/client';

export interface ChatChannel {
  id: string;
  empresaId: string;
  name: string;
  kind: 'channel' | 'dm' | 'support';
  createdAt: number;
}

export interface Attachment {
  name: string;
  size: number;
  mime: string;
}

export interface ChatMessage {
  id: string;
  channelId: string;
  senderId: string;
  senderKind: 'user' | 'pc' | 'system';
  body: string;
  attachment?: Attachment;
  createdAt: number;
}

export interface RosterEntry {
  id: string;
  kind: 'user' | 'pc';
  name: string;
  role?: string;
  online: boolean;
  currentUserId?: string | null;
}

export const chatApi = {
  channels: (empresaId: string) =>
    api.get<{ channels: ChatChannel[] }>(`/api/chat/channels?empresa_id=${empresaId}`).then((r) => r.channels),
  createChannel: (empresaId: string, name: string, kind: ChatChannel['kind'] = 'channel') =>
    api.post<{ channel: ChatChannel }>(`/api/chat/channels`, { empresaId, name, kind }).then((r) => r.channel),
  messages: (channelId: string, before?: number) =>
    api
      .get<{ messages: ChatMessage[] }>(`/api/chat/channels/${channelId}/messages${before ? `?before=${before}` : ''}`)
      .then((r) => r.messages),
  post: (channelId: string, body: string) =>
    api.post<{ message: ChatMessage }>(`/api/chat/channels/${channelId}/messages`, { body }).then((r) => r.message),
  roster: (empresaId: string) =>
    api.get<{ roster: RosterEntry[] }>(`/api/chat/empresas/${empresaId}/roster`).then((r) => r.roster),
  uploadFile: async (channelId: string, file: File, body = '') => {
    const fd = new FormData();
    fd.append('file', file);
    if (body) fd.append('body', body);
    const res = await fetch(`/api/chat/channels/${channelId}/files`, { method: 'POST', credentials: 'include', body: fd });
    if (!res.ok) throw new Error('upload_failed');
    return (await res.json()).message as ChatMessage;
  },
  fileUrl: (messageId: string) => `/api/chat/files/${messageId}`,
};
