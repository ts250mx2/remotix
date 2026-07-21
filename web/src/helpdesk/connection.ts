// Conexión de soporte: encapsula la señalización por WebSocket y la sesión
// WebRTC entre cliente (comparte pantalla) y operador (la ve). El chat viaja
// por el WebSocket (siempre funciona, aunque el vídeo P2P falle); el vídeo va
// peer-to-peer.

export type Role = 'client' | 'operator';

export interface ChatMessage {
  text: string;
  from: 'me' | 'peer' | 'system';
  ts: number;
}

export type SessionMode = 'share' | 'agent';

export interface PeerMeta {
  name?: string;
  issue?: string;
  mode?: SessionMode;
  caps?: string[];
}

export interface FileProgress {
  dir: 'in' | 'out';
  id: number;
  name: string;
  transferred: number;
  total: number;
}

export interface ReceivedFile {
  name: string;
  blob: Blob;
}

export interface ConnectionHandlers {
  onCode?: (code: string) => void;           // cliente: código asignado al hacer host
  onStatus?: (status: string) => void;        // texto de estado legible
  onPeerJoined?: (meta: PeerMeta) => void;
  onPeerLeft?: () => void;
  onChat?: (msg: ChatMessage) => void;
  onRemoteStream?: (stream: MediaStream | null) => void; // operador: vídeo entrante
  onControlReady?: (ready: boolean) => void;   // operador: canal de control abierto/cerrado
  onFilesReady?: (ready: boolean) => void;     // canal de archivos abierto/cerrado
  onFileProgress?: (p: FileProgress) => void;  // progreso de envío/recepción
  onFileReceived?: (file: ReceivedFile) => void; // archivo entrante completo
  onFileRequested?: () => void;                // el peer pide que enviemos un archivo
  onShareEnded?: () => void;                   // cliente: dejó de compartir pantalla
  onWaiting?: () => void;                      // operador: sala reservada, el equipo aún no se une
  onError?: (code: string) => void;
  onClosed?: () => void;
}

/** Evento de control operador → agente (canal 'control'). Coords normalizadas 0..1. */
export type InputEvent =
  | { k: 'move'; x: number; y: number }
  | { k: 'down'; x: number; y: number; button: number }
  | { k: 'up'; x: number; y: number; button: number }
  | { k: 'wheel'; x: number; y: number; dx: number; dy: number }
  | { k: 'key'; down: boolean; code: string; key: string };

const DEFAULT_ICE: RTCIceServer[] = [
  { urls: 'stun:stun.l.google.com:19302' },
  { urls: 'stun:stun1.l.google.com:19302' },
];

function wsUrl(): string {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${proto}//${location.host}/ws/signal`;
}

export class SupportConnection {
  private ws: WebSocket | null = null;
  private pc: RTCPeerConnection | null = null;
  private localStream: MediaStream | null = null;
  private controlChannel: RTCDataChannel | null = null;
  private filesChannel: RTCDataChannel | null = null;
  private incoming: { id: number; name: string; size: number; mime: string; chunks: Uint8Array[]; received: number } | null = null;
  private fileSeq = 0;
  private iceServers: RTCIceServer[] = DEFAULT_ICE;
  private peerPresent = false;
  private remoteDescSet = false;
  private pendingCandidates: RTCIceCandidateInit[] = [];
  private closed = false;

  constructor(
    private role: Role,
    private handlers: ConnectionHandlers,
    private joinCode?: string, // requerido para operador
  ) {}

  // ---- ciclo de vida ----

  start(meta?: { name?: string; issue?: string; code?: string }): void {
    this.handlers.onStatus?.('Conectando…');
    void this.init(meta);
  }

  private async init(meta?: { name?: string; issue?: string; code?: string }): Promise<void> {
    await this.loadIceServers();
    if (this.closed) return;
    const ws = new WebSocket(wsUrl());
    this.ws = ws;

    ws.onopen = () => {
      if (this.role === 'client') {
        // El navegador solo comparte pantalla (modo 'share'): no puede inyectar input.
        // `code` se incluye cuando el técnico lanzó la sesión desde el chat (sala reservada).
        this.send({ t: 'host', name: meta?.name, issue: meta?.issue, mode: 'share', caps: [], code: meta?.code });
      } else {
        this.send({ t: 'join', code: (this.joinCode ?? '').trim().toUpperCase() });
      }
    };

    ws.onmessage = (ev) => {
      let msg: Record<string, unknown>;
      try {
        msg = JSON.parse(typeof ev.data === 'string' ? ev.data : '');
      } catch {
        return;
      }
      void this.onSignalMessage(msg);
    };

    ws.onclose = () => {
      if (!this.closed) this.handlers.onClosed?.();
    };
    ws.onerror = () => this.handlers.onError?.('ws_error');
  }

  private async loadIceServers(): Promise<void> {
    try {
      const res = await fetch('/api/turn-credentials', { credentials: 'omit' });
      if (res.ok) {
        const data = (await res.json()) as { iceServers?: RTCIceServer[] };
        if (Array.isArray(data.iceServers) && data.iceServers.length > 0) {
          this.iceServers = data.iceServers;
        }
      }
    } catch {
      /* sin acceso al endpoint → se usa STUN por defecto */
    }
  }

  close(): void {
    this.closed = true;
    try {
      this.send({ t: 'bye' });
    } catch {
      /* noop */
    }
    this.stopScreenShare();
    this.controlChannel = null;
    this.filesChannel = null;
    this.pc?.close();
    this.pc = null;
    this.ws?.close();
    this.ws = null;
  }

  // ---- control remoto (operador → agente) ----

  /** Envía un evento de input al agente si el canal de control está abierto. */
  sendInput(evt: InputEvent): void {
    const ch = this.controlChannel;
    if (ch && ch.readyState === 'open') ch.send(JSON.stringify(evt));
  }

  hasControl(): boolean {
    return this.controlChannel?.readyState === 'open';
  }

  // ---- transferencia de archivos (canal 'files', bidireccional) ----

  hasFiles(): boolean {
    return this.filesChannel?.readyState === 'open';
  }

  /** Pide al peer que nos envíe un archivo (abre su selector / diálogo). */
  requestFile(): void {
    const ch = this.filesChannel;
    if (ch && ch.readyState === 'open') ch.send(JSON.stringify({ f: 'req' }));
  }

  /** Envía un archivo al peer en bloques, con control de backpressure. */
  async sendFile(file: File): Promise<void> {
    const ch = this.filesChannel;
    if (!ch || ch.readyState !== 'open') return;
    const id = ++this.fileSeq;
    const CHUNK = 16 * 1024;
    ch.send(JSON.stringify({ f: 'begin', id, name: file.name, size: file.size, mime: file.type }));
    let offset = 0;
    while (offset < file.size) {
      const buf = await file.slice(offset, offset + CHUNK).arrayBuffer();
      // Backpressure: no acumular más de 4 MB en el buffer de envío.
      while (ch.bufferedAmount > 4 * 1024 * 1024) {
        await new Promise((r) => setTimeout(r, 20));
        if (ch.readyState !== 'open') return;
      }
      ch.send(buf);
      offset += buf.byteLength;
      this.handlers.onFileProgress?.({ dir: 'out', id, name: file.name, transferred: offset, total: file.size });
    }
    ch.send(JSON.stringify({ f: 'end', id }));
  }

  private attachFilesChannel(ch: RTCDataChannel): void {
    ch.binaryType = 'arraybuffer';
    this.filesChannel = ch;
    ch.onopen = () => this.handlers.onFilesReady?.(true);
    ch.onclose = () => this.handlers.onFilesReady?.(false);
    ch.onmessage = (ev) => this.onFileMessage(ev.data);
    if (ch.readyState === 'open') this.handlers.onFilesReady?.(true);
  }

  private onFileMessage(data: string | ArrayBuffer): void {
    if (typeof data === 'string') {
      let m: { f?: string; id?: number; name?: string; size?: number; mime?: string };
      try {
        m = JSON.parse(data);
      } catch {
        return;
      }
      if (m.f === 'begin') {
        this.incoming = {
          id: m.id ?? 0,
          name: m.name ?? 'archivo',
          size: m.size ?? 0,
          mime: m.mime || 'application/octet-stream',
          chunks: [],
          received: 0,
        };
        this.handlers.onFileProgress?.({ dir: 'in', id: this.incoming.id, name: this.incoming.name, transferred: 0, total: this.incoming.size });
      } else if (m.f === 'end' && this.incoming) {
        const blob = new Blob(this.incoming.chunks as BlobPart[], { type: this.incoming.mime });
        this.handlers.onFileReceived?.({ name: this.incoming.name, blob });
        this.incoming = null;
      } else if (m.f === 'req') {
        this.handlers.onFileRequested?.();
      }
    } else if (this.incoming) {
      const arr = new Uint8Array(data);
      this.incoming.chunks.push(arr);
      this.incoming.received += arr.byteLength;
      this.handlers.onFileProgress?.({ dir: 'in', id: this.incoming.id, name: this.incoming.name, transferred: this.incoming.received, total: this.incoming.size });
    }
  }

  sendChat(text: string): void {
    const trimmed = text.trim();
    if (!trimmed) return;
    this.send({ t: 'chat', text: trimmed });
    this.handlers.onChat?.({ text: trimmed, from: 'me', ts: Date.now() });
  }

  // ---- compartir pantalla (solo cliente) ----

  async startScreenShare(): Promise<void> {
    if (this.role !== 'client') return;
    const stream = await navigator.mediaDevices.getDisplayMedia({
      video: { frameRate: { ideal: 15, max: 30 } },
      audio: false,
    });
    this.localStream = stream;
    // El navegador muestra su propio botón "Dejar de compartir".
    stream.getVideoTracks().forEach((track) => {
      track.onended = () => {
        this.handlers.onShareEnded?.();
        this.stopScreenShare();
      };
    });
    this.handlers.onStatus?.(
      this.peerPresent ? 'Compartiendo pantalla con el técnico.' : 'Pantalla lista. Esperando al técnico…',
    );
    this.maybeStartOffer();
  }

  getLocalStream(): MediaStream | null {
    return this.localStream;
  }

  isSharing(): boolean {
    return !!this.localStream && this.localStream.getVideoTracks().some((t) => t.readyState === 'live');
  }

  private stopScreenShare(): void {
    this.localStream?.getTracks().forEach((t) => t.stop());
    this.localStream = null;
  }

  // ---- señalización ----

  private async onSignalMessage(msg: Record<string, unknown>): Promise<void> {
    switch (msg.t) {
      case 'hosted':
        this.handlers.onCode?.(String(msg.code));
        this.handlers.onStatus?.('Esperando a que el técnico se conecte…');
        break;

      case 'joined':
        // (operador) Ya estamos en la sala: preparamos el receptor de vídeo.
        this.peerPresent = true;
        this.ensureOperatorPc();
        this.handlers.onPeerJoined?.({
          name: typeof msg.name === 'string' ? msg.name : undefined,
          issue: typeof msg.issue === 'string' ? msg.issue : undefined,
          mode: msg.mode === 'agent' ? 'agent' : 'share',
          caps: Array.isArray(msg.caps) ? (msg.caps as string[]) : [],
        });
        this.handlers.onStatus?.('Conectado. Esperando la pantalla del usuario…');
        break;

      case 'peer-joined':
        // (cliente) El técnico entró: si ya compartimos pantalla, ofertamos.
        this.peerPresent = true;
        this.handlers.onPeerJoined?.({});
        this.handlers.onStatus?.(
          this.localStream ? 'Técnico conectado. Compartiendo pantalla.' : 'Técnico conectado. Comparte tu pantalla.',
        );
        this.maybeStartOffer();
        break;

      case 'waiting':
        // (operador) sala reservada: el equipo se une al recibir el `start` —
        // al instante en modo desatendido, o cuando el usuario acepte si el
        // equipo tiene activado "pedir permiso antes de conectar". onWaiting
        // permite a la consola distinguir ambos textos.
        if (this.handlers.onWaiting) this.handlers.onWaiting();
        else this.handlers.onStatus?.('Conectando con el equipo…');
        break;

      case 'peer-left':
        this.peerPresent = false;
        this.teardownPc();
        this.handlers.onPeerLeft?.();
        break;

      case 'signal':
        await this.onRtcSignal(msg.payload as RtcSignal);
        break;

      case 'chat':
        this.handlers.onChat?.({
          text: String(msg.text ?? ''),
          from: 'peer',
          ts: typeof msg.ts === 'number' ? msg.ts : Date.now(),
        });
        break;

      case 'error':
        this.handlers.onError?.(String(msg.code ?? 'unknown'));
        break;
    }
  }

  private async onRtcSignal(payload: RtcSignal): Promise<void> {
    if (!payload) return;
    if (payload.sdp) {
      const desc = payload.sdp;
      if (desc.type === 'offer') {
        // (operador) recibe oferta del cliente.
        const pc = this.ensureOperatorPc();
        await pc.setRemoteDescription(desc);
        this.remoteDescSet = true;
        await this.flushCandidates();
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        this.send({ t: 'signal', payload: { sdp: pc.localDescription } });
      } else if (desc.type === 'answer') {
        // (cliente) recibe respuesta del operador.
        if (this.pc) {
          await this.pc.setRemoteDescription(desc);
          this.remoteDescSet = true;
          await this.flushCandidates();
        }
      }
    } else if (payload.candidate) {
      if (this.pc && this.remoteDescSet) {
        try {
          await this.pc.addIceCandidate(payload.candidate);
        } catch {
          /* candidato tardío/no aplicable */
        }
      } else {
        this.pendingCandidates.push(payload.candidate);
      }
    }
  }

  private async flushCandidates(): Promise<void> {
    if (!this.pc) return;
    const pending = this.pendingCandidates;
    this.pendingCandidates = [];
    for (const c of pending) {
      try {
        await this.pc.addIceCandidate(c);
      } catch {
        /* noop */
      }
    }
  }

  // ---- WebRTC ----

  private newPc(): RTCPeerConnection {
    const pc = new RTCPeerConnection({ iceServers: this.iceServers });
    pc.onicecandidate = (ev) => {
      if (ev.candidate) this.send({ t: 'signal', payload: { candidate: ev.candidate.toJSON() } });
    };
    pc.onconnectionstatechange = () => {
      const st = pc.connectionState;
      if (st === 'connected') this.handlers.onStatus?.('Pantalla en vivo.');
      else if (st === 'failed') this.handlers.onError?.('rtc_failed');
    };
    return pc;
  }

  /** Operador: crea el peer connection que recibirá el vídeo y el canal de control. */
  private ensureOperatorPc(): RTCPeerConnection {
    if (this.pc) return this.pc;
    const pc = this.newPc();
    pc.ontrack = (ev) => {
      this.handlers.onRemoteStream?.(ev.streams[0] ?? null);
    };
    // El offerer (agente o navegador-cliente) crea los canales; aquí los recibimos.
    pc.ondatachannel = (ev) => {
      if (ev.channel.label === 'control') this.attachControlChannel(ev.channel);
      else if (ev.channel.label === 'files') this.attachFilesChannel(ev.channel);
    };
    this.pc = pc;
    return pc;
  }

  private attachControlChannel(ch: RTCDataChannel): void {
    this.controlChannel = ch;
    ch.onopen = () => this.handlers.onControlReady?.(true);
    ch.onclose = () => this.handlers.onControlReady?.(false);
  }

  /** Cliente: cuando hay peer y pantalla, crea el PC, añade pistas y oferta. */
  private maybeStartOffer(): void {
    if (this.role !== 'client') return;
    if (!this.peerPresent || !this.localStream || this.pc) return;
    void this.createOffer();
  }

  private async createOffer(): Promise<void> {
    if (!this.localStream) return;
    const pc = this.newPc();
    this.pc = pc;
    this.localStream.getTracks().forEach((track) => pc.addTrack(track, this.localStream!));
    // El navegador-cliente (offerer) crea el canal de archivos.
    this.attachFilesChannel(pc.createDataChannel('files'));
    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    this.send({ t: 'signal', payload: { sdp: pc.localDescription } });
  }

  private teardownPc(): void {
    this.pc?.close();
    this.pc = null;
    this.controlChannel = null;
    this.filesChannel = null;
    this.incoming = null;
    this.handlers.onControlReady?.(false);
    this.handlers.onFilesReady?.(false);
    this.remoteDescSet = false;
    this.pendingCandidates = [];
    this.handlers.onRemoteStream?.(null);
  }

  private send(msg: unknown): void {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    }
  }
}

interface RtcSignal {
  sdp?: RTCSessionDescriptionInit;
  candidate?: RTCIceCandidateInit;
}

export function errorText(code: string): string {
  switch (code) {
    case 'not_found':
      return 'No existe una sesión con ese código. Verifícalo con el usuario.';
    case 'declined':
      return 'El usuario del equipo rechazó la conexión (o nadie la aceptó a tiempo).';
    case 'busy':
      return 'Esa sesión ya tiene un técnico conectado.';
    case 'server_busy':
      return 'El servidor está saturado. Inténtalo en unos minutos.';
    case 'rtc_failed':
      return 'No se pudo establecer la conexión de vídeo (¿firewall/NAT?). El chat sigue activo.';
    case 'ws_error':
      return 'Se perdió la conexión con el servidor.';
    default:
      return 'Ocurrió un error de conexión.';
  }
}
