import { useEffect, useState, type FormEvent } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { api, HttpError, type Device } from '../api/client';
import { Layout } from '../components/Layout';

function fmtKey(k: string): string {
  return k.length === 9 ? `${k.slice(0, 3)}-${k.slice(3, 6)}-${k.slice(6)}` : k;
}

export function Devices() {
  const [devices, setDevices] = useState<Device[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [keyInput, setKeyInput] = useState('');
  const [adding, setAdding] = useState(false);
  const nav = useNavigate();

  async function load() {
    const { devices } = await api.get<{ devices: Device[] }>('/api/devices');
    setDevices(devices);
  }
  useEffect(() => { void load(); }, []);

  // Reserva una sesión con el equipo (le ordena compartir) y abre la consola.
  async function connect(d: Device) {
    setError(null);
    setBusy(d.id);
    try {
      const { code } = await api.post<{ code: string }>(`/api/devices/${d.id}/connect`);
      nav(`/operador?code=${encodeURIComponent(code)}`);
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
          ¿Aún no lo tienes instalado allí? <a href="/download/RemotixSetup.exe" download>Descarga el instalador de Remotix</a>.
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
          <table className="equipos">
            <thead>
              <tr><th>Nombre</th><th>Clave</th><th>Estado</th><th>Sistema</th><th>Versión</th><th>Acceso</th><th></th></tr>
            </thead>
            <tbody>
              {devices.map((d) => (
                <tr key={d.id}>
                  <td><Link to={`/devices/${d.id}`}>💻 {d.name}</Link></td>
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
      </section>
    </Layout>
  );
}
