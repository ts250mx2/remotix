import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import { Link } from 'react-router-dom';
import { api, HttpError, type Project } from '../api/client';
import { useAuth } from '../auth/AuthContext';
import { chatApi, type ChatChannel, type ChatMessage, type RosterEntry } from '../chat/api';
import { useChat } from '../chat/useChat';
import { CallSession } from '../chat/call';
import { CallView } from '../chat/CallView';
import { MessageList } from '../chat/MessageList';
import { EmojiPicker } from '../chat/EmojiPicker';
import { avatarColor, initials } from '../chat/avatar';

export function ChatPage() {
  const { user } = useAuth();
  const [empresas, setEmpresas] = useState<Project[]>([]);
  const [empresaId, setEmpresaId] = useState<string | null>(null);
  const [channels, setChannels] = useState<ChatChannel[]>([]);
  const [channelId, setChannelId] = useState<string | null>(null);
  const [byChannel, setByChannel] = useState<Record<string, ChatMessage[]>>({});
  const [roster, setRoster] = useState<RosterEntry[]>([]);
  const [text, setText] = useState('');
  const logRef = useRef<HTMLDivElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  // Videollamada (malla)
  const callRef = useRef<CallSession | null>(null);
  const [callActive, setCallActive] = useState(false);
  const [localStream, setLocalStream] = useState<MediaStream | null>(null);
  const [remoteStreams, setRemoteStreams] = useState<Record<string, MediaStream | null>>({});
  const [callStates, setCallStates] = useState<Record<string, string[]>>({});
  const [micOn, setMicOn] = useState(true);
  const [camOn, setCamOn] = useState(true);

  const onMessage = useCallback((m: ChatMessage) => {
    setByChannel((prev) => {
      const list = prev[m.channelId] ?? [];
      if (list.some((x) => x.id === m.id)) return prev;
      return { ...prev, [m.channelId]: [...list, m] };
    });
  }, []);

  const onCall = useCallback((m: Record<string, unknown>) => {
    if (m.type === 'call-state') {
      setCallStates((p) => ({ ...p, [String(m.channelId)]: (m.peers as string[]) ?? [] }));
    } else {
      callRef.current?.handle(m as never);
    }
  }, []);

  const { ready, presence, sendMessage, sendRaw } = useChat({ onMessage, onCall });

  useEffect(() => () => callRef.current?.leave(), []);

  function startCall() {
    if (!channelId || callActive) return;
    const cs = new CallSession(sendRaw, {
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

  // Cargar empresas.
  useEffect(() => {
    void api.get<{ projects: Project[] }>('/api/projects').then((r) => {
      setEmpresas(r.projects);
      if (r.projects[0]) setEmpresaId(r.projects[0].id);
    });
  }, []);

  // Al cambiar de empresa: canales + roster.
  useEffect(() => {
    if (!empresaId) return;
    void chatApi.channels(empresaId).then((chs) => {
      setChannels(chs);
      setChannelId((cur) => (cur && chs.some((c) => c.id === cur) ? cur : chs[0]?.id ?? null));
    });
    void chatApi.roster(empresaId).then(setRoster);
  }, [empresaId]);

  // Al seleccionar canal: cargar historial si no está.
  useEffect(() => {
    if (!channelId || byChannel[channelId]) return;
    void chatApi.messages(channelId).then((msgs) => setByChannel((p) => ({ ...p, [channelId]: msgs })));
  }, [channelId, byChannel]);

  const messages = channelId ? byChannel[channelId] ?? [] : [];

  useEffect(() => {
    logRef.current?.scrollTo({ top: logRef.current.scrollHeight, behavior: 'smooth' });
  }, [messages.length, channelId]);

  // Mapa id → nombre (desde el roster).
  const names = useMemo(() => {
    const m: Record<string, string> = {};
    for (const r of roster) m[r.id] = r.name;
    return m;
  }, [roster]);

  const isOnline = (id: string) => presence[id] ?? roster.find((r) => r.id === id)?.online ?? false;

  // ¿El usuario actual es técnico/admin en esta empresa? (puede control remoto)
  const empresa = empresas.find((e) => e.id === empresaId);
  const myRole = roster.find((r) => r.id === user?.id)?.role;
  const canRemote = empresa?.isOwner || myRole === 'admin' || myRole === 'tecnico' || myRole === 'operator';

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!text.trim() || !channelId) return;
    sendMessage(channelId, text.trim());
    setText('');
  }

  async function onAttach(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = '';
    if (!file || !channelId) return;
    try {
      await chatApi.uploadFile(channelId, file, text.trim());
      setText('');
    } catch {
      alert('No se pudo subir el archivo.');
    }
  }

  async function launchRemote(equipoId: string) {
    try {
      const { code } = await api.post<{ code: string }>(`/api/chat/remote/${equipoId}`);
      window.open(`/operador?code=${code}`, '_blank', 'noopener');
    } catch (err) {
      alert(err instanceof HttpError && err.payload?.error === 'pc_offline'
        ? 'El PC no está conectado al chat.'
        : 'No se pudo iniciar el control remoto.');
    }
  }

  async function newChannel() {
    if (!empresaId) return;
    const name = prompt('Nombre del canal:')?.trim();
    if (!name) return;
    const ch = await chatApi.createChannel(empresaId, name);
    setChannels((prev) => [...prev, ch]);
    setChannelId(ch.id);
  }

  return (
    <div className="chat-shell">
      {/* Sidebar: empresas + canales */}
      <nav className="chat-nav">
        <Link to="/" className="brand">Remotix</Link>
        <div className="chat-empresas">
          <label className="muted small">Empresa</label>
          <select value={empresaId ?? ''} onChange={(e) => setEmpresaId(e.target.value)}>
            {empresas.map((e) => (
              <option key={e.id} value={e.id}>{e.name}</option>
            ))}
          </select>
        </div>
        <div className="chat-channels">
          <div className="chat-sec-head">
            <span>Canales</span>
            {canRemote && <button className="mini" onClick={newChannel} title="Nuevo canal">+</button>}
          </div>
          {channels.map((c) => (
            <button
              key={c.id}
              className={`chat-ch ${c.id === channelId ? 'active' : ''}`}
              onClick={() => setChannelId(c.id)}
            >
              {c.kind === 'support' ? '🆘' : '#'} {c.name}
            </button>
          ))}
          {channels.length === 0 && <p className="muted small">Sin canales.</p>}
        </div>
        <div className="chat-nav-foot">
          {empresa && <Link className="muted small" to={`/projects/${empresa.id}`}>⚙ Administrar empresa</Link>}
          <span className={`conn ${ready ? 'on' : ''}`}>{ready ? 'conectado' : 'reconectando…'}</span>
        </div>
      </nav>

      {/* Mensajes */}
      <main className="chat-main">
        <header className="chat-main-head">
          <h2># {channels.find((c) => c.id === channelId)?.name ?? '—'}</h2>
          {channelId && !callActive && (
            <button onClick={startCall}>
              📞 {((callStates[channelId]?.length ?? 0) > 0) ? `Unirse a llamada (${callStates[channelId].length})` : 'Videollamada'}
            </button>
          )}
        </header>

        {callActive && (
          <CallView
            localStream={localStream}
            remotes={Object.entries(remoteStreams).filter(([, s]) => s).map(([id, s]) => ({ id, name: names[id] ?? id.slice(0, 8), stream: s }))}
            micOn={micOn}
            camOn={camOn}
            onToggleMic={toggleMic}
            onToggleCam={toggleCam}
            onLeave={leaveCall}
            selfName={user?.name ?? 'Yo'}
          />
        )}

        <MessageList
          ref={logRef}
          messages={messages}
          selfId={user?.id ?? ''}
          resolveName={(id) => names[id] ?? id.slice(0, 8)}
          fileUrl={chatApi.fileUrl}
          emptyText="No hay mensajes en este canal todavía. Escribe el primero."
        />
        <form className="chat-composer" onSubmit={submit}>
          <input ref={fileRef} type="file" hidden onChange={onAttach} />
          <button type="button" className="ghost" disabled={!channelId} title="Adjuntar archivo" onClick={() => fileRef.current?.click()}>📎</button>
          <EmojiPicker onPick={(e) => setText((t) => t + e)} />
          <input
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder={channelId ? 'Escribe un mensaje…' : 'Selecciona un canal'}
            disabled={!channelId}
          />
          <button type="submit" disabled={!channelId || !text.trim()}>Enviar</button>
        </form>
      </main>

      {/* Roster */}
      <aside className="chat-roster">
        <h3>Miembros</h3>
        <div className="roster-group">
          <p className="muted small">PCs ({roster.filter((r) => r.kind === 'pc').length})</p>
          {roster.filter((r) => r.kind === 'pc').map((r) => (
            <div key={r.id} className="roster-row">
              <span className={`r-avatar ${isOnline(r.id) ? 'on' : ''}`} style={{ background: avatarColor(r.id) }}>💻</span>
              <span className="roster-name">
                {r.name}
                {r.currentUserId && names[r.currentUserId] && <span className="muted small"> · {names[r.currentUserId]}</span>}
              </span>
              {canRemote && (
                <button className="mini" disabled={!isOnline(r.id)} onClick={() => launchRemote(r.id)} title="Ver pantalla / control remoto">🖥️</button>
              )}
            </div>
          ))}
          {roster.filter((r) => r.kind === 'pc').length === 0 && <p className="muted small">Sin PCs.</p>}
        </div>
        <div className="roster-group">
          <p className="muted small">Usuarios ({roster.filter((r) => r.kind === 'user').length})</p>
          {roster.filter((r) => r.kind === 'user').map((r) => (
            <div key={r.id} className="roster-row">
              <span className={`r-avatar ${isOnline(r.id) ? 'on' : ''}`} style={{ background: avatarColor(r.id) }}>{initials(r.name)}</span>
              <span className="roster-name">{r.name}</span>
              {r.role && <span className={`role-tag ${r.role}`}>{r.role}</span>}
            </div>
          ))}
        </div>
      </aside>
    </div>
  );
}
