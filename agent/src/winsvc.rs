//! Servicio de Windows «Remotix»: arranque remoto desatendido estilo TeamViewer.
//!
//! Corre como LocalSystem con arranque automático (boot). Su trabajo:
//!   1. Asegurar que el equipo esté registrado y su clave fija persistida en
//!      HKLM —desde el arranque, antes de que nadie inicie sesión—.
//!   2. Garantizar que SIEMPRE haya un proceso «ayudante» corriendo en la sesión
//!      interactiva activa (allí es donde se puede capturar la pantalla e
//!      inyectar teclado/ratón):
//!        · usuario logueado  → se lanza en SU escritorio (CreateProcessAsUser
//!          con el token del usuario).
//!        · pantalla de login → se lanza como SYSTEM en `winsta0\Winlogon`.
//!   3. Seguir los cambios de sesión (login / logout / cambio rápido de usuario)
//!      relanzando el ayudante en la nueva sesión, y reponerlo si muere.
//!
//! Instalación/desinstalación (`install` / `uninstall`) requieren privilegios de
//! administrador y las invoca el instalador.
#![cfg(windows)]

use std::ffi::{c_void, OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    DuplicateTokenEx, SetTokenInformation, SecurityImpersonation, TokenPrimary, TokenSessionId,
    TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID, TOKEN_ALL_ACCESS, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_QUERY,
};
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcess, OpenProcessToken, TerminateProcess,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION,
    STARTUPINFOW,
};

use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

pub const SERVICE_NAME: &str = "Remotix";
pub const SERVICE_DISPLAY: &str = "Remotix (acceso remoto)";
pub const SERVICE_DESC: &str =
    "Mantiene este equipo accesible de forma remota y desatendida (Remotix).";

const WAIT_TIMEOUT_U32: u32 = 0x0000_0102;
const INVALID_SESSION: u32 = 0xFFFF_FFFF;

// ---------------------------------------------------------------------------
// Utilidades
// ---------------------------------------------------------------------------

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// Servidor efectivo: la config guardada manda; si no, el baked de compilación.
fn effective_server() -> String {
    if let Some(c) = crate::config::LiteConfig::load() {
        return c.server;
    }
    option_env!("REMOTIX_DEFAULT_SERVER")
        .unwrap_or("ws://localhost:8080")
        .to_string()
}

fn computer_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Equipo".into())
}

/// Log best-effort a %ProgramData%\Remotix\service.log (el servicio no tiene
/// consola). No falla nunca; solo ayuda a diagnosticar in situ.
fn log_line(msg: &str) {
    use std::io::Write;
    let dir = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".into());
    let dir = std::path::Path::new(&dir).join("Remotix");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("service.log"))
    {
        let _ = writeln!(f, "{msg}");
    }
}

// ---------------------------------------------------------------------------
// Instalación / desinstalación (requiere administrador)
// ---------------------------------------------------------------------------

/// Registra el servicio (AutoStart, LocalSystem) y lo arranca. Idempotente:
/// si ya existe, lo reconfigura al exe actual y se asegura de que esté corriendo.
pub fn install() -> Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;

    let exe = std::env::current_exe()?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec![OsString::from("service")],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };

    let access = ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::START
        | ServiceAccess::QUERY_STATUS
        | ServiceAccess::STOP;

    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(svc) => {
            // Ya existía: actualiza la ruta/args por si el exe se movió.
            let _ = svc.change_config(&info);
            svc
        }
        Err(_) => manager.create_service(&info, access)?,
    };
    let _ = service.set_description(SERVICE_DESC);

    // Arrancar ahora (ignora el error si ya está corriendo).
    let no_args: [&OsStr; 0] = [];
    let _ = service.start(&no_args);
    Ok(())
}

/// Detiene y elimina el servicio. Idempotente.
pub fn uninstall() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let access = ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(svc) => svc,
        Err(_) => return Ok(()), // no existe: nada que hacer
    };

    if let Ok(status) = service.query_status() {
        if status.current_state != ServiceState::Stopped {
            let _ = service.stop();
            for _ in 0..25 {
                std::thread::sleep(Duration::from_millis(200));
                if let Ok(s) = service.query_status() {
                    if s.current_state == ServiceState::Stopped {
                        break;
                    }
                }
            }
        }
    }
    service.delete()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Punto de entrada del servicio (lo llama el SCM)
// ---------------------------------------------------------------------------

define_windows_service!(ffi_service_main, service_main);

/// Conecta con el SCM. Solo tiene éxito cuando lo arranca el Administrador de
/// servicios; ejecutado a mano desde consola fallará (error 1063), es normal.
pub fn run() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|e| anyhow!("no se pudo iniciar el dispatcher del servicio: {e}"))
}

fn service_main(_args: Vec<OsString>) {
    if let Err(e) = run_service() {
        log_line(&format!("servicio terminó con error: {e:#}"));
    }
}

fn run_service() -> Result<()> {
    log_line("=== servicio Remotix arrancando ===");

    let shutdown = Arc::new(AtomicBool::new(false));
    let session_dirty = Arc::new(AtomicBool::new(true));

    let sd = shutdown.clone();
    let ssd = session_dirty.clone();
    let event_handler = move |control| -> ServiceControlHandlerResult {
        match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                sd.store(true, Ordering::SeqCst);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::SessionChange(_) => {
                ssd.store(true, Ordering::SeqCst);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    let running = |state: ServiceState, accept: ServiceControlAccept| ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };

    status_handle.set_service_status(running(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN | ServiceControlAccept::SESSION_CHANGE,
    ))?;

    // Permite que el ayudante (usuario) escriba el lock de sesión en ProgramData.
    grant_state_dir_access();

    // Registro de identidad (HKLM) en segundo plano: la clave existe desde el
    // arranque, aunque nadie haya iniciado sesión.
    {
        let server = effective_server();
        let name = computer_name();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    log_line(&format!("no se pudo crear runtime de registro: {e}"));
                    return;
                }
            };
            rt.block_on(crate::device::ensure_registered(&server, &name));
        });
    }

    // Bucle de seguimiento de sesión + auto-actualización.
    let mut current: Option<Helper> = None;
    let mut last_session: u32 = INVALID_SESSION;
    let mut ticks: u64 = 0;
    const FIRST_UPDATE_CHECK: u64 = 20; // ~30 s tras arrancar
    const UPDATE_EVERY: u64 = 1200; // ~30 min (bucle de 1.5 s)

    while !shutdown.load(Ordering::SeqCst) {
        let active = unsafe { WTSGetActiveConsoleSessionId() };
        let dirty = session_dirty.swap(false, Ordering::SeqCst);
        let helper_dead = current.as_ref().map(|h| !h.is_alive()).unwrap_or(true);

        if dirty || helper_dead || active != last_session {
            if let Some(h) = current.take() {
                h.kill();
            }
            // Sesión 0 = servicios (sin escritorio); 0xFFFFFFFF = sin consola.
            if active != 0 && active != INVALID_SESSION {
                match launch_helper_in_session(active) {
                    Ok(h) => {
                        log_line(&format!("ayudante lanzado en sesión {active} (pid {})", h.pid));
                        current = Some(h);
                        last_session = active;
                    }
                    Err(e) => {
                        log_line(&format!("no se pudo lanzar ayudante en sesión {active}: {e:#}"));
                        last_session = INVALID_SESSION; // reintentar en la próxima vuelta
                    }
                }
            } else {
                last_session = active;
            }
        }

        if ticks == FIRST_UPDATE_CHECK || (ticks > FIRST_UPDATE_CHECK && ticks % UPDATE_EVERY == 0) {
            maybe_auto_update();
        }
        ticks = ticks.wrapping_add(1);

        std::thread::sleep(Duration::from_millis(1500));
    }

    if let Some(h) = current.take() {
        h.kill();
    }

    status_handle.set_service_status(running(ServiceState::Stopped, ServiceControlAccept::empty()))?;
    log_line("=== servicio Remotix detenido ===");
    Ok(())
}

/// Concede a los usuarios permiso de modificación sobre %ProgramData%\Remotix,
/// para que el ayudante (que corre como usuario) pueda escribir el lock de
/// sesión que el servicio lee antes de auto-actualizar.
fn grant_state_dir_access() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let base = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".into());
    let dir = std::path::Path::new(&base).join("Remotix");
    let _ = std::fs::create_dir_all(&dir);
    // *S-1-5-32-545 = BUILTIN\Users; (OI)(CI)M = herencia + modificación.
    let _ = std::process::Command::new("icacls")
        .arg(dir.as_os_str())
        .args(["/grant", "*S-1-5-32-545:(OI)(CI)M", "/T", "/C", "/Q"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

/// Comprueba si hay una versión más nueva y, si no hay sesión activa (o es
/// obligatoria), la aplica. Bloquea unos segundos como mucho; se llama ~cada
/// 30 min, así que el coste es despreciable.
fn maybe_auto_update() {
    let server = effective_server();
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(_) => return,
    };
    let info = match rt.block_on(crate::update::check_latest_host(&server)) {
        Some(i) => i,
        None => return, // ya estamos al día (o no hay manifiesto del canal host)
    };
    // Actualización EN CALIENTE: ya no se pospone por sesión activa. El ayudante
    // mantiene un marcador de reanudación (resume-session.json) que el proceso
    // nuevo usa para re-hospedar la MISMA sala; el visor del técnico espera con
    // `host-reconnecting` y la sesión continúa sola tras unos segundos.
    if crate::update::session_active() {
        log_line(&format!(
            "actualización {} con sesión activa: se aplicará y la sesión se reanudará en caliente",
            info.version
        ));
    }
    log_line(&format!(
        "aplicando actualización {} → {}",
        crate::update::CURRENT_VERSION,
        info.version
    ));
    if let Err(e) = rt.block_on(crate::update::download_and_apply(&server, &info.url)) {
        log_line(&format!("fallo al aplicar actualización: {e:#}"));
    }
    // El instalador detendrá el servicio y lo reinstalará; el bucle terminará
    // cuando llegue el Stop del SCM.
}

// ---------------------------------------------------------------------------
// Lanzamiento del ayudante en la sesión interactiva activa
// ---------------------------------------------------------------------------

/// Proceso ayudante en curso; se monitoriza y se puede matar.
struct Helper {
    process: HANDLE,
    pid: u32,
}

impl Helper {
    fn is_alive(&self) -> bool {
        unsafe { WaitForSingleObject(self.process, 0) == WAIT_TIMEOUT_U32 }
    }
    fn kill(self) {
        unsafe {
            TerminateProcess(self.process, 0);
            WaitForSingleObject(self.process, 3000);
            CloseHandle(self.process);
        }
    }
}

/// Lanza `Remotix.exe helper` en la sesión indicada. Si hay un usuario logueado
/// usa su token (su escritorio); si no (pantalla de login), duplica el token
/// SYSTEM del servicio hacia esa sesión y usa el escritorio `Winlogon`.
fn launch_helper_in_session(session_id: u32) -> Result<Helper> {
    unsafe {
        let mut user_token: HANDLE = std::ptr::null_mut();
        let have_user = WTSQueryUserToken(session_id, &mut user_token) != 0;

        let (token, desktop) = if have_user {
            (user_token, "winsta0\\default")
        } else {
            let sys = duplicate_system_token_for_session(session_id)?;
            (sys, "winsta0\\Winlogon")
        };

        // Bloque de entorno del usuario/sesión (rutas, TEMP, etc.).
        let mut env: *mut c_void = std::ptr::null_mut();
        let have_env = CreateEnvironmentBlock(&mut env, token, 0) != 0;

        let exe = std::env::current_exe()?;
        let mut cmdline = wide(&format!("\"{}\" helper", exe.display()));
        let mut desktop_w = wide(desktop);

        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.lpDesktop = desktop_w.as_mut_ptr();

        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();

        let flags = CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW;
        let ok = CreateProcessAsUserW(
            token,
            std::ptr::null(),
            cmdline.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0, // no heredar handles
            flags,
            if have_env { env } else { std::ptr::null() },
            std::ptr::null(),
            &si,
            &mut pi,
        );

        if have_env {
            DestroyEnvironmentBlock(env);
        }
        CloseHandle(token);

        if ok == 0 {
            let err = windows_sys::Win32::Foundation::GetLastError();
            return Err(anyhow!("CreateProcessAsUser falló (GetLastError={err})"));
        }

        CloseHandle(pi.hThread);
        Ok(Helper { process: pi.hProcess, pid: pi.dwProcessId })
    }
}

/// Duplica el token SYSTEM del propio servicio a un token primario asignado a la
/// sesión `session_id` (para lanzar el ayudante en la pantalla de login).
unsafe fn duplicate_system_token_for_session(session_id: u32) -> Result<HANDLE> {
    let mut proc_token: HANDLE = std::ptr::null_mut();
    let ok = OpenProcessToken(
        GetCurrentProcess(),
        TOKEN_DUPLICATE
            | TOKEN_QUERY
            | TOKEN_ASSIGN_PRIMARY
            | TOKEN_ADJUST_DEFAULT
            | TOKEN_ADJUST_SESSIONID,
        &mut proc_token,
    );
    if ok == 0 {
        return Err(anyhow!("OpenProcessToken falló"));
    }

    let mut dup: HANDLE = std::ptr::null_mut();
    let ok = DuplicateTokenEx(
        proc_token,
        TOKEN_ALL_ACCESS,
        std::ptr::null(),
        SecurityImpersonation,
        TokenPrimary,
        &mut dup,
    );
    CloseHandle(proc_token);
    if ok == 0 {
        return Err(anyhow!("DuplicateTokenEx falló"));
    }

    let sid = session_id;
    let ok = SetTokenInformation(
        dup,
        TokenSessionId,
        &sid as *const u32 as *const c_void,
        std::mem::size_of::<u32>() as u32,
    );
    if ok == 0 {
        CloseHandle(dup);
        return Err(anyhow!("SetTokenInformation(SessionId) falló"));
    }
    Ok(dup)
}
