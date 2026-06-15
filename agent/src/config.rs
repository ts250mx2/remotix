//! Configuración persistente del agente (credenciales tras enrolar por UUID).

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AgentConfig {
    pub server: String,       // base, ej: https://soporte.dominio.com  o  http://localhost:8080
    pub equipo_id: String,
    pub agent_secret: String,
    pub project_id: String,
    pub name: String,
}

fn config_path() -> PathBuf {
    let dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("Remotix");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("config.json")
}

impl AgentConfig {
    pub fn load() -> Option<AgentConfig> {
        let data = std::fs::read_to_string(config_path()).ok()?;
        let cfg: AgentConfig = serde_json::from_str(&data).ok()?;
        if cfg.equipo_id.is_empty() || cfg.agent_secret.is_empty() { None } else { Some(cfg) }
    }
    pub fn save(&self) {
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(config_path(), data);
        }
    }
    pub fn clear() {
        let _ = std::fs::remove_file(config_path());
    }
    pub fn ws_chat_url(&self) -> String { to_ws(&self.server, "/ws/chat") }
    pub fn ws_signal_url(&self) -> String { to_ws(&self.server, "/ws/signal") }
    pub fn http_base(&self) -> String { to_http(&self.server) }
}

/// Configuración persistente del Lite desatendido (clave fija permanente).
/// `session_token`/`user_email` guardan el login del usuario en el exe (para
/// mantener la sesión entre arranques: rol operador).
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct LiteConfig {
    pub server: String,
    pub device_id: String,
    pub access_key: String,
    pub secret: String,
    #[serde(default)]
    pub session_token: Option<String>,
    #[serde(default)]
    pub user_email: Option<String>,
}

fn lite_path() -> PathBuf {
    let dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("Remotix");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("lite.json")
}

impl LiteConfig {
    pub fn load() -> Option<LiteConfig> {
        // Preferir el registro: sobrevive a reinstalar o borrar %APPDATA%.
        if let Some(c) = registry_load() {
            return Some(c);
        }
        let data = std::fs::read_to_string(lite_path()).ok()?;
        let cfg: LiteConfig = serde_json::from_str(&data).ok()?;
        if cfg.device_id.is_empty() || cfg.access_key.is_empty() { None } else { Some(cfg) }
    }
    pub fn save(&self) {
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(lite_path(), data);
        }
        registry_save(self); // copia durable en el registro
    }
    /// Persiste (o limpia) el login del usuario en este equipo.
    pub fn set_session(token: Option<String>, email: Option<String>) {
        if let Some(mut c) = LiteConfig::load() {
            c.session_token = token;
            c.user_email = email;
            c.save();
        }
    }
    pub fn ws_device_url(&self) -> String { to_ws(&self.server, "/ws/device") }
    pub fn ws_signal_url(&self) -> String { to_ws(&self.server, "/ws/signal") }
    pub fn http_base(&self) -> String { to_http(&self.server) }
}

/// Convierte una base (http/https/ws/wss o sin esquema) a una URL WS con `path`.
pub fn to_ws(server: &str, path: &str) -> String {
    let base = server.trim().trim_end_matches('/');
    let scheme = if let Some(r) = base.strip_prefix("https://") {
        format!("wss://{r}")
    } else if let Some(r) = base.strip_prefix("http://") {
        format!("ws://{r}")
    } else if base.starts_with("ws://") || base.starts_with("wss://") {
        base.to_string()
    } else {
        format!("ws://{base}")
    };
    format!("{scheme}{path}")
}

/// Normaliza la base a http(s) para llamadas REST.
pub fn to_http(server: &str) -> String {
    let base = server.trim().trim_end_matches('/');
    if let Some(r) = base.strip_prefix("wss://") {
        format!("https://{r}")
    } else if let Some(r) = base.strip_prefix("ws://") {
        format!("http://{r}")
    } else if base.starts_with("http://") || base.starts_with("https://") {
        base.to_string()
    } else {
        format!("http://{base}")
    }
}

// ---------------------------------------------------------------------------
// Persistencia durable en el registro de Windows (HKCU\Software\Remotix) +
// identificador estable de la máquina. Evita que reinstalar duplique el equipo.
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn registry_load() -> Option<LiteConfig> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Remotix").ok()?;
    let device_id: String = key.get_value("device_id").ok()?;
    let access_key: String = key.get_value("access_key").ok()?;
    if device_id.is_empty() || access_key.is_empty() {
        return None;
    }
    let server: String = key.get_value("server").unwrap_or_default();
    let secret: String = key.get_value("secret").unwrap_or_default();
    let session_token: String = key.get_value("session_token").unwrap_or_default();
    let user_email: String = key.get_value("user_email").unwrap_or_default();
    Some(LiteConfig {
        server,
        device_id,
        access_key,
        secret,
        session_token: (!session_token.is_empty()).then_some(session_token),
        user_email: (!user_email.is_empty()).then_some(user_email),
    })
}

#[cfg(windows)]
fn registry_save(c: &LiteConfig) {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    if let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey("Software\\Remotix") {
        let _ = key.set_value("server", &c.server);
        let _ = key.set_value("device_id", &c.device_id);
        let _ = key.set_value("access_key", &c.access_key);
        let _ = key.set_value("secret", &c.secret);
        let _ = key.set_value("session_token", &c.session_token.clone().unwrap_or_default());
        let _ = key.set_value("user_email", &c.user_email.clone().unwrap_or_default());
    }
}

/// MachineGuid de Windows: estable por instalación de SO, no cambia al reinstalar
/// la app. El server lo usa para no duplicar el equipo.
#[cfg(windows)]
pub fn machine_id() -> Option<String> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Microsoft\\Cryptography")
        .ok()?;
    let guid: String = key.get_value("MachineGuid").ok()?;
    (!guid.trim().is_empty()).then_some(guid)
}

#[cfg(not(windows))]
fn registry_load() -> Option<LiteConfig> { None }
#[cfg(not(windows))]
fn registry_save(_c: &LiteConfig) {}
#[cfg(not(windows))]
pub fn machine_id() -> Option<String> { None }
