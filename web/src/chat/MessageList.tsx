import { forwardRef } from 'react';
import type { ChatMessage } from './api';
import { avatarColor, initials } from './avatar';

interface MessageListProps {
  messages: ChatMessage[];
  selfId: string;
  resolveName: (senderId: string, senderKind: ChatMessage['senderKind']) => string;
  fileUrl?: (messageId: string) => string;
  emptyText?: string;
}

const GROUP_MS = 5 * 60 * 1000;

function dayLabel(ts: number): string {
  const d = new Date(ts);
  const today = new Date();
  const same = d.toDateString() === today.toDateString();
  return same ? 'Hoy' : d.toLocaleDateString();
}

/** Lista de mensajes estilo Slack: avatar con iniciales, agrupación de mensajes
 * consecutivos del mismo emisor, separadores de día y mensajes de sistema. */
export const MessageList = forwardRef<HTMLDivElement, MessageListProps>(function MessageList(
  { messages, selfId, resolveName, fileUrl, emptyText },
  ref,
) {
  return (
    <div className="msg-log" ref={ref}>
      {messages.length === 0 && <p className="chat-empty muted">{emptyText ?? 'No hay mensajes todavía.'}</p>}
      {messages.map((m, i) => {
        const prev = messages[i - 1];
        const newDay = !prev || dayLabel(prev.createdAt) !== dayLabel(m.createdAt);

        if (m.senderKind === 'system') {
          return (
            <div key={m.id}>
              {newDay && <div className="day-sep"><span>{dayLabel(m.createdAt)}</span></div>}
              <div className="msg-system">{m.body}</div>
            </div>
          );
        }

        const mine = m.senderId === selfId;
        const name = mine ? 'Tú' : resolveName(m.senderId, m.senderKind);
        const grouped = !newDay && prev && prev.senderKind !== 'system' &&
          prev.senderId === m.senderId && m.createdAt - prev.createdAt < GROUP_MS;
        const time = new Date(m.createdAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

        return (
          <div key={m.id}>
            {newDay && <div className="day-sep"><span>{dayLabel(m.createdAt)}</span></div>}
            <div className={`m ${grouped ? 'grouped' : ''}`}>
              {grouped ? (
                <span className="m-gutter">{time}</span>
              ) : (
                <div className="m-avatar" style={{ background: avatarColor(m.senderId) }}>
                  {m.senderKind === 'pc' ? '💻' : initials(name)}
                </div>
              )}
              <div className="m-main">
                {!grouped && (
                  <div className="m-head">
                    <span className="m-author">{name}{m.senderKind === 'pc' && ' · PC'}</span>
                    <span className="m-time">{time}</span>
                  </div>
                )}
                {m.body && <div className="m-text">{m.body}</div>}
                {m.attachment && fileUrl && (
                  <a className="msg-attach" href={fileUrl(m.id)} download={m.attachment.name}>
                    📎 {m.attachment.name}
                    <span className="muted small"> ({Math.max(1, Math.round(m.attachment.size / 1024))} KB)</span>
                  </a>
                )}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
});
