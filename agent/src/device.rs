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

    // Escritor único del socket: el hello y los mensajes que nacen en tareas
    // asíncronas (p. ej. `declined` desde el prompt de confirmación) comparten
    // esta misma conexión autenticada.
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if w.send(Message::Text(text)).await.is_err() { break; }
        }
    });
    let _ = out_tx.send(serde_json::json!({
        "type": "hello",
        "deviceId": cfg.device_id,
        "secret": cfg.secret,
        "version": crate::update::CURRENT_VERSION,
    }).to_string());

    loop {
        match r.next().await {
            Some(Ok(Message::Text(text))) => {
                let m: serde_json::Value = match serde_json::from_str(&text) { Ok(v) => v, Err(_) => continue };
                match m.get("type").and_then(|v| v.as_str()) {
                    Some("ready") => {
                        let confirm = m.get("requireConfirm").and_then(|v| v.as_bool()).unwrap_or(false);
                        let _ = ui.send(LiteEvent::ConfirmMode(confirm));
                        let _ = ui.send(LiteEvent::Status("En línea · listo para recibir conexiones".into()));
                    }
                    Some("error") => { warn!("device auth: {text}"); anyhow::bail!("auth_failed"); }
                    // Push del servidor: hay versión nueva publicada. La GUI (o el
                    // servicio, según el binario) decide cuándo aplicarla.
                    Some("update") => { let _ = ui.send(LiteEvent::UpdateAvailable); }
                    // Alguien cambió el toggle de confirmación (desde esta ventana u
                    // otro proceso del equipo): sincroniza el checkbox de la GUI.
                    Some("confirm_mode") => {
                        let v = m.get("value").and_then(|v| v.as_bool()).unwrap_or(false);
                        let _ = ui.send(LiteEvent::ConfirmMode(v));
                    }
                    Some("start") => {
                        let code = m.get("code").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let confirm = m.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false);
                        if code.is_empty() || busy.swap(true, Ordering::SeqCst) { continue; }
                        // Marca sesión activa: el servicio no auto-actualizará mientras dure.
                        crate::update::set_session_active(true);
                        let signal = cfg.ws_signal_url();
                        let name = name.to_string();
                        let ui2 = ui.clone();
                        let busy2 = busy.clone();
                        let out2 = out_tx.clone();
                        tokio::spawn(async move {
                            // Modo "pedir permiso": el usuario del equipo debe aceptar
                            // antes de hospedar. Rechazo o timeout → se avisa al server
                            // (que informa al operador) y no se comparte nada.
                            if confirm {
                                match ask_permission(&ui2).await {
                                    ConfirmOutcome::Accepted => {}
                                    ConfirmOutcome::Declined => {
                                        let _ = out2.send(serde_json::json!({ "type": "declined", "code": code }).to_string());
                                        busy2.store(false, Ordering::SeqCst);
                                        crate::update::set_session_active(false);
                                        let _ = ui2.send(LiteEvent::Status("En línea · listo para recibir conexiones".into()));
                                        return;
                                    }
                                    // Otro proceso del equipo tiene el diálogo: él decide
                                    // (y hospeda si aceptan); este proceso se retira.
                                    ConfirmOutcome::Delegated => {
                                        busy2.store(false, Ordering::SeqCst);
                                        crate::update::set_session_active(false);
                                        return;
                                    }
                                }
                            }
                            // Estado honesto: el visor lo actualizará a "Conectado"
                            // solo cuando WebRTC lo confirme (dentro de run_remote_session).
                            let _ = ui2.send(LiteEvent::Status("El técnico se está conectando…".into()));
                            if let Err(e) = run_remote_session(&signal, &name, &code, Some(ui2.clone())).await {
                                warn!("sesión: {e:#}");
                            }
                            busy2.store(false, Ordering::SeqCst);
                            crate::update::set_session_active(false);
                            let _ = ui2.send(LiteEvent::Status("En línea · listo para recibir conexiones".into()));
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

/// Tiempo que se le da al usuario para responder al diálogo de confirmación.
/// Sin respuesta (nadie frente al equipo) se rechaza: "pedir permiso" implica
/// que sin alguien que apruebe no hay acceso.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(30);

enum ConfirmOutcome {
    Accepted,
    Declined,
    /// Otro proceso del mismo equipo ya está mostrando el diálogo.
    Delegated,
}

/// Un único diálogo vivo por proceso. El diálogo nativo no puede cerrarse desde
/// fuera: si expiró el timeout queda "huérfano" hasta que alguien lo cierre, con
/// su hilo bloqueado. Este flag evita abrir otro mientras tanto (las peticiones
/// nuevas se rechazan directas), acotando a 1 los popups/hilos por proceso en un
/// equipo sin nadie delante que reciba intentos repetidos.
static PROMPT_LIVE: AtomicBool = AtomicBool::new(false);

/// Muestra el diálogo nativo "¿Permitir la conexión?" con timeout. El mutex con
/// nombre evita diálogos duplicados cuando la ventana y el ayudante del servicio
/// reciben el mismo `start` (solo el primero pregunta; el resto delega).
async fn ask_permission(ui: &std::sync::mpsc::Sender<LiteEvent>) -> ConfirmOutcome {
    #[cfg(windows)]
    let _guard = match PromptGuard::acquire() {
        Some(g) => g,
        None => return ConfirmOutcome::Delegated,
    };
    if PROMPT_LIVE.swap(true, Ordering::SeqCst) {
        // Hay un diálogo anterior sin responder: no apilar otro.
        return ConfirmOutcome::Declined;
    }

    let _ = ui.send(LiteEvent::Status("Un técnico pide conectarse · esperando tu respuesta…".into()));
    let dialog = tokio::task::spawn_blocking(|| {
        let r = rfd::MessageDialog::new()
            .set_title("Remotix")
            .set_level(rfd::MessageLevel::Info)
            .set_description("Un técnico quiere conectarse a este equipo para verlo y controlarlo.\n\n¿Permitir la conexión?")
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        // Lo limpia el propio hilo del diálogo: aunque la sesión ya se haya
        // rechazado por timeout, hasta aquí el popup seguía vivo.
        PROMPT_LIVE.store(false, Ordering::SeqCst);
        matches!(r, rfd::MessageDialogResult::Yes)
    });
    tokio::select! {
        r = dialog => if r.unwrap_or(false) { ConfirmOutcome::Accepted } else { ConfirmOutcome::Declined },
        _ = tokio::time::sleep(CONFIRM_TIMEOUT) => {
            // El diálogo huérfano queda hasta que alguien lo cierre; su respuesta
            // tardía se descarta (la sesión ya se rechazó).
            ConfirmOutcome::Declined
        }
    }
}

/// Mutex con nombre por sesión de Windows: el proceso que lo crea "posee" el
/// prompt de confirmación. Se guarda el HANDLE como isize para poder cruzar
/// `.await` (los punteros crudos no son Send).
#[cfg(windows)]
struct PromptGuard(isize);

#[cfg(windows)]
impl PromptGuard {
    fn acquire() -> Option<Self> {
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let name: Vec<u16> = "Local\\Remotix-Confirm-Prompt\0".encode_utf16().collect();
        let h = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if h.is_null() {
            // Sin mutex disponible: mejor preguntar (posible diálogo doble) que
            // no preguntar nunca.
            return Some(Self(0));
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(h) };
            return None;
        }
        Some(Self(h as isize))
    }
}

#[cfg(windows)]
impl Drop for PromptGuard {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0 as *mut core::ffi::c_void) };
        }
    }
}

/// Cambia el toggle "pedir permiso antes de conectar" en el servidor (REST,
/// autenticado con el secreto del device). El servidor difunde `confirm_mode`
/// a todos los procesos del equipo, que es lo que actualiza los checkboxes.
pub async fn set_confirm_mode(value: bool) -> anyhow::Result<()> {
    let cfg = LiteConfig::load().ok_or_else(|| anyhow::anyhow!("equipo sin registrar"))?;
    let url = format!("{}/api/device/confirm-mode", to_http(&cfg.server));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({ "deviceId": cfg.device_id, "secret": cfg.secret, "value": value }))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("no se pudo guardar el modo ({})", resp.status());
    }
    Ok(())
}
