import { useEffect, useRef, useState } from 'react';
import type { ChatMessage } from './api';

interface ChatHandlers {
  onMessage?: (m: ChatMessage) => void;
  onPresence?: (id: string, online: boolean) => void;
  onReady?: () => void;
  onCall?: (m: Record<string, unknown>) => void; // mensajes call-*
}

function chatWsUrl(): string {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${proto}//${location.host}/ws/chat`;
}

/** Conexión WebSocket persistente al chat (auth por cookie de sesión). */
export function useChat(handlers: ChatHandlers) {
  const [ready, setReady] = useState(false);
  const [presence, setPresence] = useState<Record<string, boolean>>({});
  const wsRef = useRef<WebSocket | null>(null);
  const h = useRef(handlers);
  h.current = handlers;

  useEffect(() => {
    let retry: ReturnType<typeof setTimeout> | undefined;
    let closed = false;

    function connect() {
      const ws = new WebSocket(chatWsUrl());
      wsRef.current = ws;
      ws.onmessage = (e) => {
        let m: Record<string, unknown>;
        try {
          m = JSON.parse(e.data);
        } catch {
          return;
        }
        if (m.type === 'ready') {
          setReady(true);
          h.current.onReady?.();
        } else if (m.type === 'message') {
          h.current.onMessage?.(m.message as ChatMessage);
        } else if (m.type === 'presence') {
          const id = String(m.id);
          const online = !!m.online;
          setPresence((p) => ({ ...p, [id]: online }));
          h.current.onPresence?.(id, online);
        } else if (typeof m.type === 'string' && m.type.startsWith('call-')) {
          h.current.onCall?.(m);
        }
      };
      ws.onclose = () => {
        setReady(false);
        if (!closed) retry = setTimeout(connect, 1500); // reconexión
      };
      ws.onerror = () => ws.close();
    }
    connect();

    return () => {
      closed = true;
      if (retry) clearTimeout(retry);
      wsRef.current?.close();
    };
  }, []);

  function sendMessage(channelId: string, body: string) {
    sendRaw({ type: 'message', channelId, body });
  }

  function sendRaw(obj: unknown) {
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(obj));
  }

  return { ready, presence, sendMessage, sendRaw };
}
