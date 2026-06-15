//! Transferencia de archivos por el DataChannel 'files' (bidireccional).
//!  - Recepción (operador → equipo): los archivos se guardan en %USERPROFILE%\Downloads\Remotix.
//!  - Envío (equipo → operador): ante una petición del operador, se abre un
//!    diálogo nativo para que el usuario elija el archivo a enviar.

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

#[derive(Deserialize)]
struct FileCtrl {
    f: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Serialize)]
struct Begin<'a> {
    f: &'a str,
    id: u64,
    name: &'a str,
    size: u64,
    mime: &'a str,
}

#[derive(Serialize)]
struct End {
    f: &'static str,
    id: u64,
}

struct Incoming {
    file: File,
    name: String,
    received: u64,
}

pub fn wire_files_channel(dc: Arc<RTCDataChannel>) {
    let incoming: Arc<Mutex<Option<Incoming>>> = Arc::new(Mutex::new(None));
    let dc_for_msg = dc.clone();

    dc.on_message(Box::new(move |msg: DataChannelMessage| {
        let incoming = incoming.clone();
        let dc = dc_for_msg.clone();
        Box::pin(async move {
            if msg.is_string {
                let text = String::from_utf8_lossy(&msg.data);
                if let Ok(ctrl) = serde_json::from_str::<FileCtrl>(&text) {
                    match ctrl.f.as_str() {
                        "begin" => start_incoming(&incoming, ctrl),
                        "end" => finish_incoming(&incoming),
                        "req" => pick_and_send(dc),
                        _ => {}
                    }
                }
            } else {
                let mut guard = incoming.lock();
                if let Some(i) = guard.as_mut() {
                    if i.file.write_all(&msg.data).is_ok() {
                        i.received += msg.data.len() as u64;
                    }
                }
            }
        })
    }));
}

fn target_dir() -> PathBuf {
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

fn start_incoming(incoming: &Arc<Mutex<Option<Incoming>>>, ctrl: FileCtrl) {
    let name = sanitize(ctrl.name.as_deref().unwrap_or("archivo"));
    let path = unique_path(target_dir().join(&name));
    match File::create(&path) {
        Ok(file) => {
            info!("recibiendo archivo → {} ({} bytes)", path.display(), ctrl.size.unwrap_or(0));
            *incoming.lock() = Some(Incoming { file, name, received: 0 });
        }
        Err(e) => error!("no se pudo crear {}: {e}", path.display()),
    }
}

fn finish_incoming(incoming: &Arc<Mutex<Option<Incoming>>>) {
    if let Some(i) = incoming.lock().take() {
        info!("archivo recibido: {} ({} bytes)", i.name, i.received);
    }
}

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
                handle.spawn(async move { send_file(dc, name, data).await });
            }
            Err(e) => warn!("no se pudo leer {}: {e}", path.display()),
        }
    });
}

/// Pide al otro lado que elija y envíe un archivo (el otro abre su diálogo).
pub async fn request_file(dc: Arc<RTCDataChannel>) {
    let _ = dc.send_text("{\"f\":\"req\"}".to_string()).await;
}

async fn send_file(dc: Arc<RTCDataChannel>, name: String, data: Vec<u8>) {
    let begin = serde_json::to_string(&Begin {
        f: "begin",
        id: 1,
        name: &name,
        size: data.len() as u64,
        mime: "application/octet-stream",
    })
    .unwrap_or_default();
    if dc.send_text(begin).await.is_err() {
        return;
    }
    for (i, chunk) in data.chunks(CHUNK).enumerate() {
        if dc.send(&Bytes::copy_from_slice(chunk)).await.is_err() {
            warn!("envío de archivo interrumpido");
            return;
        }
        // Backpressure suave para no saturar el buffer SCTP en archivos grandes.
        if i % 64 == 63 {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }
    let end = serde_json::to_string(&End { f: "end", id: 1 }).unwrap_or_default();
    let _ = dc.send_text(end).await;
    info!("archivo enviado al técnico: {} ({} bytes)", name, data.len());
}
