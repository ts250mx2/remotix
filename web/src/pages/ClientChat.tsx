import { useEffect, useRef, useState, type FormEvent } from 'react';
import type { ChatChannel, ChatMessage } from '../chat/api';
import { SupportConnection } from '../helpdesk/connection';
import { CallSession } from '../chat/call';
import { CallView } from '../chat/CallView';
import { MessageList } from '../chat/MessageList';
import { EmojiPicker } from '../chat/EmojiPicker';

interface PcCreds {
  equipoId: string;
  agentSecret: string;
  projectId: string;
  name: string;
}

const LS_KEY = 'remotix_pc';

function loadCreds(): PcCreds | null {
  try {
    const raw = localStorage.getItem(LS_KEY);
    return raw ? (JSON.parse(raw) as PcCreds) : null;
  } catch {
    return null;
  }
}

function chatWsUrl(): string {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${proto}//${location.host}/ws/chat`;
}

export function ClientChat() {
  const [creds, setCreds] = useState<PcCreds | null>(loadCreds());
  const [uuid, setUuid] = useState('');
  const [pcName, setPcName] = useState('');
  const [joining, setJoining] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [ready, setReady] = useState(false);
  const [channels, setChannels] = useState<ChatChannel[]>([]);
  const [channelId, setChannelId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [text, setText] = useState('');
  const [invite, setInvite] = useState<{ code: string; from: string } | null>(null);
  const [sharing, setSharing] = useState(false);
  const [shareStatus, setShareStatus] = useState('');
  const [callActive, setCallActive] = useState(false);
  const [localStream, setLocalStream] = useState<MediaStream | null>(null);
  const [remoteStreams, setRemoteStreams] = useState<Record<string, MediaStream | null>>({});
  const [callPeers, setCallPeers] = useState<string[]>([]);
  const [micOn, setMicOn] = useState(true);
  const [camOn, setCamOn] = useState(true);
  const wsRef = useRef<WebSocket | null>(null);
  const callRef = useRef<CallSession | null>(null);
  const remoteRef = useRef<SupportConnection | null>(null);
  const credsRef = useRef<PcCreds | null>(creds);
  credsRef.current = creds;
  const logRef = useRef<HTMLDivElement>(null);
  const channelIdRef = useRef<string | null>(null);
  channelIdRef.current = channelId;

  // Conectar al chat como PC cuando hay credenciales.
  useEffect(() => {
    if (!creds) return;
    let closed = false;
    let retry: ReturnType<typeof setTimeout> | undefined;

    function connect() {
      const ws = new WebSocket(chatWsUrl());
      wsRef.current = ws;
      ws.onopen = () => ws.send(JSON.stringify({ type: 'auth', equipoId: creds!.equipoId, agentSecret: creds!.agentSecret }));
      ws.onmessage = (e) => {
        const m = JSON.parse(e.data);
        if (m.type === 'ready') {
          setReady(true);
          setChannels(m.channels ?? []);
          const first = (m.channels ?? [])[0];
          if (first) {
            setChannelId(first.id);
            ws.send(JSON.stringify({ type: 'history', channelId: first.id }));
          }
        } else if (m.type === 'history') {
          if (m.channelId === channelIdRef.current) setMessages(m.messages ?? []);
        } else if (m.type === 'message') {
          if (m.message.channelId === channelIdRef.current) {
            setMessages((prev) => (prev.some((x) => x.id === m.message.id) ? prev : [...prev, m.message]));
          }
        } else if (m.type === 'remote-invite') {
          setInvite({ code: String(m.code), from: String(m.from ?? 'El técnico') });
        } else if (typeof m.type === 'string' && m.type.startsWith('call-')) {
          if (m.type === 'call-state') {
            if (m.channelId === channelIdRef.current) setCallPeers(m.peers ?? []);
          } else {
            callRef.current?.handle(m);
          }
        } else if (m.type === 'error' && m.code === 'auth_failed') {
          setError('Las credenciales del equipo no son válidas. Vuelve a conectar con el UUID.');
          localStorage.removeItem(LS_KEY);
          setCreds(null);
        }
      };
      ws.onclose = () => {
        setReady(false);
        if (!closed) retry = setTimeout(connect, 1500);
      };
      ws.onerror = () => ws.close();
    }
    connect();
    return () => { closed = true; if (retry) clearTimeout(retry); wsRef.current?.close(); };
  }, [creds]);

  useEffect(() => {
    logRef.current?.scrollTo({ top: logRef.current.scrollHeight, behavior: 'smooth' });
  }, [messages.length]);

  async function join(e: FormEvent) {
    e.preventDefault();
    setError(null);
    const projectId = uuid.trim();
    const name = pcName.trim();
    if (!projectId || !name) return;
    setJoining(true);
    try {
      const res = await fetch('/api/agent/join', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ projectId, name }),
      });
      if (!res.ok) throw new Error('join');
      const data = (await res.json()) as PcCreds;
      localStorage.setItem(LS_KEY, JSON.stringify(data));
      setCreds(data);
    } catch {
      setError('No se pudo conectar. Verifica el UUID del proyecto.');
    } finally {
      setJoining(false);
    }
  }

  function selectChannel(id: string) {
    setChannelId(id);
    setMessages([]);
    setCallPeers([]);
    wsRef.current?.send(JSON.stringify({ type: 'history', channelId: id }));
  }

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!text.trim() || !channelId) return;
    wsRef.current?.send(JSON.stringify({ type: 'message', channelId, body: text.trim() }));
    setText('');
  }

  function requestSupport() {
    if (!channelId) return;
    wsRef.current?.send(JSON.stringify({ type: 'support', channelId }));
  }

  async function acceptRemote() {
    const inv = invite;
    const creds = credsRef.current;
    if (!inv || !creds) return;
    setInvite(null);
    const conn = new SupportConnection('client', {
      onStatus: (s) => setShareStatus(s),
      onPeerLeft: () => { setSharing(false); setShareStatus('El técnico cerró la sesión.'); },
      onShareEnded: () => { setSharing(false); setShareStatus('Dejaste de compartir.'); },
      onError: () => setShareStatus('Error en la conexión de pantalla.'),
      onClosed: () => setSharing(false),
    });
    conn.start({ name: creds.name, code: inv.code });
    remoteRef.current = conn;
    try {
      await conn.startScreenShare();
      setSharing(true);
    } catch {
      setShareStatus('No se pudo compartir la pantalla (¿cancelaste?).');
      conn.close();
    }
  }

  function stopSharing() {
    remoteRef.current?.close();
    remoteRef.current = null;
    setSharing(false);
  }

  function startCall() {
    if (!channelId || callActive) return;
    const cs = new CallSession((obj) => wsRef.current?.send(JSON.stringify(obj)), {
      onLocalStream: setLocalStream,
      onRemoteStream: (id, s) => setRemoteStreams((p) => ({ ...p, [id]: s })),
      onActive: (a) => { setCallActive(a); if (!a) setRemoteStreams({}); },
      onError: (e) => alert(e),
    });
    callRef.current = cs;
    setMicOn(true); setCamOn(true);
    void cs.start(channelId);
  }
  function leaveCall() { callRef.current?.leave(); callRef.current = null; }
  function toggleMic() { setMicOn((v) => { callRef.current?.toggleMic(!v); return !v; }); }
  function toggleCam() { setCamOn((v) => { callRef.current?.toggleCam(!v); return !v; }); }

  function disconnect() {
    localStorage.removeItem(LS_KEY);
    callRef.current?.leave();
    remoteRef.current?.close();
    wsRef.current?.close();
    setCreds(null);
    setChannels([]);
    setMessages([]);
  }

  if (!creds) {
    return (
      <main className="centered">
        <form onSubmit={join} className="card narrow">
          <h1>Conectar al soporte</h1>
          <p className="muted">Introduce el código (UUID) del proyecto que te dio tu proveedor y un nombre para este equipo.</p>
          <label>
            UUID del proyecto
            <input value={uuid} onChange={(e) => setUuid(e.target.value)} placeholder="py_…" autoFocus />
          </label>
          <label>
            Nombre de este equipo
            <input value={pcName} onChange={(e) => setPcName(e.target.value)} placeholder="Ej: Recepción, Caja 1…" />
          </label>
          {error && <div className="error">{error}</div>}
          <button type="submit" disabled={joining || !uuid.trim() || !pcName.trim()}>
            {joining ? 'Conectando…' : 'Conectar'}
          </button>
        </form>
      </main>
    );
  }

  return (
    <div className="chat-shell client">
      <nav className="chat-nav">
        <span className="brand">Remotix</span>
        <p className="muted small">Equipo: <strong>{creds.name}</strong></p>
        <div className="chat-channels">
          <div className="chat-sec-head"><span>Canales</span></div>
          {channels.map((c) => (
            <button key={c.id} className={`chat-ch ${c.id === channelId ? 'active' : ''}`} onClick={() => selectChannel(c.id)}>
              # {c.name}
            </button>
          ))}
        </div>
        <div className="chat-nav-foot">
          <span className={`conn ${ready ? 'on' : ''}`}>{ready ? 'conectado' : 'reconectando…'}</span>
          <button className="ghost" onClick={disconnect}>Desconectar este equipo</button>
        </div>
      </nav>

      <main className="chat-main">
        <header className="chat-main-head">
          <h2># {channels.find((c) => c.id === channelId)?.name ?? '—'}</h2>
          <span className="row">
            {channelId && !callActive && (
              <button onClick={startCall}>📞 {callPeers.length > 0 ? `Unirse (${callPeers.length})` : 'Videollamada'}</button>
            )}
            <button className="danger" onClick={requestSupport} disabled={!channelId}>🆘 Pedir soporte</button>
          </span>
        </header>

        {callActive && (
          <CallView
            localStream={localStream}
            remotes={Object.entries(remoteStreams).filter(([, s]) => s).map(([id, s]) => ({ id, name: id.slice(0, 8), stream: s }))}
            micOn={micOn}
            camOn={camOn}
            onToggleMic={toggleMic}
            onToggleCam={toggleCam}
            onLeave={leaveCall}
            selfName={creds.name}
          />
        )}

        {invite && !sharing && (
          <div className="remote-invite">
            <span>🖥️ <strong>{invite.from}</strong> quiere ver tu pantalla para ayudarte.</span>
            <span className="row">
              <button onClick={acceptRemote}>Permitir</button>
              <button className="ghost" onClick={() => setInvite(null)}>Rechazar</button>
            </span>
          </div>
        )}
        {sharing && (
          <div className="remote-sharing">
            <span>🔴 Compartiendo tu pantalla con el técnico. {shareStatus}</span>
            <button className="danger" onClick={stopSharing}>Detener</button>
          </div>
        )}
        {!sharing && !invite && shareStatus && <div className="muted small remote-note">{shareStatus}</div>}

        <MessageList
          ref={logRef}
          messages={messages}
          selfId={creds.equipoId}
          resolveName={(id, kind) => (id === creds.equipoId ? creds.name : kind === 'pc' ? 'Equipo' : 'Técnico')}
          emptyText="Escribe a tu técnico o pulsa “Pedir soporte”."
        />
        <form className="chat-composer" onSubmit={submit}>
          <EmojiPicker onPick={(e) => setText((t) => t + e)} />
          <input value={text} onChange={(e) => setText(e.target.value)} placeholder="Escribe un mensaje al técnico…" disabled={!channelId} />
          <button type="submit" disabled={!channelId || !text.trim()}>Enviar</button>
        </form>
      </main>
    </div>
  );
}
