import { useEffect, useRef, useState, type FormEvent } from 'react';
import type { ChatMessage } from './connection';

interface ChatPanelProps {
  messages: ChatMessage[];
  onSend: (text: string) => void;
  disabled?: boolean;
  placeholder?: string;
}

export function ChatPanel({ messages, onSend, disabled, placeholder }: ChatPanelProps) {
  const [text, setText] = useState('');
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages.length]);

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!text.trim() || disabled) return;
    onSend(text);
    setText('');
  }

  return (
    <div className="chat">
      <div className="chat-log">
        {messages.length === 0 ? (
          <p className="muted small chat-empty">Aún no hay mensajes.</p>
        ) : (
          messages.map((m, i) => (
            <div key={i} className={`chat-msg chat-${m.from}`}>
              {m.from === 'system' ? (
                <span className="chat-system">{m.text}</span>
              ) : (
                <span className="chat-bubble">{m.text}</span>
              )}
            </div>
          ))
        )}
        <div ref={endRef} />
      </div>
      <form className="chat-input" onSubmit={submit}>
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={placeholder ?? 'Escribe un mensaje…'}
          disabled={disabled}
        />
        <button type="submit" disabled={disabled || !text.trim()}>
          Enviar
        </button>
      </form>
    </div>
  );
}
