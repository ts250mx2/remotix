import { useEffect, useState, type FormEvent } from 'react';
import { Link } from 'react-router-dom';
import { api, type Project } from '../api/client';
import { Topbar } from '../components/Topbar';

export function Dashboard() {
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [name, setName] = useState('');
  const [creating, setCreating] = useState(false);

  async function load() {
    const { projects } = await api.get<{ projects: Project[] }>('/api/projects');
    setProjects(projects);
  }

  useEffect(() => { void load(); }, []);

  async function onCreate(e: FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    setCreating(true);
    try {
      await api.post('/api/projects', { name: name.trim() });
      setName('');
      await load();
    } finally {
      setCreating(false);
    }
  }

  return (
    <>
      <Topbar />
      <main className="container">
        <section className="card">
          <h2>Nueva empresa</h2>
          <form onSubmit={onCreate} className="row">
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Nombre de la empresa cliente"
            />
            <button type="submit" disabled={creating}>{creating ? 'Creando…' : 'Crear'}</button>
          </form>
        </section>

        <section className="card">
          <h2>Tus empresas</h2>
          {projects === null ? (
            <p className="muted">Cargando…</p>
          ) : projects.length === 0 ? (
            <p className="muted">Aún no tienes empresas. Crea una arriba.</p>
          ) : (
            <ul className="project-list">
              {projects.map((p) => (
                <li key={p.id}>
                  <Link to={`/projects/${p.id}`} className="project-link">
                    <span className="project-name">{p.name}</span>
                    <span className="mono small">{p.id}</span>
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </section>
      </main>
    </>
  );
}
