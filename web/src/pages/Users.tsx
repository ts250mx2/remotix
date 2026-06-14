import { useEffect, useState } from 'react';
import { api, type User } from '../api/client';
import { Topbar } from '../components/Topbar';

export function Users() {
  const [users, setUsers] = useState<User[] | null>(null);

  useEffect(() => {
    void api.get<{ users: User[] }>('/api/users').then((r) => setUsers(r.users));
  }, []);

  return (
    <>
      <Topbar />
      <main className="container">
        <section className="card">
          <h2>Usuarios</h2>
          <p className="muted small">
            Personas con cuenta. Para añadir a alguien, pídele que se registre en{' '}
            <a href="/register">/register</a>; luego podrás concederle acceso a tus PCs.
          </p>
          {users === null ? (
            <p className="muted">Cargando…</p>
          ) : (
            <ul className="group-list">
              {users.map((u) => (
                <li key={u.id}>
                  <span>👤 {u.name} <span className="muted small">{u.email}</span></span>
                  <span className="mono small">{u.id}</span>
                </li>
              ))}
            </ul>
          )}
        </section>
      </main>
    </>
  );
}
