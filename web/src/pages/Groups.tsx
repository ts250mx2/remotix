import { useEffect, useState, type FormEvent } from 'react';
import { api, HttpError } from '../api/client';
import { Layout } from '../components/Layout';

interface Group { id: string; name: string; }
interface Member { userId: string; email: string | null; name: string | null; }

export function Groups() {
  const [groups, setGroups] = useState<Group[] | null>(null);
  const [name, setName] = useState('');
  const [creating, setCreating] = useState(false);
  const [selected, setSelected] = useState<Group | null>(null);
  const [members, setMembers] = useState<Member[]>([]);
  const [memEmail, setMemEmail] = useState('');
  const [notice, setNotice] = useState<string | null>(null);

  async function load() {
    const { groups } = await api.get<{ groups: Group[] }>('/api/groups');
    setGroups(groups);
  }
  useEffect(() => { void load(); }, []);

  async function loadMembers(g: Group) {
    setSelected(g);
    setNotice(null);
    const { members } = await api.get<{ members: Member[] }>(`/api/groups/${g.id}/members`);
    setMembers(members);
  }

  async function onCreate(e: FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    setCreating(true);
    try {
      const { group } = await api.post<{ group: Group }>('/api/groups', { name: name.trim() });
      setName('');
      await load();
      await loadMembers(group);
    } finally {
      setCreating(false);
    }
  }

  async function addMember(e: FormEvent) {
    e.preventDefault();
    if (!selected || !memEmail.trim()) return;
    setNotice(null);
    try {
      const { user } = await api.get<{ user: { id: string } }>(`/api/users/lookup?email=${encodeURIComponent(memEmail.trim())}`);
      await api.post(`/api/groups/${selected.id}/members`, { userId: user.id });
      setMemEmail('');
      await loadMembers(selected);
    } catch (err) {
      setNotice(err instanceof HttpError && err.status === 404
        ? 'No existe un usuario con ese email (debe registrarse primero).'
        : 'No se pudo agregar al grupo.');
    }
  }

  async function removeMember(userId: string) {
    if (!selected) return;
    await api.del(`/api/groups/${selected.id}/members/${userId}`);
    await loadMembers(selected);
  }

  return (
    <Layout title="Grupos">
        <section className="card">
          <h2>Nuevo grupo</h2>
          <p className="muted small">Agrupa usuarios para darles acceso a varias PCs de una vez.</p>
          <form onSubmit={onCreate} className="row">
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Nombre del grupo (ej. soporte-méxico)" />
            <button type="submit" disabled={creating}>{creating ? 'Creando…' : 'Crear'}</button>
          </form>
        </section>

        <div className="groups-layout">
          <section className="card">
            <h2>Grupos</h2>
            {groups === null ? (
              <p className="muted">Cargando…</p>
            ) : groups.length === 0 ? (
              <p className="muted">Aún no hay grupos.</p>
            ) : (
              <ul className="group-list">
                {groups.map((g) => (
                  <li key={g.id} className={selected?.id === g.id ? 'sel' : ''}>
                    <button className="link-row" onClick={() => loadMembers(g)}>{g.name}</button>
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section className="card">
            {!selected ? (
              <p className="muted">Selecciona un grupo para ver y gestionar sus técnicos.</p>
            ) : (
              <>
                <h2>{selected.name} · técnicos ({members.length})</h2>
                <form className="row" onSubmit={addMember}>
                  <input type="email" value={memEmail} onChange={(e) => setMemEmail(e.target.value)} placeholder="email del técnico (ya registrado)" />
                  <button type="submit">Agregar</button>
                </form>
                {notice && <div className="error">{notice}</div>}
                <ul className="group-list">
                  {members.map((m) => (
                    <li key={m.userId}>
                      <span>{m.name ?? m.userId} <span className="muted small">{m.email}</span></span>
                      <button className="danger ghost" onClick={() => removeMember(m.userId)}>Quitar</button>
                    </li>
                  ))}
                  {members.length === 0 && <li className="muted small">Grupo vacío.</li>}
                </ul>
              </>
            )}
          </section>
        </div>
    </Layout>
  );
}
