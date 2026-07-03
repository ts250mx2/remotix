import type { ReactNode } from 'react';
import { Link, NavLink, useNavigate } from 'react-router-dom';
import { useAuth } from '../auth/AuthContext';

// Navegación del portal (menú lateral). `end` marca la ruta exacta (para que "/"
// no quede activa en todas las subrutas).
const NAV = [
  { to: '/', label: 'Mis PCs', icon: '🖥️', end: true },
  { to: '/groups', label: 'Grupos', icon: '👥' },
  { to: '/users', label: 'Usuarios', icon: '👤' },
  { to: '/operador', label: 'Conectar por clave', icon: '🔑' },
];

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return '?';
  const first = parts[0]![0] ?? '';
  const last = parts.length > 1 ? parts[parts.length - 1]![0] ?? '' : '';
  return (first + last).toUpperCase();
}

/** Estructura del portal: menú lateral fijo + área de contenido. `title` pinta la
 *  cabecera de la página; `actions` va a la derecha de esa cabecera. */
export function Layout({
  children,
  title,
  actions,
}: {
  children: ReactNode;
  title?: string;
  actions?: ReactNode;
}) {
  const { user, logout } = useAuth();
  const nav = useNavigate();

  async function handleLogout() {
    await logout();
    nav('/login');
  }

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <Link to="/" className="sb-brand">
          <span className="sb-logo" />
          Remotix
        </Link>

        <nav className="sb-nav">
          {NAV.map((n) => (
            <NavLink
              key={n.to}
              to={n.to}
              end={n.end}
              className={({ isActive }) => `sb-link${isActive ? ' active' : ''}`}
            >
              <span className="sb-ico">{n.icon}</span>
              {n.label}
            </NavLink>
          ))}
        </nav>

        <div className="sb-foot">
          {user && (
            <>
              <div className="sb-user">
                <span className="sb-avatar">{initials(user.name)}</span>
                <span className="sb-uname">{user.name}</span>
              </div>
              <button className="ghost sb-logout" onClick={handleLogout}>Cerrar sesión</button>
            </>
          )}
        </div>
      </aside>

      <main className="content">
        {(title || actions) && (
          <header className="content-head">
            {title && <h1>{title}</h1>}
            {actions && <div className="content-actions">{actions}</div>}
          </header>
        )}
        {children}
      </main>
    </div>
  );
}
