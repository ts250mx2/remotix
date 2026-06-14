// Videollamada en malla (mesh): cada participante abre un RTCPeerConnection con
// cada otro. La señalización (call-*) viaja por el WebSocket del chat. Regla
// anti-glare: el que ENTRA más tarde oferta a los que ya estaban.

const DEFAULT_ICE: RTCIceServer[] = [
  { urls: 'stun:stun.l.google.com:19302' },
  { urls: 'stun:stun1.l.google.com:19302' },
];

export interface CallHandlers {
  onLocalStream?: (s: MediaStream | null) => void;
  onRemoteStream?: (peerId: string, s: MediaStream | null) => void;
  onActive?: (active: boolean) => void;
  onError?: (msg: string) => void;
}

type Send = (obj: unknown) => void;

export class CallSession {
  private pcs = new Map<string, RTCPeerConnection>();
  private pending = new Map<string, RTCIceCandidateInit[]>();
  private remoteSet = new Set<string>();
  private local: MediaStream | null = null;
  private ice: RTCIceServer[] = DEFAULT_ICE;
  private channelId: string | null = null;
  private active = false;

  constructor(private send: Send, private handlers: CallHandlers) {}

  isActive() { return this.active; }

  async start(channelId: string, withVideo = true): Promise<void> {
    if (this.active) return;
    await this.loadIce();
    try {
      this.local = await navigator.mediaDevices.getUserMedia({ video: withVideo, audio: true });
    } catch {
      this.handlers.onError?.('No se pudo acceder a la cámara/micrófono.');
      return;
    }
    this.channelId = channelId;
    this.active = true;
    this.handlers.onLocalStream?.(this.local);
    this.handlers.onActive?.(true);
    this.send({ type: 'call-join', channelId });
  }

  /** Procesa un mensaje call-* del WebSocket del chat. */
  handle(m: { type: string; peers?: string[]; peerId?: string; from?: string; payload?: RtcSignal }): void {
    if (!this.active) return;
    switch (m.type) {
      case 'call-peers':
        for (const p of m.peers ?? []) void this.offerTo(p);
        break;
      case 'call-peer-joined':
        // El recién llegado nos ofertará; esperamos.
        break;
      case 'call-peer-left':
        if (m.peerId) this.closePeer(m.peerId);
        break;
      case 'call-signal':
        if (m.from) void this.onSignal(m.from, m.payload ?? {});
        break;
    }
  }

  toggleMic(on: boolean) { this.local?.getAudioTracks().forEach((t) => (t.enabled = on)); }
  toggleCam(on: boolean) { this.local?.getVideoTracks().forEach((t) => (t.enabled = on)); }

  leave(): void {
    if (this.channelId) this.send({ type: 'call-leave', channelId: this.channelId });
    for (const [id, pc] of this.pcs) { pc.close(); this.handlers.onRemoteStream?.(id, null); }
    this.pcs.clear();
    this.pending.clear();
    this.remoteSet.clear();
    this.local?.getTracks().forEach((t) => t.stop());
    this.local = null;
    this.active = false;
    this.channelId = null;
    this.handlers.onLocalStream?.(null);
    this.handlers.onActive?.(false);
  }

  private async loadIce(): Promise<void> {
    try {
      const res = await fetch('/api/turn-credentials', { credentials: 'omit' });
      if (res.ok) {
        const data = (await res.json()) as { iceServers?: RTCIceServer[] };
        if (Array.isArray(data.iceServers) && data.iceServers.length) this.ice = data.iceServers;
      }
    } catch { /* STUN por defecto */ }
  }

  private peer(peerId: string): RTCPeerConnection {
    let pc = this.pcs.get(peerId);
    if (pc) return pc;
    pc = new RTCPeerConnection({ iceServers: this.ice });
    this.local?.getTracks().forEach((t) => pc!.addTrack(t, this.local!));
    pc.onicecandidate = (e) => {
      if (e.candidate && this.channelId) {
        this.send({ type: 'call-signal', channelId: this.channelId, to: peerId, payload: { candidate: e.candidate.toJSON() } });
      }
    };
    pc.ontrack = (e) => {
      if (!this.remoteSet.has(peerId)) { this.remoteSet.add(peerId); }
      this.handlers.onRemoteStream?.(peerId, e.streams[0] ?? null);
    };
    pc.onconnectionstatechange = () => {
      if (pc!.connectionState === 'failed' || pc!.connectionState === 'closed') this.closePeer(peerId);
    };
    this.pcs.set(peerId, pc);
    return pc;
  }

  private async offerTo(peerId: string): Promise<void> {
    const pc = this.peer(peerId);
    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    if (this.channelId) this.send({ type: 'call-signal', channelId: this.channelId, to: peerId, payload: { sdp: pc.localDescription } });
  }

  private async onSignal(from: string, payload: RtcSignal): Promise<void> {
    const pc = this.peer(from);
    if (payload.sdp) {
      await pc.setRemoteDescription(payload.sdp);
      await this.flush(from, pc);
      if (payload.sdp.type === 'offer') {
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        if (this.channelId) this.send({ type: 'call-signal', channelId: this.channelId, to: from, payload: { sdp: pc.localDescription } });
      }
    } else if (payload.candidate) {
      if (pc.remoteDescription) {
        try { await pc.addIceCandidate(payload.candidate); } catch { /* noop */ }
      } else {
        const q = this.pending.get(from) ?? [];
        q.push(payload.candidate);
        this.pending.set(from, q);
      }
    }
  }

  private async flush(peerId: string, pc: RTCPeerConnection): Promise<void> {
    const q = this.pending.get(peerId);
    if (!q) return;
    this.pending.delete(peerId);
    for (const c of q) { try { await pc.addIceCandidate(c); } catch { /* noop */ } }
  }

  private closePeer(peerId: string): void {
    const pc = this.pcs.get(peerId);
    if (pc) { pc.close(); this.pcs.delete(peerId); }
    this.pending.delete(peerId);
    this.remoteSet.delete(peerId);
    this.handlers.onRemoteStream?.(peerId, null);
  }
}

interface RtcSignal {
  sdp?: RTCSessionDescriptionInit;
  candidate?: RTCIceCandidateInit;
}
