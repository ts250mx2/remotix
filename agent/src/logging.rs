//! Inicialización del logging del agente.
//!
//! En producción la app corre SIN consola (`windows_subsystem = "windows"`) y el
//! ayudante lo lanza el servicio sin terminal, así que los `tracing` a stderr no
//! se ven en ningún sitio. Por eso escribimos SIEMPRE a un archivo
//! (`%ProgramData%\Remotix\agent.log`) además de a stderr —útil para diagnosticar
//! por qué no conecta (candidatos ICE, TURN, escritorio de entrada, etc.)—.

use std::sync::Arc;

use parking_lot::Mutex;
use tracing_subscriber::prelude::*;

const MAX_LOG_BYTES: u64 = 3 * 1024 * 1024; // rotación simple al superar ~3 MB

/// Ruta del log. Preferimos %ProgramData% (compartida entre el servicio SYSTEM y
/// el ayudante de usuario; el servicio concede escritura a Usuarios); si no está,
/// el directorio de configuración del usuario.
fn log_path() -> std::path::PathBuf {
    let base = std::env::var("ProgramData")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = base.join("Remotix");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("agent.log")
}

fn open_log_file() -> Option<std::fs::File> {
    let path = log_path();
    // Rotación best-effort: si el archivo creció demasiado, empezar de cero.
    if std::fs::metadata(&path).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()
}

#[derive(Clone)]
struct FileMaker(Arc<Mutex<std::fs::File>>);

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileMaker {
    type Writer = FileHandle;
    fn make_writer(&'a self) -> Self::Writer {
        FileHandle(self.0.clone())
    }
}

struct FileHandle(Arc<Mutex<std::fs::File>>);

impl std::io::Write for FileHandle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().flush()
    }
}

fn env_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,remotix_agent=info"))
}

/// Inicializa tracing a stderr + archivo. Idempotente y best-effort: si ya hay un
/// subscriber, o no se puede abrir el archivo, no falla.
pub fn init() {
    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    match open_log_file() {
        Some(file) => {
            let file_layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(FileMaker(Arc::new(Mutex::new(file))));
            let _ = tracing_subscriber::registry()
                .with(env_filter())
                .with(stderr_layer)
                .with(file_layer)
                .try_init();
        }
        None => {
            let _ = tracing_subscriber::registry()
                .with(env_filter())
                .with(stderr_layer)
                .try_init();
        }
    }
}
