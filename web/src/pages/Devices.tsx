import { useEffect, useMemo, useState, type FormEvent } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { api, HttpError, type Device } from '../api/client';
import { Layout } from '../components/Layout';
import { DownloadButton } from '../components/DownloadButton';

function fmtKey(k: string): string {
  return k.length === 9 ? `${k.slice(0, 3)}-${k.slice(3, 6)}-${k.slice(6)}` : k;
}

export function Devices() {
  const [devices, setDevices] = useState<Device[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [keyInput, setKeyInput] = useState('');
  const [adding, setAdding] = useState(false);
  const [query, setQuery] = useState('');
  const [editingNoteId, setEditingNoteId] = useState<string | null>(null);
  const [noteDraft, setNoteDraft] = useState('');
  const [savingNoteId, setSavingNoteId] = useState<string | null>(null);
  const nav = useNavigate();

  async function load() {
    const { devices } = await api.get<{ devices: Device[] }>('/api/devices');
    setDevices(devices);
  }
  useEffect(() => { void load(); }, []);

  // Filtro de la lista: busca en nombre, comentario, sistema, hostname y clave
  // (la clave ignora los guiones: "123456" encuentra "123-456-789").
  const visible = useMemo(() => {
    if (!devices) return null;
    const q = query.trim().toLowerCase();
    if (!q) return devices;
    const qKey = q.replace(/[^0-9a-z]/g, '');
    return devices.filter((d) =>
      d.name.toLowerCase().includes(q)
      || (d.note ?? '').toLowerCase().includes(q)
      || (d.os ?? '').toLowerCase().includes(q)
      || (d.hostname ?? '').toLowerCase().includes(q)
      || (qKey !== '' && d.accessKey.toLowerCase().includes(qKey)),
    );
  }, [devices, query]);

  // Guarda el comentario personal de una PC (vacío = borrarlo) y refresca.
  // El cierre del editor se decide contra el id guardado: si mientras viajaba
  // la petición el usuario abrió el editor de OTRA PC, ese editor no se toca
  // (y su borrador tampoco; `startEditNote` siempre re-inicializa el draft).
  async function saveNote(deviceId: string) {
    setError(null);
    setSavingNoteId(deviceId);
    try {
      await api.patch(`/api/devices/${deviceId}/note`, { note: noteDraft.trim() });
      setEditingNoteId((id) => (id === deviceId ? null : id));
      await load();
    } catch {
      setError('No se pudo guardar el comentario.');
    } finally {
      setSavingNoteId((id) => (id === deviceId ? null : id));
    }
  }

  function startEditNote(d: Device) {
    setEditingNoteId(d.id);
    setNoteDraft(d.note ?? '');
  }

  // Reserva una sesión con el equipo (le ordena compartir) y abre la consola.
  async function connect(d: Device) {
    setError(null);
    setBusy(d.id);
    try {
      const { code, confirm } = await api.post<{ code: string; confirm?: boolean }>(`/api/devices/${d.id}/connect`);
      // `confirm=1` → la consola muestra "esperando que el usuario acepte…".
      nav(`/operador?code=${encodeURIComponent(code)}${confirm ? '&confirm=1' : ''}`);
    } catch (err) {
      if (err instanceof HttpError && err.status === 409) setError(`${d.name} no está en línea.`);
      else if (err instanceof HttpError && err.status === 403) setError(`No tienes acceso a ${d.name}.`);
      else setError('No se pudo iniciar la conexión.');
    } finally {
      setBusy(null);
    }
  }

  // Suscribir una PC a mi cuenta con su clave (self-service). Si nadie la ha
  // reclamado quedo como dueño; si ya tiene dueño, me suscribo (la comparto).
  async function addByKey(e: FormEvent) {
    e.preventDefault();
    const key = keyInput.trim();
    if (!key) return;
    setError(null);
    setAdding(true);
    try {
      await api.post('/api/devices/claim', { accessKey: key });
      setKeyInput('');
      await load();
    } catch (err) {
      setError(err instanceof HttpError && err.status === 404
        ? 'No existe una PC con esa clave.'
        : 'No se pudo agregar la PC.');
    } finally {
      setAdding(false);
    }
  }

  // Dueño → elimina el equipo (para todos). Compartido → se da de baja (solo tú).
  async function removeFromAccount(d: Device) {
    const owner = d.role === 'owner';
    const msg = owner
      ? `¿Eliminar "${d.name}"?\n\nSe borrará el equipo y TODOS los accesos compartidos (para todos los usuarios). No se puede deshacer.`
      : `¿Quitar "${d.name}" de tu cuenta?\n\nDejarás de verlo; el equipo y los demás usuarios no se ven afectados.`;
    if (!confirm(msg)) return;
    setError(null);
    setBusy(d.id);
    try {
      await api.del(owner ? `/api/devices/${d.id}` : `/api/devices/${d.id}/subscription`);
      await load();
    } catch {
      setError(`No se pudo ${owner ? 'eliminar' : 'quitar'} ${d.name}.`);
    } finally {
      setBusy(null);
    }
  }

  return (
    <Layout title="Mis PCs">
      <section className="card">
        <h2>Agregar una PC por clave</h2>
        <p className="muted small">
          Escribe la clave que muestra el programa Remotix en esa PC y quedará en tu cuenta.
        </p>
        <form className="row" onSubmit={addByKey}>
          <input
            value={keyInput}
            onChange={(e) => setKeyInput(e.target.value)}
            placeholder="Clave del equipo (ej. 123-456-789)"
          />
          <button type="submit" disabled={adding}>{adding ? 'Agregando…' : 'Agregar'}</button>
        </form>
        {error && <div className="error">{error}</div>}
        <hr className="sep" />
        <p className="muted small">¿Aún no tienes Remotix en esa PC? Instálalo y te dará su clave:</p>
        <DownloadButton />
      </section>

      <section className="card">
        <h2>Equipos de tu cuenta</h2>
        {devices === null ? (
          <p className="muted">Cargando…</p>
        ) : devices.length === 0 ? (
          <p className="muted">
            Aún no tienes PCs. Agrega una con su clave arriba, o conéctate puntualmente por clave desde la{' '}
            <Link to="/operador">consola</Link>.
          </p>
        ) : (
          <>
            <input
              className="device-search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="🔍 Buscar por nombre, comentario, clave o sistema…"
            />
            {visible !== null && visible.length === 0 ? (
              <p className="muted">
                Sin resultados para «{query.trim()}».{' '}
                <button className="ghost" onClick={() => setQuery('')}>Limpiar búsqueda</button>
              </p>
            ) : (
          <table className="equipos">
            <thead>
              <tr><th>Nombre</th><th>Comentario</th><th>Clave</th><th>Estado</th><th>Sistema</th><th>Versión</th><th>Acceso</th><th></th></tr>
            </thead>
            <tbody>
              {(visible ?? []).map((d) => (
                <tr key={d.id}>
                  <td><Link to={`/devices/${d.id}`}>💻 {d.name}</Link></td>
                  <td className="note-cell">
                    {editingNoteId === d.id ? (
                      <form
                        className="row note-form"
                        onSubmit={(e) => { e.preventDefault(); void saveNote(d.id); }}
                      >
                        <input
                          autoFocus
                          value={noteDraft}
                          maxLength={500}
                          placeholder="Escribe un comentario…"
                          onChange={(e) => setNoteDraft(e.target.value)}
                          onKeyDown={(e) => { if (e.key === 'Escape') setEditingNoteId(null); }}
                        />
                        <button type="submit" disabled={savingNoteId === d.id}>{savingNoteId === d.id ? '…' : 'Guardar'}</button>
                        <button type="button" className="ghost" onClick={() => setEditingNoteId(null)}>✕</button>
                      </form>
                    ) : (
                      <button
                        className="ghost notebtn"
                        title="Comentario personal (solo lo ves tú) — clic para editar"
                        onClick={() => startEditNote(d)}
                      >
                        {d.note ? d.note : '✎ añadir'}
                      </button>
                    )}
                  </td>
                  <td className="mono small">{fmtKey(d.accessKey)}</td>
                  <td><span className={`pcstate ${d.online ? 'on' : ''}`}>{d.online ? 'En línea' : 'Desconectado'}</span></td>
                  <td>{d.os ?? '—'}</td>
                  <td className="mono small">{d.agentVersion ? `v${d.agentVersion}` : '—'}</td>
                  <td><span className="badge small">{d.role === 'owner' ? 'dueño' : 'compartido'}</span></td>
                  <td>
                    <div className="row-actions">
                      <button disabled={!d.online || busy === d.id} onClick={() => connect(d)}>
                        {busy === d.id ? 'Conectando…' : 'Conectar'}
                      </button>
                      <button
                        className="danger ghost iconbtn"
                        title={d.role === 'owner' ? 'Eliminar equipo (borrar de todas las cuentas)' : 'Quitar de mi cuenta'}
                        disabled={busy === d.id}
                        onClick={() => removeFromAccount(d)}
                      >
                        {d.role === 'owner' ? '🗑' : '✕'}
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
            )}
          </>
        )}
      </section>
    </Layout>
  );
}
