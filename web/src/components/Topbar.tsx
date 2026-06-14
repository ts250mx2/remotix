import { Link, useNavigate } from 'react-router-dom';
import { useAuth } from '../auth/AuthContext';

export function Topbar() {
  const { user, logout } = useAuth();
  const nav = useNavigate();

  async function handleLogout() {
    await logout();
    nav('/login');
  }

  return (
    <header className="topbar">
      <Link to="/" className="brand">Remotix</Link>
      <nav>
        <Link to="/">Mis PCs</Link>
        <Link to="/groups">Grupos</Link>
        <Link to="/users">Usuarios</Link>
        <Link to="/operador">Conectar por clave</Link>
      </nav>
      <div className="topbar-right">
        {user && (
          <>
            <span className="user-info">
              <span className="user-name">{user.name}</span>
            </span>
            <button className="ghost" onClick={handleLogout}>Salir</button>
          </>
        )}
      </div>
    </header>
  );
}
