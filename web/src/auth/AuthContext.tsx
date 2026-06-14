import { createContext, useContext, useEffect, useState, type ReactNode } from 'react';
import { api, HttpError, type User } from '../api/client';

interface AuthState {
  user: User | null;
  loading: boolean;
  login: (email: string, password: string) => Promise<void>;
  register: (email: string, password: string, name: string) => Promise<void>;
  logout: () => Promise<void>;
  refresh: () => Promise<void>;
}

const AuthCtx = createContext<AuthState | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<User | null>(null);
  const [loading, setLoading] = useState(true);

  async function refresh() {
    try {
      const { user } = await api.get<{ user: User }>('/api/auth/me');
      setUser(user);
    } catch (err) {
      if (err instanceof HttpError && err.status === 401) setUser(null);
      else throw err;
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { void refresh(); }, []);

  async function login(email: string, password: string) {
    const { user } = await api.post<{ user: User }>('/api/auth/login', { email, password });
    setUser(user);
  }

  async function register(email: string, password: string, name: string) {
    const { user } = await api.post<{ user: User }>('/api/auth/register', { email, password, name });
    setUser(user);
  }

  async function logout() {
    await api.post('/api/auth/logout');
    setUser(null);
  }

  return (
    <AuthCtx.Provider value={{ user, loading, login, register, logout, refresh }}>
      {children}
    </AuthCtx.Provider>
  );
}

export function useAuth() {
  const ctx = useContext(AuthCtx);
  if (!ctx) throw new Error('useAuth must be used inside <AuthProvider>');
  return ctx;
}
