import { useState, type FormEvent } from 'react';
import { Link, useLocation, useNavigate } from 'react-router-dom';
import { useAuth } from '../auth/AuthContext';
import { HttpError } from '../api/client';

export function Login() {
  const { login } = useAuth();
  const nav = useNavigate();
  const loc = useLocation();
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setErr(null);
    setLoading(true);
    try {
      await login(email, password);
      const from = (loc.state as { from?: { pathname: string } } | null)?.from?.pathname ?? '/';
      nav(from, { replace: true });
    } catch (e) {
      setErr(e instanceof HttpError && e.status === 401 ? 'Credenciales inválidas' : 'Error al iniciar sesión');
    } finally {
      setLoading(false);
    }
  }

  return (
    <main className="centered">
      <form onSubmit={onSubmit} className="card narrow">
        <h1>Remotix</h1>
        <p className="muted">Acceder al portal</p>
        <label>Email
          <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} required autoFocus />
        </label>
        <label>Contraseña
          <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} required />
        </label>
        {err && <div className="error">{err}</div>}
        <button type="submit" disabled={loading}>{loading ? 'Entrando…' : 'Entrar'}</button>
        <p className="muted small">¿No tienes cuenta? <Link to="/register">Crea una</Link>.</p>
        <hr className="sep" />
        <p className="muted small helpdesk-links">
          Soporte instantáneo (sin cuenta): <Link to="/ayuda">Necesito ayuda</Link> · <Link to="/operador">Soy técnico</Link>
        </p>
        <p className="muted small helpdesk-links">
          ¿Tu proveedor te dio un código de proyecto? <Link to="/conectar">Conectar al chat de soporte</Link>
        </p>
      </form>
    </main>
  );
}
