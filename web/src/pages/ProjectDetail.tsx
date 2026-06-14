import { useEffect, useState, type FormEvent } from 'react';
import { Link, useParams } from 'react-router-dom';
import { api, HttpError, type Equipo, type Project } from '../api/client';
import { Topbar } from '../components/Topbar';

interface ProjectResp { project: Project; role: 'admin' | 'tecnico' | 'usuario' | 'operator'; }
interface Member { principalId: string; role: string; }
interface Directory { id: string; email: string; name: string; }
interface Group { id: string; name: string; }
interface RosterEntry { id: string; kind: 'user' | 'pc'; online: boolean; currentUserId?: string | null; }

export function ProjectDetail() {
  const { id } = useParams<{ id: string }>();
  const [project, setProject] = useState<Project | null>(null);
  const [role, setRole] = useState<string | null>(null);
  const [equipos, setEquipos] = useState<Equipo[] | null>(null);
  const [members, setMembers] = useState<Member[]>([]);
  const [dir, setDir] = useState<Record<string, Directory>>({});
  const [groups, setGroups] = useState<Group[]>([]);
  const [roster, setRoster] = useState<RosterEntry[]>([]);
  const [groupToAdd, setGroupToAdd] = useState('');
  const [tecEmail, setTecEmail] = useState('');
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  async function loadAll() {
    if (!id) return;
    setError(null);
    try {
      const projResp = await api.get<ProjectResp>(`/api/projects/${id}`);
      setProject(projResp.project);
      setRole(projResp.role);
      const eqResp = await api.get<{ equipos: Equipo[] }>(`/api/equipos?project_id=${id}`);
      setEquipos(eqResp.equipos);
      const memResp = await api.get<{ members: Member[] }>(`/api/projects/${id}/members`);
      setMembers(memResp.members);
      const usersResp = await api.get<{ users: Directory[] }>(`/api/users`);
      setDir(Object.fromEntries(usersResp.users.map((u) => [u.id, u])));
      const groupsResp = await api.get<{ groups: Group[] }>(`/api/groups`);
      setGroups(groupsResp.groups);
      const rosterResp = await api.get<{ roster: RosterEntry[] }>(`/api/chat/empresas/${id}/roster`);
      setRoster(rosterResp.roster);
    } catch {
      setError('No se pudo cargar la empresa');
    }
  }

  useEffect(() => { void loadAll(); }, [id]);

  function copyUuid() {
    if (!project) return;
    void navigator.clipboard?.writeText(project.id);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  async function addTecnico(e: FormEvent) {
    e.preventDefault();
    if (!id || !tecEmail.trim()) return;
    setNotice(null);
    try {
      const { user } = await api.get<{ user: Directory }>(`/api/users/lookup?email=${encodeURIComponent(tecEmail.trim())}`);
      await api.post(`/api/projects/${id}/members`, { principalId: user.id, role: 'tecnico' });
      setTecEmail('');
      await loadAll();
    } catch (err) {
      setNotice(err instanceof HttpError && err.status === 404
        ? 'No existe un usuario con ese email (debe registrarse primero).'
        : 'No se pudo agregar el técnico.');
    }
  }

  async function removeMember(principalId: string) {
    if (!id) return;
    if (!confirm('¿Quitar a este técnico de la empresa?')) return;
    await api.del(`/api/projects/${id}/members/${principalId}`);
    await loadAll();
  }

  async function assignGroup() {
    if (!id || !groupToAdd) return;
    await api.post(`/api/projects/${id}/members`, { principalId: groupToAdd, role: 'tecnico' });
    setGroupToAdd('');
    await loadAll();
  }

  async function removeEquipo(equipoId: string) {
    if (!confirm(`¿Eliminar el PC ${equipoId}? No se puede deshacer.`)) return;
    await api.del(`/api/equipos/${equipoId}`);
    await loadAll();
  }

  if (error) return <><Topbar /><main className="container"><div className="error">{error}</div></main></>;
  if (!project) return <><Topbar /><main className="container"><p className="muted">Cargando…</p></main></>;

  const isAdmin = role === 'admin';
  const tecnicos = members.filter((m) => m.principalId.startsWith('us_') && (m.role === 'tecnico' || m.role === 'admin' || m.role === 'operator'));
  const assignedGroupIds = new Set(members.filter((m) => m.principalId.startsWith('gp_')).map((m) => m.principalId));
  const assignedGroups = groups.filter((g) => assignedGroupIds.has(g.id));
  const availableGroups = groups.filter((g) => !assignedGroupIds.has(g.id));
  const groupName = (id: string) => groups.find((g) => g.id === id)?.name ?? id;
  const onlineMap: Record<string, boolean> = Object.fromEntries(roster.map((r) => [r.id, r.online]));
  const boundUserName = (pcId: string) => {
    const r = roster.find((x) => x.id === pcId);
    return r?.currentUserId ? dir[r.currentUserId]?.name ?? null : null;
  };

  return (
    <>
      <Topbar />
      <main className="container">
        <section className="project-header">
          <div>
            <h1>{project.name}</h1>
            <p className="mono small muted">{project.id}</p>
          </div>
          <div className="row">
            <Link className="badge" to="/chat">Abrir chat →</Link>
            <span className="badge">{role}</span>
          </div>
        </section>

        {/* UUID para conectar PCs */}
        <section className="card">
          <h2>Conectar un PC a esta empresa</h2>
          <p className="muted small">
            Instala el agente (o abre la web del chat) en el equipo y, cuando pida el <strong>UUID del proyecto</strong>,
            pega este valor. El PC quedará ligado a esta empresa con el nombre que elijas.
          </p>
          <div className="uuid-box">
            <code className="uuid">{project.id}</code>
            <button onClick={copyUuid}>{copied ? '¡Copiado!' : 'Copiar UUID'}</button>
          </div>
        </section>

        {/* Técnicos */}
        <section className="card">
          <h2>Técnicos ({tecnicos.length})</h2>
          <p className="muted small">Los técnicos pueden chatear en todas las empresas asignadas y controlar los PCs remotamente.</p>
          {isAdmin && (
            <form className="row" onSubmit={addTecnico}>
              <input type="email" value={tecEmail} onChange={(e) => setTecEmail(e.target.value)} placeholder="email del técnico (ya registrado)" />
              <button type="submit">Agregar técnico</button>
            </form>
          )}
          {notice && <div className="error">{notice}</div>}
          <ul className="group-list">
            {tecnicos.map((m) => (
              <li key={m.principalId}>
                <span>{dir[m.principalId]?.name ?? m.principalId} <span className="muted small">{dir[m.principalId]?.email}</span></span>
                <span className="row">
                  <span className="badge small">{m.role}</span>
                  {isAdmin && m.principalId !== project.ownerId && (
                    <button className="danger ghost" onClick={() => removeMember(m.principalId)}>Quitar</button>
                  )}
                </span>
              </li>
            ))}
            {tecnicos.length === 0 && <li className="muted small">Aún no hay técnicos asignados.</li>}
          </ul>
        </section>

        {/* Grupos de técnicos */}
        <section className="card">
          <h2>Grupos de técnicos ({assignedGroups.length})</h2>
          <p className="muted small">
            Asigna un grupo y todos sus técnicos tendrán acceso a este proyecto.{' '}
            <Link to="/groups">Gestionar grupos →</Link>
          </p>
          {isAdmin && (
            <div className="row">
              <select value={groupToAdd} onChange={(e) => setGroupToAdd(e.target.value)}>
                <option value="">— elige un grupo —</option>
                {availableGroups.map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}
              </select>
              <button onClick={assignGroup} disabled={!groupToAdd}>Asignar grupo</button>
            </div>
          )}
          <ul className="group-list">
            {assignedGroups.map((g) => (
              <li key={g.id}>
                <span>👥 {g.name}</span>
                {isAdmin && <button className="danger ghost" onClick={() => removeMember(g.id)}>Quitar</button>}
              </li>
            ))}
            {assignedGroups.length === 0 && <li className="muted small">Ningún grupo asignado.</li>}
          </ul>
        </section>

        {/* PCs */}
        <section className="card">
          <h2>PCs ({equipos?.length ?? 0})</h2>
          {equipos === null ? (
            <p className="muted">Cargando…</p>
          ) : equipos.length === 0 ? (
            <p className="muted">Aún no hay PCs. Conéctalos con el UUID de arriba.</p>
          ) : (
            <table className="equipos">
              <thead>
                <tr><th>Nombre</th><th>Estado</th><th>Usuario</th><th>Sistema</th><th>Última vez</th><th></th></tr>
              </thead>
              <tbody>
                {equipos.map((e) => (
                  <tr key={e.id}>
                    <td>💻 {e.name}</td>
                    <td><span className={`pcstate ${onlineMap[e.id] ? 'on' : ''}`}>{onlineMap[e.id] ? 'Conectado' : 'Desconectado'}</span></td>
                    <td>{boundUserName(e.id) ?? <span className="muted">—</span>}</td>
                    <td>{e.os ?? '—'}</td>
                    <td>{e.lastSeenAt ? new Date(e.lastSeenAt).toLocaleString() : 'nunca'}</td>
                    <td>{isAdmin && <button className="danger ghost" onClick={() => removeEquipo(e.id)}>Eliminar</button>}</td>
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
