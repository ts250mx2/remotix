import { useEffect, useRef } from 'react';

export interface RemoteTile {
  id: string;
  name: string;
  stream: MediaStream | null;
}

interface CallViewProps {
  localStream: MediaStream | null;
  remotes: RemoteTile[];
  micOn: boolean;
  camOn: boolean;
  onToggleMic: () => void;
  onToggleCam: () => void;
  onLeave: () => void;
  selfName: string;
}

function Video({ stream, muted }: { stream: MediaStream | null; muted?: boolean }) {
  const ref = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.srcObject = stream;
  }, [stream]);
  // eslint-disable-next-line jsx-a11y/media-has-caption
  return <video ref={ref} autoPlay playsInline muted={muted} className="call-video" />;
}

export function CallView({ localStream, remotes, micOn, camOn, onToggleMic, onToggleCam, onLeave, selfName }: CallViewProps) {
  return (
    <div className="call-view">
      <div className="call-grid">
        <div className="call-tile">
          <Video stream={localStream} muted />
          <span className="call-name">{selfName} (tú){!camOn && ' · cámara off'}</span>
        </div>
        {remotes.map((r) => (
          <div className="call-tile" key={r.id}>
            <Video stream={r.stream} />
            <span className="call-name">{r.name}</span>
          </div>
        ))}
      </div>
      <div className="call-controls">
        <button className={micOn ? '' : 'danger'} onClick={onToggleMic}>{micOn ? '🎤' : '🔇'}</button>
        <button className={camOn ? '' : 'danger'} onClick={onToggleCam}>{camOn ? '📹' : '🚫'}</button>
        <button className="danger" onClick={onLeave}>Colgar</button>
      </div>
    </div>
  );
}
