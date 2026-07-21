//! Transferencia de archivos por el DataChannel 'files' (bidireccional), con
//! explorador remoto estilo TeamViewer.
//!
//! Protocolo (JSON con campo `f` + chunks binarios):
//!  - `begin {id,name,size,dir?}` / chunks / `end {id}` — envío de un archivo.
//!    `dir` (opcional) es la carpeta destino ABSOLUTA en el receptor (la eligió
//!    el operador navegando); sin `dir` se guarda en Downloads\Remotix (compat
//!    con la consola web, que no manda `dir`).
//!  - `req` — pide al otro lado que abra su diálogo y envíe un archivo (flujo
//!    clásico, lo sigue usando la web).
//!  - `ls {path}` → `ls_res {path,ok,err?,entries:[{n,s,d}]}` — listar carpeta
//!    (path vacío = unidades). Lo responde cualquiera de los dos lados; en la
//!    práctica pregunta el operador y responde el host.
//!  - `get {path}` — el otro lado lee ese archivo y lo envía con begin/end.
//!  - `err {msg}` — error de una operación (se muestra en el explorador).

use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;

const CHUNK: usize = 16 * 1024;
/// Tope de entradas por listado: mantiene el ls_res bajo el límite de mensaje
/// del DataChannel (~64 KB) incluso con nombres largos.
const MAX_ENTRIES: usize = 800;

// ---------------------------------------------------------------------------
// Protocolo
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct FileCtrl {
    f: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    /// Carpeta destino (begin) o ruta a listar/leer (ls/get).
    #[serde(default)]
    dir: Option<String>,
    #[serde(default)]
    path: Option<String>,
    /// Campos de ls_res.
    #[serde(default)]
    ok: Option<bool>,
    #[serde(default)]
    err: Option<String>,
    #[serde(default)]
    entries: Option<Vec<DirEntry>>,
    #[serde(default)]
    msg: Option<String>,
}

#[derive(Serialize)]
struct Begin<'a> {
    f: &'a str,
    id: u64,
    name: &'a str,
    size: u64,
    mime: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    dir: Option<&'a str>,
}

#[derive(Serialize)]
struct End {
    f: &'static str,
    id: u64,
}

/// Entrada de un listado de carpeta: `n` nombre, `s` tamaño, `d` es-carpeta.
#[derive(Serialize, Deserialize, Clone)]
pub struct DirEntry {
    pub n: String,
    #[serde(default)]
    pub s: u64,
    pub d: bool,
}

// ---------------------------------------------------------------------------
// Estado compartido con la GUI del explorador (lado operador)
// ---------------------------------------------------------------------------

/// Transferencia en curso, para la barra de progreso del explorador.
pub struct Transfer {
    pub name: String,
    pub done: u64,
    pub total: u64,
    /// true = enviando (local → remoto); false = recibiendo.
    pub upload: bool,
}

/// Estado que el canal de archivos comparte con la GUI (solo lo usa el visor;
/// el host lo ignora). La GUI lo sondea cada frame.
#[derive(Default)]
pub struct FilesUi {
    /// Último listado remoto recibido: (ruta, entradas | error).
    pub remote_list: Mutex<Option<(String, Result<Vec<DirEntry>, String>)>>,
    /// Carpeta local elegida donde guardar lo próximo que llegue.
    pub save_dir: Mutex<Option<PathBuf>>,
    /// Progreso de la transferencia en curso.
    pub progress: Mutex<Option<Transfer>>,
    /// Mensaje de la última transferencia terminada o del último error.
    pub last_msg: Mutex<Option<String>>,
}

// ---------------------------------------------------------------------------
// Listado local (lo usa la GUI para su panel y el handler para responder `ls`)
// ---------------------------------------------------------------------------

/// Lista una carpeta local. `path` vacío = unidades del sistema. Carpetas
/// primero, orden alfabético, tope MAX_ENTRIES.
pub fn list_local(path: &str) -> Result<Vec<DirEntry>, String> {
    if path.trim().is_empty() {
        let mut drives = Vec::new();
        for letter in b'A'..=b'Z' {
            let root = format!("{}:\\", letter as char);
            if std::path::Path::new(&root).exists() {
                drives.push(DirEntry { n: root, s: 0, d: true });
            }
        }
        return Ok(drives);
    }
    let rd = fs::read_dir(path).map_err(|e| format!("{e}"))?;
    let mut dirs: Vec<DirEntry> = Vec::new();
    let mut files: Vec<DirEntry> = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = entry.metadata();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let size = meta.map(|m| m.len()).unwrap_or(0);
        if is_dir {
            dirs.push(DirEntry { n: name, s: 0, d: true });
        } else {
            files.push(DirEntry { n: name, s: size, d: false });
        }
        if dirs.len() + files.len() >= MAX_ENTRIES {
            break;
        }
    }
    let key = |e: &DirEntry| e.n.to_lowercase();
    dirs.sort_by_key(key);
    files.sort_by_key(key);
    dirs.extend(files);
    Ok(dirs)
}

/// Carpeta padre de una ruta Windows. `"C:\Users\x"` → `"C:\Users"`,
/// `"C:\Users"` → `"C:\"`, `"C:\"` → `""` (unidades).
pub fn parent_dir(p: &str) -> String {
    let t = p.trim_end_matches('\\');
    if t.len() <= 2 {
        return String::new();
    }
    match t.rfind('\\') {
        Some(2) => t[..3].to_string(),
        Some(i) => t[..i].to_string(),
        None => String::new(),
    }
}

/// Une carpeta + nombre. Con base vacía (unidades) el nombre YA es `X:\`.
pub fn join_dir(base: &str, name: &str) -> String {
    if base.trim().is_empty() {
        name.to_string()
    } else {
        format!("{}\\{}", base.trim_end_matches('\\'), name)
    }
}

// ---------------------------------------------------------------------------
// Canal
// ---------------------------------------------------------------------------

struct IncomingFile {
    file: File,
    name: String,
    received: u64,
    total: u64,
}

/// Cablea el canal SIN GUI (host): responde ls/get/req y guarda lo entrante.
pub fn wire_files_channel(dc: Arc<RTCDataChannel>) {
    wire_files_impl(dc, None);
}

/// Cablea el canal CON GUI (visor del operador): igual que el host, pero además
/// publica listados/progreso/errores en `ui` para el explorador.
pub fn wire_files_channel_ui(dc: Arc<RTCDataChannel>, ui: Arc<FilesUi>) {
    wire_files_impl(dc, Some(ui));
}

fn wire_files_impl(dc: Arc<RTCDataChannel>, ui: Option<Arc<FilesUi>>) {
    let incoming: Arc<Mutex<Option<IncomingFile>>> = Arc::new(Mutex::new(None));
    let dc_for_msg = dc.clone();

    dc.on_message(Box::new(move |msg: DataChannelMessage| {
        let incoming = incoming.clone();
        let dc = dc_for_msg.clone();
        let ui = ui.clone();
        Box::pin(async move {
            if msg.is_string {
                let text = String::from_utf8_lossy(&msg.data);
                if let Ok(ctrl) = serde_json::from_str::<FileCtrl>(&text) {
                    match ctrl.f.as_str() {
                        "begin" => start_incoming(&incoming, &ui, ctrl),
                        "end" => finish_incoming(&incoming, &ui),
                        "req" => pick_and_send(dc),
                        "ls" => answer_ls(dc, ctrl.path.unwrap_or_default()).await,
                        "get" => answer_get(dc, ctrl.path.unwrap_or_default()).await,
                        "ls_res" => {
                            if let Some(ui) = &ui {
                                let path = ctrl.path.unwrap_or_default();
                                let res = if ctrl.ok.unwrap_or(false) {
                                    Ok(ctrl.entries.unwrap_or_default())
                                } else {
                                    Err(ctrl.err.unwrap_or_else(|| "error".into()))
                                };
                                *ui.remote_list.lock() = Some((path, res));
                            }
                        }
                        "err" => {
                            if let Some(ui) = &ui {
                                *ui.last_msg.lock() = Some(format!("✗ {}", ctrl.msg.unwrap_or_else(|| "error".into())));
                                *ui.progress.lock() = None;
                            }
                        }
                        _ => {}
                    }
                }
            } else {
                let mut guard = incoming.lock();
                if let Some(i) = guard.as_mut() {
                    if i.file.write_all(&msg.data).is_ok() {
                        i.received += msg.data.len() as u64;
                        if let Some(ui) = &ui {
                            *ui.progress.lock() = Some(Transfer {
                                name: i.name.clone(),
                                done: i.received,
                                total: i.total,
                                upload: false,
                            });
                        }
                    }
                }
            }
        })
    }));
}

/// Responde `ls`: lista la carpeta pedida y devuelve `ls_res`.
async fn answer_ls(dc: Arc<RTCDataChannel>, path: String) {
    let listed = tokio::task::spawn_blocking({
        let path = path.clone();
        move || list_local(&path)
    })
    .await
    .unwrap_or_else(|_| Err("error interno".into()));
    let resp = match listed {
        Ok(entries) => serde_json::json!({ "f": "ls_res", "path": path, "ok": true, "entries": entries }),
        Err(e) => serde_json::json!({ "f": "ls_res", "path": path, "ok": false, "err": e }),
    };
    let _ = dc.send_text(resp.to_string()).await;
}

/// Responde `get`: lee el archivo pedido y lo envía con begin/chunks/end.
async fn answer_get(dc: Arc<RTCDataChannel>, path: String) {
    let read = tokio::task::spawn_blocking({
        let path = path.clone();
        move || fs::read(&path)
    })
    .await;
    match read {
        Ok(Ok(data)) => {
            let name = path.rsplit(['/', '\\']).next().unwrap_or("archivo").to_string();
            send_file_ex(dc, name, data, None, None).await;
        }
        Ok(Err(e)) => {
            let resp = serde_json::json!({ "f": "err", "msg": format!("no se pudo leer {path}: {e}") });
            let _ = dc.send_text(resp.to_string()).await;
        }
        Err(_) => {}
    }
}

fn default_dir() -> PathBuf {
    let base = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
    let dir = PathBuf::from(base).join("Downloads").join("Remotix");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn sanitize(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("archivo");
    let cleaned: String = base.chars().filter(|c| !"<>:\"|?*\0".contains(*c)).collect();
    if cleaned.trim().is_empty() {
        "archivo".into()
    } else {
        cleaned
    }
}

fn unique_path(mut path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = path.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for n in 1..1000 {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            path = candidate;
            break;
        }
    }
    path
}

/// Carpeta destino de un archivo entrante, por prioridad: `dir` del begin (el
/// emisor navegó hasta ella), carpeta local del explorador (save_dir), o
/// Downloads\Remotix (flujo clásico / consola web).
fn incoming_dir(ctrl_dir: &Option<String>, ui: &Option<Arc<FilesUi>>) -> PathBuf {
    if let Some(d) = ctrl_dir {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return p;
        }
    }
    if let Some(ui) = ui {
        if let Some(d) = ui.save_dir.lock().clone() {
            if d.is_dir() {
                return d;
            }
        }
    }
    default_dir()
}

fn start_incoming(incoming: &Arc<Mutex<Option<IncomingFile>>>, ui: &Option<Arc<FilesUi>>, ctrl: FileCtrl) {
    let name = sanitize(ctrl.name.as_deref().unwrap_or("archivo"));
    let total = ctrl.size.unwrap_or(0);
    let path = unique_path(incoming_dir(&ctrl.dir, ui).join(&name));
    match File::create(&path) {
        Ok(file) => {
            info!("recibiendo archivo → {} ({} bytes)", path.display(), total);
            if let Some(ui) = ui {
                *ui.progress.lock() = Some(Transfer { name: name.clone(), done: 0, total, upload: false });
            }
            *incoming.lock() = Some(IncomingFile { file, name, received: 0, total });
        }
        Err(e) => error!("no se pudo crear {}: {e}", path.display()),
    }
}

fn finish_incoming(incoming: &Arc<Mutex<Option<IncomingFile>>>, ui: &Option<Arc<FilesUi>>) {
    if let Some(i) = incoming.lock().take() {
        info!("archivo recibido: {} ({} bytes)", i.name, i.received);
        if let Some(ui) = ui {
            *ui.progress.lock() = None;
            *ui.last_msg.lock() = Some(format!("✓ recibido: {}", i.name));
        }
    }
}

// ---------------------------------------------------------------------------
// API para la GUI del explorador (lado operador)
// ---------------------------------------------------------------------------

/// Pide el listado de una carpeta remota (respuesta → `FilesUi.remote_list`).
pub fn browse_remote(dc: Arc<RTCDataChannel>, path: String) {
    tokio::spawn(async move {
        let msg = serde_json::json!({ "f": "ls", "path": path });
        let _ = dc.send_text(msg.to_string()).await;
    });
}

/// Descarga un archivo remoto a la carpeta local `save_into`.
pub fn fetch_remote(dc: Arc<RTCDataChannel>, path: String, save_into: PathBuf, ui: Arc<FilesUi>) {
    *ui.save_dir.lock() = Some(save_into);
    tokio::spawn(async move {
        let msg = serde_json::json!({ "f": "get", "path": path });
        let _ = dc.send_text(msg.to_string()).await;
    });
}

/// Sube un archivo local a la carpeta remota `remote_dir`.
pub fn upload_local(dc: Arc<RTCDataChannel>, local: PathBuf, remote_dir: String, ui: Arc<FilesUi>) {
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || match fs::read(&local) {
        Ok(data) => {
            let name = local
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "archivo".into());
            handle.spawn(async move {
                send_file_ex(dc, name, data, Some(remote_dir), Some(ui)).await;
            });
        }
        Err(e) => {
            *ui.last_msg.lock() = Some(format!("✗ no se pudo leer {}: {e}", local.display()));
        }
    });
}

// ---------------------------------------------------------------------------
// Envío
// ---------------------------------------------------------------------------

/// Abre un diálogo nativo y envía el archivo elegido por el canal. Lo usa tanto el
/// host al recibir "req" como el operador desde el botón "Enviar archivo".
pub fn pick_and_send(dc: Arc<RTCDataChannel>) {
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let picked = rfd::FileDialog::new()
            .set_title("Remotix: elige un archivo para enviar")
            .pick_file();
        let Some(path) = picked else {
            info!("envío de archivo cancelado por el usuario");
            return;
        };
        match fs::read(&path) {
            Ok(data) => {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "archivo".into());
                handle.spawn(async move { send_file_ex(dc, name, data, None, None).await });
            }
            Err(e) => warn!("no se pudo leer {}: {e}", path.display()),
        }
    });
}

/// Pide al otro lado que elija y envíe un archivo (el otro abre su diálogo).
pub async fn request_file(dc: Arc<RTCDataChannel>) {
    let _ = dc.send_text("{\"f\":\"req\"}".to_string()).await;
}

/// Envía un archivo por el canal. `dir` = carpeta destino en el receptor (la
/// eligió el operador navegando); `ui` = progreso para el explorador.
async fn send_file_ex(
    dc: Arc<RTCDataChannel>,
    name: String,
    data: Vec<u8>,
    dir: Option<String>,
    ui: Option<Arc<FilesUi>>,
) {
    let total = data.len() as u64;
    let begin = serde_json::to_string(&Begin {
        f: "begin",
        id: 1,
        name: &name,
        size: total,
        mime: "application/octet-stream",
        dir: dir.as_deref(),
    })
    .unwrap_or_default();
    if dc.send_text(begin).await.is_err() {
        return;
    }
    let mut sent: u64 = 0;
    for (i, chunk) in data.chunks(CHUNK).enumerate() {
        if dc.send(&Bytes::copy_from_slice(chunk)).await.is_err() {
            warn!("envío de archivo interrumpido");
            if let Some(ui) = &ui {
                *ui.progress.lock() = None;
                *ui.last_msg.lock() = Some(format!("✗ envío interrumpido: {name}"));
            }
            return;
        }
        sent += chunk.len() as u64;
        if let Some(ui) = &ui {
            *ui.progress.lock() = Some(Transfer { name: name.clone(), done: sent, total, upload: true });
        }
        // Backpressure suave para no saturar el buffer SCTP en archivos grandes.
        if i % 64 == 63 {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }
    let end = serde_json::to_string(&End { f: "end", id: 1 }).unwrap_or_default();
    let _ = dc.send_text(end).await;
    info!("archivo enviado: {} ({} bytes)", name, total);
    if let Some(ui) = &ui {
        *ui.progress.lock() = None;
        *ui.last_msg.lock() = Some(format!("✓ enviado: {name}"));
    }
}
