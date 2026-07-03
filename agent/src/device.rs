//! Cliente del Lite desatendido: se registra una vez (clave fija), mantiene una
//! conexión persistente a /ws/device para presencia, y cuando el técnico se
//! conecta por la clave recibe `start {code}` y hospeda la sesión de control.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use crate::config::{to_http, LiteConfig};
use crate::session::{run_remote_session, LiteEvent};

/// Registra el dispositivo y devuelve la config con la clave fija permanente.
pub async fn register_device(server: &str, name: &str) -> anyhow::Result<LiteConfig> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Resp { device_id: String, access_key: String, secret: String }
    let url = format!("{}/api/device/register", to_http(server));
    let mut body = serde_json::json!({ "name": name });
    if let Some(mid) = crate::config::machine_id() {
        body["machineId"] = serde_json::Value::String(mid);
    }
    let resp = reqwest::Client::new().post(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("no se pudo registrar ({})", resp.status());
    }
    let r: Resp = resp.json().await?;
    Ok(LiteConfig {
        server: server.trim().to_string(),
        device_id: r.device_id,
        access_key: r.access_key,
        secret: r.secret,
        ..Default::default()
    })
}

/// Orquestador del Lite: registra (con reintento) si no hay config, activa el
/// arranque con Windows la primera vez, y corre el dispositivo.
pub async fn run_lite_unattended(server: String, name: String, ui: std::sync::mpsc::Sender<LiteEvent>) {
    let cfg = loop {
        // Reusa la identidad guardada y la copia al registro (durable) por si solo
        // estaba en %APPDATA%. Así nunca se re-registra/duplica un equipo conocido.
        if let Some(c) = LiteConfig::load() { c.save(); break c; }
        let _ = ui.send(LiteEvent::Status("Registrando este equipo…".into()));
        match register_device(&server, &name).await {
            Ok(c) => {
                c.save();
                let _ = crate::autostart::set_autostart(true); // arranca con Windows por defecto
                info!("dispositivo registrado: clave {}", c.access_key);
                break c;
            }
            Err(e) => {
                let _ = ui.send(LiteEvent::Status(format!("Sin conexión al servidor ({e}). Reintentando…")));
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    };
    run_device(cfg, name, ui).await;
}

/// Ayudante lanzado por el servicio de Windows en la sesión interactiva: NO
/// registra ni toca el autoarranque (de eso se encarga el servicio); solo carga
/// la identidad ya persistida en HKLM y hospeda las sesiones. Si aún no existe
/// (el servicio la está registrando), espera y reintenta.
pub async fn run_helper_device(server: String, name: String, ui: std::sync::mpsc::Sender<LiteEvent>) {
    let _ = server; // el servidor efectivo sale de la config guardada.
    let cfg = loop {
        if let Some(c) = LiteConfig::load() {
            break c;
        }
        let _ = ui.send(LiteEvent::Status("Registrando este equipo…".into()));
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    run_device(cfg, name, ui).await;
}

/// Garantiza que el equipo esté registrado y su identidad persistida en HKLM.
/// La invoca el servicio (SYSTEM) al arrancar, para que la clave exista desde el
/// boot —incluso antes de que nadie inicie sesión—. Reintenta hasta lograrlo.
pub async fn ensure_registered(server: &str, name: &str) {
    if LiteConfig::load().is_some() {
        return;
    }
    loop {
        if LiteConfig::load().is_some() {
            return;
        }
        match register_device(server, name).await {
            Ok(c) => {
                c.save(); // SYSTEM escribe en HKLM (compartida con el ayudante)
                info!("dispositivo registrado por el servicio: clave {}", c.access_key);
                return;
            }
            Err(e) => {
                warn!("registro (servicio) falló: {e}. Reintentando…");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// Bucle persistente: presencia + atención de solicitudes de conexión. Si el
/// servidor rechaza la autenticación (`auth_failed` —p. ej. su BD se recreó o el
/// equipo fue borrado— ) se RE-REGISTRA solo, conservando el login, para no
/// quedarse atascado en "Sin conexión" con credenciales muertas.
pub async fn run_device(mut cfg: LiteConfig, name: String, ui: std::sync::mpsc::Sender<LiteEvent>) {
    let _ = ui.send(LiteEvent::Code(cfg.access_key.clone()));
    let busy = Arc::new(AtomicBool::new(false));
    loop {
        let _ = ui.send(LiteEvent::Status("Conectando al servidor…".into()));
        match connect_once(&cfg, &name, &ui, &busy).await {
            Ok(()) => {}
            Err(e) if e.to_string().contains("auth_failed") => {
                warn!("device auth_failed: re-registrando este equipo");
                let _ = ui.send(LiteEvent::Status("Re-registrando este equipo…".into()));
                match register_device(&cfg.server, &name).await {
                    Ok(mut newcfg) => {
                        // Conserva el login del usuario tras el re-registro.
                        newcfg.session_token = cfg.session_token.take();
                        newcfg.user_email = cfg.user_email.take();
                        newcfg.save();
                        cfg = newcfg;
                        let _ = ui.send(LiteEvent::Code(cfg.access_key.clone()));
                        continue; // reconecta ya con las credenciales nuevas
                    }
                    Err(re) => warn!("re-registro falló: {re}"),
                }
            }
            Err(e) => warn!("device WS: {e}"),
        }
        let _ = ui.send(LiteEvent::Status("Sin conexión. Reintentando…".into()));
        tokio::time::sleep(Duration::from_millis(2500)).await;
    }
}

async fn connect_once(
    cfg: &LiteConfig,
    name: &str,
    ui: &std::sync::mpsc::Sender<LiteEvent>,
    busy: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let (ws, _) = tokio_tungstenite::connect_async(cfg.ws_device_url()).await?;
    let (mut w, mut r) = ws.split();
    w.send(Message::Text(serde_json::json!({
        "type": "hello",
        "deviceId": cfg.device_id,
        "secret": cfg.secret,
        "version": crate::update::CURRENT_VERSION,
    }).to_string())).await?;

    loop {
        match r.next().await {
            Some(Ok(Message::Text(text))) => {
                let m: serde_json::Value = match serde_json::from_str(&text) { Ok(v) => v, Err(_) => continue };
                match m.get("type").and_then(|v| v.as_str()) {
                    Some("ready") => { let _ = ui.send(LiteEvent::Status("En línea · esperando al técnico".into())); }
                    Some("error") => { warn!("device auth: {text}"); anyhow::bail!("auth_failed"); }
                    Some("start") => {
                        let code = m.get("code").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if code.is_empty() || busy.swap(true, Ordering::SeqCst) { continue; }
                        // Marca sesión activa: el servicio no auto-actualizará mientras dure.
                        crate::update::set_session_active(true);
                        let signal = cfg.ws_signal_url();
                        let name = name.to_string();
                        let ui2 = ui.clone();
                        let busy2 = busy.clone();
                        tokio::spawn(async move {
                            let _ = ui2.send(LiteEvent::Status("Conectado · compartiendo tu pantalla".into()));
                            if let Err(e) = run_remote_session(&signal, &name, &code).await {
                                warn!("sesión: {e:#}");
                            }
                            busy2.store(false, Ordering::SeqCst);
                            crate::update::set_session_active(false);
                            let _ = ui2.send(LiteEvent::Status("En línea · esperando al técnico".into()));
                        });
                    }
                    _ => {}
                }
            }
            Some(Ok(Message::Close(_))) | None => break,
            Some(Err(_)) => break,
            _ => {}
        }
    }
    info!("device WS cerrado");
    Ok(())
}
