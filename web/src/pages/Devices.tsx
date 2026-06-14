import { useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { api, HttpError, type Device } from '../api/client';
import { Topbar } from '../components/Topbar';

function fmtKey(k: string): string {
  return k.length === 9 ? `${k.slice(0, 3)}-${k.slice(3, 6)}-${k.slice(6)}` : k;
}

export function Devices() {
  const [devices, setDevices] = useState<Device[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
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

  return (
    <>
      <Topbar />
      <main className="container">
        <section className="card">
          <h2>Mis PCs</h2>
          <p className="muted small">
            Equipos a los que tienes acceso. Para añadir uno nuevo: <a href="/download/remotix.exe" download>descarga Remotix</a>,
            instálalo en esa PC, inicia sesión con tu cuenta en el programa y aparecerá aquí. También
            puedes conectarte por clave desde la <Link to="/operador">consola</Link>.
          </p>
          {error && <div className="error">{error}</div>}
          {devices === null ? (
            <p className="muted">Cargando…</p>
          ) : devices.length === 0 ? (
            <p className="muted">Aún no tienes PCs. Inicia sesión en el programa Remotix de un equipo para reclamarlo.</p>
          ) : (
            <table className="equipos">
              <thead>
                <tr><th>Nombre</th><th>Clave</th><th>Estado</th><th>Sistema</th><th>Acceso</th><th></th></tr>
              </thead>
              <tbody>
                {devices.map((d) => (
                  <tr key={d.id}>
                    <td><Link to={`/devices/${d.id}`}>💻 {d.name}</Link></td>
                    <td className="mono small">{fmtKey(d.accessKey)}</td>
                    <td><span className={`pcstate ${d.online ? 'on' : ''}`}>{d.online ? 'En línea' : 'Desconectado'}</span></td>
                    <td>{d.os ?? '—'}</td>
                    <td><span className="badge small">{d.role === 'owner' ? 'dueño' : 'compartido'}</span></td>
                    <td>
                      <button disabled={!d.online || busy === d.id} onClick={() => connect(d)}>
                        {busy === d.id ? 'Conectando…' : 'Conectar'}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
      </main>
    </>
  );
}
