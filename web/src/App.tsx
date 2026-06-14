import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { AuthProvider } from './auth/AuthContext';
import { RequireAuth } from './components/RequireAuth';
import { Login } from './pages/Login';
import { Register } from './pages/Register';
import { Devices } from './pages/Devices';
import { DeviceDetail } from './pages/DeviceDetail';
import { Groups } from './pages/Groups';
import { Users } from './pages/Users';
import { ClientSupport } from './pages/ClientSupport';
import { OperatorConsole } from './pages/OperatorConsole';

export function App() {
  return (
    <BrowserRouter>
      <AuthProvider>
        <Routes>
          {/* Consola del operador (requiere login para conectar por clave/id) y
              compartir pantalla rápido desde el navegador (sin instalar). */}
          <Route path="/operador" element={<OperatorConsole />} />
          <Route path="/ayuda" element={<ClientSupport />} />

          {/* Portal de administración (requiere cuenta) */}
          <Route path="/login" element={<Login />} />
          <Route path="/register" element={<Register />} />
          <Route path="/" element={<RequireAuth><Devices /></RequireAuth>} />
          <Route path="/devices/:id" element={<RequireAuth><DeviceDetail /></RequireAuth>} />
          <Route path="/groups" element={<RequireAuth><Groups /></RequireAuth>} />
          <Route path="/users" element={<RequireAuth><Users /></RequireAuth>} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </AuthProvider>
    </BrowserRouter>
  );
}
