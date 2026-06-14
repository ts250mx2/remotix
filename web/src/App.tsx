import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { AuthProvider } from './auth/AuthContext';
import { RequireAuth } from './components/RequireAuth';
import { Login } from './pages/Login';
import { Register } from './pages/Register';
import { Dashboard } from './pages/Dashboard';
import { ProjectDetail } from './pages/ProjectDetail';
import { Groups } from './pages/Groups';
import { ClientSupport } from './pages/ClientSupport';
import { OperatorConsole } from './pages/OperatorConsole';
import { ChatPage } from './pages/ChatPage';
import { ClientChat } from './pages/ClientChat';

export function App() {
  return (
    <BrowserRouter>
      <AuthProvider>
        <Routes>
          {/* Helpdesk público (sin login): soporte instantáneo */}
          <Route path="/ayuda" element={<ClientSupport />} />
          <Route path="/operador" element={<OperatorConsole />} />
          <Route path="/conectar" element={<ClientChat />} />

          {/* Portal (requiere cuenta) */}
          <Route path="/login" element={<Login />} />
          <Route path="/register" element={<Register />} />
          <Route path="/" element={<RequireAuth><Dashboard /></RequireAuth>} />
          <Route path="/chat" element={<RequireAuth><ChatPage /></RequireAuth>} />
          <Route path="/projects/:id" element={<RequireAuth><ProjectDetail /></RequireAuth>} />
          <Route path="/groups" element={<RequireAuth><Groups /></RequireAuth>} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </AuthProvider>
    </BrowserRouter>
  );
}
