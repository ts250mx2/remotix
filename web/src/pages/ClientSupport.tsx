import { useEffect, useRef, useState, type FormEvent } from 'react';
import { SupportConnection, errorText, type ChatMessage } from '../helpdesk/connection';
import { ChatPanel } from '../helpdesk/ChatPanel';
import { FilePanel, useFileTransfers } from '../helpdesk/FilePanel';

type Phase = 'form' | 'live';

export function ClientSupport() {
  const [phase, setPhase] = useState<Phase>('form');
  const [name, setName] = useState('');
  const [issue, setIssue] = useState('');

  const [code, setCode] = useState<string | null>(null);
  const [status, setStatus] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [techPresent, setTechPresent] = useState(false);
  const [sharing, setSharing] = useState(false);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const { filesReady, transfers, received, requested, clearRequested, fileHandlers } = useFileTransfers();

  const connRef = useRef<SupportConnection | null>(null);
  const previewRef = useRef<HTMLVideoElement>(null);

  useEffect(() => {
    return () => connRef.current?.close();
  }, []);

  function pushSystem(text: string) {
    setMessages((m) => [...m, { text, from: 'system', ts: Date.now() }]);
  }

  function startSession(e: FormEvent) {
    e.preventDefault();
    setError(null);
    const conn = new SupportConnection('client', {
      onCode: (c) => setCode(c),
      onStatus: (s) => setStatus(s),
      onPeerJoined: () => {
        setTechPresent(true);
        pushSystem('El técnico se conectó.');
      },
      onPeerLeft: () => {
        setTechPresent(false);
        pushSystem('El técnico se desconectó.');
      },
      onChat: (msg) => setMessages((m) => [...m, msg]),
      onShareEnded: () => {
        setSharing(false);
        pushSystem('Dejaste de compartir la pantalla.');
      },
      onError: (c) => setError(errorText(c)),
      onClosed: () => setStatus('Conexión cerrada.'),
      ...fileHandlers,
    });
    conn.start({ name: name.trim() || undefined, issue: issue.trim() || undefined });
    connRef.current = conn;
    setPhase('live');
  }

  async function share() {
    setError(null);
    try {
      await connRef.current?.startScreenShare();
      setSharing(true);
      const stream = connRef.current?.getLocalStream() ?? null;
      if (previewRef.current) previewRef.current.srcObject = stream;
    } catch {
      setError('No se pudo iniciar la captura de pantalla (¿la cancelaste?).');
    }
  }

  if (phase === 'form') {
    return (
      <main className="centered">
        <form onSubmit={startSession} className="card narrow">
          <h1>Soporte remoto</h1>
          <p className="muted">
            Inicia una sesión y comparte tu pantalla con el técnico. No necesitas instalar nada.
          </p>
          <label>
            Tu nombre
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Ej: María" autoFocus />
          </label>
          <label>
            ¿En qué necesitas ayuda? (opcional)
            <input value={issue} onChange={(e) => setIssue(e.target.value)} placeholder="Describe el problema" />
          </label>
          <button type="submit">Compartir mi pantalla (sin instalar)</button>

          <hr className="sep" />
          <p className="muted small">
            ¿El técnico necesita <strong>controlar</strong> tu equipo (mouse, teclado y archivos)?
            Descarga Remotix, ábrelo y dile la <strong>clave</strong> que aparece. Sin instalación.
          </p>
          <a className="download-btn" href="/download/remotix-lite.exe" download>
            Descargar Remotix · control remoto (Windows)
          </a>
        </form>
      </main>
    );
  }

  return (
    <main className="helpdesk">
      <section className="hd-main">
        <header className="hd-header">
          <h1>Soporte remoto</h1>
          <span className={`hd-dot ${techPresent ? 'on' : ''}`}>{techPresent ? 'Técnico conectado' : 'Esperando técnico'}</span>
        </header>

        {code && (
          <div className="code-card">
            <span className="muted small">Dale este código al técnico:</span>
            <code className="code-big">{code}</code>
          </div>
        )}

        {error && <div className="error">{error}</div>}
        {status && <p className="muted small">{status}</p>}

        <div className="share-box">
          {sharing ? (
            <video ref={previewRef} autoPlay muted playsInline className="preview" />
          ) : (
            <div className="share-cta">
              <p>Comparte tu pantalla para que el técnico pueda verla.</p>
              <button onClick={share}>Compartir mi pantalla</button>
            </div>
          )}
        </div>
        {sharing && (
          <p className="muted small">
            Estás compartiendo tu pantalla. Usa el control del navegador para dejar de compartir cuando termines.
          </p>
        )}
      </section>

      <aside className="hd-side">
        <h2>Chat</h2>
        <ChatPanel
          messages={messages}
          onSend={(t) => connRef.current?.sendChat(t)}
          placeholder="Escribe al técnico…"
        />
        <FilePanel
          ready={filesReady}
          transfers={transfers}
          received={received}
          requested={requested}
          onSend={(f) => connRef.current?.sendFile(f)}
          onRequest={() => connRef.current?.requestFile()}
          onSent={clearRequested}
        />
      </aside>
    </main>
  );
}
