import { useMemo, useRef, useState, type ChangeEvent } from 'react';
import type { ConnectionHandlers, FileProgress, ReceivedFile } from './connection';

/** Estado de transferencias para usar en una página de helpdesk. Devuelve los
 * handlers a fusionar en los `ConnectionHandlers` de la SupportConnection. */
export function useFileTransfers() {
  const [filesReady, setFilesReady] = useState(false);
  const [transfers, setTransfers] = useState<FileProgress[]>([]);
  const [received, setReceived] = useState<ReceivedFile[]>([]);
  const [requested, setRequested] = useState(false);

  const fileHandlers = useMemo<Partial<ConnectionHandlers>>(
    () => ({
      onFilesReady: (r) => {
        setFilesReady(r);
        if (!r) setRequested(false);
      },
      onFileProgress: (p) =>
        setTransfers((prev) => {
          const key = `${p.dir}-${p.id}`;
          const next = prev.filter((t) => `${t.dir}-${t.id}` !== key);
          next.push(p);
          return next.slice(-6);
        }),
      onFileReceived: (f) => setReceived((prev) => [...prev, f]),
      onFileRequested: () => setRequested(true),
    }),
    [],
  );

  return { filesReady, transfers, received, requested, clearRequested: () => setRequested(false), fileHandlers };
}

interface FilePanelProps {
  ready: boolean;
  transfers: FileProgress[];
  received: ReceivedFile[];
  requested?: boolean;
  onSend: (file: File) => void;
  onRequest?: () => void;
  onSent?: () => void;
}

export function FilePanel({ ready, transfers, received, requested, onSend, onRequest, onSent }: FilePanelProps) {
  const inputRef = useRef<HTMLInputElement>(null);

  function pick(e: ChangeEvent<HTMLInputElement>) {
    const files = e.target.files;
    if (files) {
      for (const f of Array.from(files)) onSend(f);
      onSent?.();
    }
    e.target.value = '';
  }

  return (
    <div className="files">
      <div className="files-head">
        <h2>Archivos</h2>
        {!ready && <span className="muted small">No disponible (sin conexión P2P).</span>}
      </div>

      {requested && ready && (
        <div className="files-req">El otro lado te pidió un archivo. Pulsa “Enviar archivo”.</div>
      )}

      <div className="row">
        <input ref={inputRef} type="file" multiple hidden onChange={pick} />
        <button disabled={!ready} onClick={() => inputRef.current?.click()}>
          Enviar archivo
        </button>
        {onRequest && (
          <button className="ghost" disabled={!ready} onClick={onRequest}>
            Pedir archivo
          </button>
        )}
      </div>

      {transfers.length > 0 && (
        <ul className="file-list">
          {transfers.map((t) => {
            const pct = t.total ? Math.round((t.transferred / t.total) * 100) : 0;
            return (
              <li key={`${t.dir}-${t.id}`} className="file-row">
                <span className="file-name">
                  {t.dir === 'out' ? '↑' : '↓'} {t.name}
                </span>
                <span className="muted small">{pct}%</span>
                <div className="file-bar">
                  <div className="file-bar-fill" style={{ width: `${pct}%` }} />
                </div>
              </li>
            );
          })}
        </ul>
      )}

      {received.length > 0 && (
        <div className="files-received">
          <p className="muted small">Recibidos:</p>
          <ul className="file-list">
            {received.map((f, i) => (
              <li key={i} className="file-row">
                <a href={URL.createObjectURL(f.blob)} download={f.name}>
                  ⬇ {f.name}
                </a>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
