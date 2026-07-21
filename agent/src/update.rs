//! Auto-actualización del agente.
//!
//! El servicio (SYSTEM) consulta periódicamente `/api/update/latest`; si hay una
//! versión más nueva y no hay una sesión remota activa (o es obligatoria), la
//! aplica en silencio ejecutando el instalador —que detiene el servicio,
//! reemplaza los archivos y lo vuelve a arrancar—. Como lo lanza el servicio, no
//! salta ningún UAC.
//!
//! La versión instalada se reporta al servidor en el `hello` del WS (ver
//! `device.rs`) para saber qué PC tiene qué versión.

use std::time::Duration;

use serde::Deserialize;

/// Versión de este binario (del Cargo.toml). Fuente única de verdad.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize, Clone, Debug)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub mandatory: bool,
}

/// Compara versiones "a.b.c". Devuelve true si `latest` es más nueva que `current`.
/// Ignora sufijos no numéricos (1.2.0-beta → [1,2,0]).
pub fn version_is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split(['.', '-', '+'])
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .collect()
    };
    let a = parse(latest);
    let b = parse(current);
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// Consulta la última versión publicada de la APP. Devuelve Some solo si hay
/// una MÁS NUEVA.
pub async fn check_latest(server: &str) -> Option<UpdateInfo> {
    check_channel(server, "app").await
}

/// Igual, pero del canal HOST (instalación con servicio de Windows). Manifiesto
/// separado para que un host jamás se "actualice" con el instalador de la app.
pub async fn check_latest_host(server: &str) -> Option<UpdateInfo> {
    check_channel(server, "host").await
}

async fn check_channel(server: &str, channel: &str) -> Option<UpdateInfo> {
    let url = format!("{}/api/update/latest?channel={channel}", crate::config::to_http(server));
    let client = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().ok()?;
    let info: UpdateInfo = client.get(&url).send().await.ok()?.json().await.ok()?;
    version_is_newer(&info.version, CURRENT_VERSION).then_some(info)
}

// ---------------------------------------------------------------------------
// Windows: lock de sesión activa + aplicación de la actualización
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn state_dir() -> std::path::PathBuf {
    let base = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".into());
    let dir = std::path::Path::new(&base).join("Remotix");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(windows)]
fn lock_path() -> std::path::PathBuf {
    state_dir().join("session.lock")
}

/// Marca (o limpia) que hay una sesión remota activa. Lo escribe el ayudante al
/// empezar/terminar de hospedar; el servicio lo lee para NO actualizar en medio.
#[cfg(windows)]
pub fn set_session_active(active: bool) {
    if active {
        let _ = std::fs::write(lock_path(), b"1");
    } else {
        let _ = std::fs::remove_file(lock_path());
    }
}

/// ¿Hay una sesión remota activa? (lo consulta el servicio antes de actualizar)
#[cfg(windows)]
pub fn session_active() -> bool {
    lock_path().exists()
}

// ---------------------------------------------------------------------------
// Reanudación de sesión tras actualización en caliente
// ---------------------------------------------------------------------------
//
// Mientras hay una sesión activa, el host escribe (y refresca cada ~30 s) un
// archivo con el código de la sala. Si el proceso muere a mitad (el instalador
// lo reemplaza), el proceso NUEVO encuentra el archivo fresco al arrancar y
// vuelve a hospedar LA MISMA sala — el visor del técnico, que quedó esperando
// (`host-reconnecting`), renegocia y la sesión continúa sola.

#[cfg(windows)]
fn resume_path() -> std::path::PathBuf {
    state_dir().join("resume-session.json")
}

/// Frescura máxima del archivo de reanudación: cubre descarga + instalador +
/// relanzamiento, pero descarta sesiones de un apagado/crash de hace horas.
const RESUME_MAX_AGE_SECS: u64 = 120;

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Escribe (o refresca) el código de la sesión activa. Lo llama el host al
/// empezar a hospedar y cada ~30 s mientras dure.
#[cfg(windows)]
pub fn write_resume(code: &str) {
    let body = serde_json::json!({ "code": code, "ts": now_epoch() });
    let _ = std::fs::write(resume_path(), body.to_string());
}

/// Borra el marcador (la sesión terminó de forma normal).
#[cfg(windows)]
pub fn clear_resume() {
    let _ = std::fs::remove_file(resume_path());
}

/// Si el proceso anterior murió con una sesión activa (marcador fresco),
/// devuelve el código para re-hospedarla. Consume el archivo.
#[cfg(windows)]
pub fn take_resume() -> Option<String> {
    let raw = std::fs::read_to_string(resume_path()).ok()?;
    let _ = std::fs::remove_file(resume_path());
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let ts = v.get("ts").and_then(|x| x.as_u64())?;
    let code = v.get("code").and_then(|x| x.as_str())?.to_string();
    let age = now_epoch().saturating_sub(ts);
    (!code.is_empty() && age <= RESUME_MAX_AGE_SECS).then_some(code)
}

#[cfg(not(windows))]
pub fn write_resume(_code: &str) {}
#[cfg(not(windows))]
pub fn clear_resume() {}
#[cfg(not(windows))]
pub fn take_resume() -> Option<String> {
    None
}

/// Descarga el instalador y lo ejecuta en silencio. El instalador detiene el
/// servicio, reemplaza los archivos y lo vuelve a arrancar. Sin UAC (lo lanza el
/// servicio como SYSTEM).
#[cfg(windows)]
pub async fn download_and_apply(server: &str, url: &str) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let full = if url.starts_with("http") {
        url.to_string()
    } else {
        format!("{}{}", crate::config::to_http(server), url)
    };
    let client = reqwest::Client::builder().timeout(Duration::from_secs(600)).build()?;
    let bytes = client.get(&full).send().await?.error_for_status()?.bytes().await?;

    let tmp = std::env::temp_dir().join("RemotixHostSetup.exe");
    std::fs::write(&tmp, &bytes)?;

    std::process::Command::new(&tmp)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/NOCANCEL"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    Ok(())
}

#[cfg(not(windows))]
pub fn set_session_active(_active: bool) {}
#[cfg(not(windows))]
pub fn session_active() -> bool {
    false
}
#[cfg(not(windows))]
pub async fn download_and_apply(_server: &str, _url: &str) -> anyhow::Result<()> {
    Ok(())
}
