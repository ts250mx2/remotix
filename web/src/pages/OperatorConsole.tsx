import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import {
  SupportConnection,
  errorText,
  type ChatMessage,
  type InputEvent as ControlEvent,
  type SessionMode,
} from '../helpdesk/connection';
import { ChatPanel } from '../helpdesk/ChatPanel';
import { FilePanel, useFileTransfers } from '../helpdesk/FilePanel';

type Phase = 'enter-code' | 'live';

/** Mapea un punto del cursor (clientX/Y) a coords normalizadas 0..1 del contenido
 * del vídeo, teniendo en cuenta el letterboxing de object-fit: contain. */
function pointToVideo(video: HTMLVideoElement, clientX: number, clientY: number): { x: number; y: number } | null {
  const rect = video.getBoundingClientRect();
  const vw = video.videoWidth;
  const vh = video.videoHeight;
  if (!vw || !vh || !rect.width || !rect.height) return null;
  const scale = Math.min(rect.width / vw, rect.height / vh);
  const dispW = vw * scale;
  const dispH = vh * scale;
  const offX = (rect.width - dispW) / 2;
  const offY = (rect.height - dispH) / 2;
  const px = clientX - rect.left - offX;
  const py = clientY - rect.top - offY;
  if (px < 0 || py < 0 || px > dispW || py > dispH) return null;
  return { x: px / dispW, y: py / dispH };
}

export function OperatorConsole() {
  const [phase, setPhase] = useState<Phase>('enter-code');
  const [code, setCode] = useState('');

  const [status, setStatus] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [clientMeta, setClientMeta] = useState<{ name?: string; issue?: string }>({});
  const [clientPresent, setClientPresent] = useState(true);
  const [hasVideo, setHasVideo] = useState(false);
  const [mode, setMode] = useState<SessionMode>('share');
  const [controlReady, setControlReady] = useState(false);
  const [controlling, setControlling] = useState(false);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const { filesReady, transfers, received, requested, clearRequested, fileHandlers } = useFileTransfers();

  const connRef = useRef<SupportConnection | null>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const fullscreenRef = useRef<HTMLDivElement>(null);
  const lastMoveRef = useRef(0);
  const controllingRef = useRef(false);

  useEffect(() => {
    controllingRef.current = controlling;
  }, [controlling]);

  useEffect(() => {
    return () => connRef.current?.close();
  }, []);

  function pushSystem(text: string) {
    setMessages((m) => [...m, { text, from: 'system', ts: Date.now() }]);
  }

  const send = useCallback((evt: ControlEvent) => connRef.current?.sendInput(evt), []);

  // Captura de teclado a nivel ventana mientras se controla.
  useEffect(() => {
    if (!controlling) return;
    const onKey = (e: KeyboardEvent) => {
      // No interferir si el foco está en un input (p. ej. el chat).
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA') return;
      e.preventDefault();
      send({ k: 'key', down: e.type === 'keydown', code: e.code, key: e.key });
    };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('keyup', onKey, true);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('keyup', onKey, true);
    };
  }, [controlling, send]);

  // Auto-conexión cuando el técnico lanza la sesión desde el chat (?code=...).
  useEffect(() => {
    const p = new URLSearchParams(window.location.search).get('code');
    if (p) {
      setCode(p.toUpperCase());
      connectWith(p);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function connect(e: FormEvent) {
    e.preventDefault();
    const entered = code.trim();
    if (!entered) return;
    setError(null);
    // ¿Es una clave fija de equipo (Lite)? Intentar conectar por clave.
    try {
      const res = await fetch('/api/device/connect', {
        method: 'POST', headers: { 'content-type': 'application/json' }, credentials: 'omit',
        body: JSON.stringify({ accessKey: entered }),
      });
      if (res.ok) { const data = await res.json(); connectWith(data.code); return; }
      if (res.status === 409) { setError('Ese equipo no está en línea en este momento.'); return; }
      // 404 → no es una clave de equipo; se trata como código de soporte efímero.
    } catch { /* sin acceso → intentar como código directo */ }
    connectWith(entered);
  }

  function connectWith(rawCode: string) {
    setError(null);
    const normalized = rawCode.trim().toUpperCase();
    if (!normalized) return;

    const conn = new SupportConnection(
      'operator',
      {
        onStatus: (s) => setStatus(s),
        onPeerJoined: (meta) => {
          setClientMeta({ name: meta.name, issue: meta.issue });
          setMode(meta.mode ?? 'share');
          setClientPresent(true);
        },
        onPeerLeft: () => {
          setClientPresent(false);
          setHasVideo(false);
          setControlling(false);
          if (videoRef.current) videoRef.current.srcObject = null;
          pushSystem('El usuario se desconectó.');
        },
        onChat: (msg) => setMessages((m) => [...m, msg]),
        onRemoteStream: (stream) => {
          if (videoRef.current) videoRef.current.srcObject = stream;
          setHasVideo(!!stream);
        },
        onControlReady: (ready) => {
          setControlReady(ready);
          if (!ready) setControlling(false);
        },
        onError: (c) => {
          setError(errorText(c));
          if (c === 'not_found' || c === 'busy') setPhase('enter-code');
        },
        onClosed: () => setStatus('Conexión cerrada.'),
        ...fileHandlers,
      },
      normalized,
    );
    conn.start();
    connRef.current = conn;
    setPhase('live');
  }

  function goFullscreen() {
    void fullscreenRef.current?.requestFullscreen?.();
  }

  // --- handlers de control sobre el vídeo ---
  function emitPointer(kind: 'move' | 'down' | 'up', e: React.PointerEvent<HTMLVideoElement>) {
    if (!controllingRef.current || !videoRef.current) return;
    const p = pointToVideo(videoRef.current, e.clientX, e.clientY);
    if (!p) return;
    if (kind === 'move') {
      const now = performance.now();
      if (now - lastMoveRef.current < 20) return; // ~50 ev/s máx
      lastMoveRef.current = now;
      send({ k: 'move', x: p.x, y: p.y });
    } else {
      send({ k: kind, x: p.x, y: p.y, button: e.button });
    }
  }

  function onWheel(e: React.WheelEvent<HTMLVideoElement>) {
    if (!controllingRef.current || !videoRef.current) return;
    const p = pointToVideo(videoRef.current, e.clientX, e.clientY);
    if (!p) return;
    send({ k: 'wheel', x: p.x, y: p.y, dx: e.deltaX, dy: e.deltaY });
  }

  if (phase === 'enter-code') {
    return (
      <main className="centered">
        <form onSubmit={connect} className="card narrow">
          <h1>Consola del técnico</h1>
          <p className="muted">Introduce el <strong>código de soporte</strong> (temporal, 6 caracteres) o la <strong>clave fija</strong> de un equipo Remotix.</p>
          <label>
            Código o clave del equipo
            <input
              value={code}
              onChange={(e) => setCode(e.target.value.toUpperCase())}
              placeholder="Ej: K7M2QP  ·  o  ABC-DEF-GHJ"
              maxLength={20}
              autoFocus
              style={{ letterSpacing: '0.25em', textTransform: 'uppercase' }}
            />
          </label>
          {error && <div className="error">{error}</div>}
          <button type="submit" disabled={!code.trim()}>
            Conectar
          </button>
        </form>
      </main>
    );
  }

  const canControl = mode === 'agent';

  return (
    <main className="helpdesk">
      <section className="hd-main">
        <header className="hd-header">
          <div>
            <h1>{clientMeta.name ? `Soporte a ${clientMeta.name}` : 'Sesión de soporte'}</h1>
            {clientMeta.issue && <p className="muted small">{clientMeta.issue}</p>}
          </div>
          <span className={`hd-dot ${clientPresent ? 'on' : ''}`}>
            {clientPresent ? 'Usuario conectado' : 'Usuario desconectado'}
          </span>
        </header>

        {error && <div className="error">{error}</div>}
        {!hasVideo && status && <p className="muted small">{status}</p>}

        <div className="hd-toolbar">
          {canControl ? (
            <button
              className={controlling ? 'danger' : ''}
              disabled={!controlReady}
              onClick={() => setControlling((v) => !v)}
              title={controlReady ? '' : 'Esperando el canal de control del agente…'}
            >
              {controlling ? 'Soltar control' : 'Tomar control'}
            </button>
          ) : (
            <span className="muted small">Modo solo lectura (pantalla compartida desde el navegador).</span>
          )}
          {hasVideo && (
            <button className="ghost" onClick={goFullscreen}>
              Pantalla completa
            </button>
          )}
        </div>

        <div className={`screen-box ${controlling ? 'controlling' : ''}`} ref={fullscreenRef}>
          {/* eslint-disable-next-line jsx-a11y/media-has-caption */}
          <video
            ref={videoRef}
            autoPlay
            playsInline
            tabIndex={canControl ? 0 : undefined}
            className="remote-screen"
            onPointerMove={(e) => emitPointer('move', e)}
            onPointerDown={(e) => {
              if (controllingRef.current) (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
              emitPointer('down', e);
            }}
            onPointerUp={(e) => emitPointer('up', e)}
            onWheel={onWheel}
            onContextMenu={(e) => {
              if (controllingRef.current) e.preventDefault();
            }}
          />
          {!hasVideo && (
            <div className="screen-overlay">
              <p className="muted">Esperando a que el usuario comparta su pantalla…</p>
            </div>
          )}
        </div>
        {controlling && <p className="muted small">Estás controlando el equipo remoto. Tu mouse y teclado se envían al usuario.</p>}
      </section>

      <aside className="hd-side">
        <h2>Chat</h2>
        <ChatPanel
          messages={messages}
          onSend={(t) => connRef.current?.sendChat(t)}
          disabled={!clientPresent}
          placeholder="Escribe al usuario…"
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
