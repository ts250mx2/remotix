import { useEffect, useState, type FormEvent } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { api, HttpError, type Device, type DeviceGrant, type User } from '../api/client';
import { Topbar } from '../components/Topbar';

interface Group { id: string; name: string; }

function fmtKey(k: string): string {
  return k.length === 9 ? `${k.slice(0, 3)}-${k.slice(3, 6)}-${k.slice(6)}` : k;
}

export function DeviceDetail() {
  const { id } = useParams<{ id: string }>();
  const nav = useNavigate();
  const [device, setDevice] = useState<Device | null>(null);
  const [grants, setGrants] = useState<DeviceGrant[]>([]);
  const [groups, setGroups] = useState<Group[]>([]);
  const [name, setName] = useState('');
  const [email, setEmail] = useState('');
  const [groupToAdd, setGroupToAdd] = useState('');
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function loadAll() {
    if (!id) return;
    setError(null);
    try {
      const { devices } = await api.get<{ devices: Device[] }>('/api/devices');
      const d = devices.find((x) => x.id === id) ?? null;
      setDevice(d);
      setName(d?.name ?? '');
      if (d?.role === 'owner') {
        const acc = await api.get<{ owner: string | null; grants: DeviceGrant[] }>(`/api/devices/${id}/access`);
        setGrants(acc.grants);
        const g = await api.get<{ groups: Group[] }>('/api/groups');
        setGroups(g.groups);
      }
    } catch {
      setError('No se pudo cargar el equipo.');
    }
  }
  useEffect(() => { void loadAll(); }, [id]);

  async function rename(e: FormEvent) {
    e.preventDefault();
    if (!id || !name.trim()) return;
    await api.patch(`/api/devices/${id}`, { name: name.trim() });
    setNotice('Nombre actualizado.');
    await loadAll();
  }

  async function removeDevice() {
    if (!id) return;
    if (!confirm(`¿Eliminar este equipo (${device?.name})? Útil para borrar duplicados. No se puede deshacer.`)) return;
    await api.del(`/api/devices/${id}`);
    nav('/');
  }

  async function addUser(e: FormEvent) {
    e.preventDefault();
    if (!id || !email.trim()) return;
    setNotice(null);
    try {
      const { user } = await api.get<{ user: User }>(`/api/users/lookup?email=${encodeURIComponent(email.trim())}`);
      await api.post(`/api/devices/${id}/access`, { principalId: user.id });
      setEmail('');
      await loadAll();
    } catch (err) {
      setNotice(err instanceof HttpError && err.status === 404
        ? 'No existe un usuario con ese email (debe registrarse primero).'
        : 'No se pudo conceder el acceso.');
    }
  }

  async function addGroup() {
    if (!id || !groupToAdd) return;
    await api.post(`/api/devices/${id}/access`, { principalId: groupToAdd });
    setGroupToAdd('');
    await loadAll();
  }

  async function revoke(principalId: string) {
    if (!id) return;
    if (!confirm('¿Revocar este acceso?')) return;
    await api.del(`/api/devices/${id}/access/${principalId}`);
    await loadAll();
  }

  if (error) return <><Topbar /><main className="container"><div className="error">{error}</div></main></>;
  if (!device) return <><Topbar /><main className="container"><p className="muted">Cargando…</p></main></>;

  const isOwner = device.role === 'owner';
  const grantedGroupIds = new Set(grants.filter((g) => g.kind === 'group').map((g) => g.principalId));
  const availableGroups = groups.filter((g) => !grantedGroupIds.has(g.id));

  return (
    <>
      <Topbar />
      <main className="container">
        <section className="project-header">
          <div>
            <h1>💻 {device.name}</h1>
            <p className="mono small muted">{fmtKey(device.accessKey)}</p>
          </div>
          <div className="row">
            <span className={`pcstate ${device.online ? 'on' : ''}`}>{device.online ? 'En línea' : 'Desconectado'}</span>
            <span className="badge">{isOwner ? 'dueño' : 'compartido'}</span>
            <Link className="badge" to="/">← Mis PCs</Link>
          </div>
        </section>

        <section className="card">
          <h2>Datos</h2>
          <p className="muted small">Sistema: {device.os ?? '—'} · Host: {device.hostname ?? '—'}</p>
          {isOwner && (
            <form className="row" onSubmit={rename}>
              <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Nombre del equipo" />
              <button type="submit">Renombrar</button>
            </form>
          )}
          {notice && <div className="muted small">{notice}</div>}
          {isOwner && (
            <>
              <hr className="sep" />
              <button className="danger ghost" onClick={removeDevice}>Eliminar este equipo</button>
            </>
          )}
        </section>

        {isOwner ? (
          <section className="card">
            <h2>Quién puede acceder ({grants.length})</h2>
            <p className="muted small">
              Concede acceso a usuarios o grupos. Solo ellos (y tú) podrán conectarse a este equipo.{' '}
              <Link to="/groups">Gestionar grupos →</Link>
            </p>
            <form className="row" onSubmit={addUser}>
              <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="email del usuario (ya registrado)" />
              <button type="submit">Conceder a usuario</button>
            </form>
            <div className="row">
              <select value={groupToAdd} onChange={(e) => setGroupToAdd(e.target.value)}>
                <option value="">— elige un grupo —</option>
                {availableGroups.map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}
              </select>
              <button onClick={addGroup} disabled={!groupToAdd}>Conceder a grupo</button>
            </div>
            <ul className="group-list">
              {grants.map((g) => (
                <li key={g.principalId}>
                  <span>{g.kind === 'group' ? '👥' : '👤'} {g.label} {g.name && <span className="muted small">{g.name}</span>}</span>
                  <button className="danger ghost" onClick={() => revoke(g.principalId)}>Revocar</button>
                </li>
              ))}
              {grants.length === 0 && <li className="muted small">Solo tú tienes acceso.</li>}
            </ul>
          </section>
        ) : (
          <section className="card">
            <p className="muted">Este equipo se comparte contigo. El dueño gestiona los accesos.</p>
          </section>
        )}
      </main>
    </>
  );
}
