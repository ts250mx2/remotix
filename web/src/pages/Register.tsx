import { useState, type FormEvent } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { useAuth } from '../auth/AuthContext';
import { HttpError } from '../api/client';

export function Register() {
  const { register } = useAuth();
  const nav = useNavigate();
  const [name, setName] = useState('');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setErr(null);
    setLoading(true);
    try {
      await register(email, password, name);
      nav('/', { replace: true });
    } catch (e) {
      if (e instanceof HttpError && e.status === 409) setErr('Ese email ya está registrado');
      else if (e instanceof HttpError && e.status === 400) setErr('Datos inválidos (contraseña ≥ 8 caracteres)');
      else setErr('Error al registrar');
    } finally {
      setLoading(false);
    }
  }

  return (
    <main className="centered">
      <form onSubmit={onSubmit} className="card narrow">
        <h1>Crear cuenta</h1>
        <label>Nombre
          <input value={name} onChange={(e) => setName(e.target.value)} required autoFocus />
        </label>
        <label>Email
          <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} required />
        </label>
        <label>Contraseña
          <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} required minLength={8} />
        </label>
        {err && <div className="error">{err}</div>}
        <button type="submit" disabled={loading}>{loading ? 'Creando…' : 'Crear cuenta'}</button>
        <p className="muted small">¿Ya tienes cuenta? <Link to="/login">Inicia sesión</Link>.</p>
      </form>
    </main>
  );
}
