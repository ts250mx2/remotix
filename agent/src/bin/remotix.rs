//! Remotix — exe único cliente/servidor estilo TeamViewer.
//!
//!  - SIEMPRE es host: registra el equipo, mantiene presencia en /ws/device y, si
//!    alguien con acceso se conecta, comparte pantalla (run_remote_session).
//!  - SIN login: solo acepta conexiones (muestra su clave + estado).
//!  - CON login: además es operador: muestra la libreta de PCs accesibles y, al
//!    pulsar Conectar, abre el VISOR NATIVO (ve y controla la pantalla remota).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui;
use parking_lot::Mutex;
use tokio::sync::mpsc::UnboundedSender;
use tracing::warn;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use remotix_agent::account::{Account, DeviceInfo, UserInfo};
use remotix_agent::autostart;
use remotix_agent::config::{to_http, to_ws, LiteConfig};
use remotix_agent::device::run_lite_unattended;
use remotix_agent::files;
use remotix_agent::input::InputEvent;
use remotix_agent::session::LiteEvent;
use remotix_agent::update::{self, UpdateInfo};
use remotix_agent::viewer::{self, ViewerShared};

fn resolve_server() -> String {
    let baked = option_env!("REMOTIX_DEFAULT_SERVER").unwrap_or("ws://localhost:8080");
    if let Some(c) = LiteConfig::load() {
        if !c.server.is_empty() {
            return c.server;
        }
    }
    // Ni subcomandos ni flags (--tray) son URLs de servidor.
    std::env::var("REMOTIX_SERVER").ok()
        .or_else(|| std::env::args().skip(1).find(|a| *a != "console" && !a.starts_with('-')))
        .unwrap_or_else(|| baked.to_string())
}

fn format_key(k: &str) -> String {
    k.chars().collect::<Vec<_>>().chunks(3).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

/// Dominio "limpio" del servidor para mostrarlo en el pie (sin esquema ni /).
fn server_host(s: &str) -> String {
    s.trim_start_matches("wss://")
        .trim_start_matches("ws://")
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string()
}

// ---- Tema "terminal": verde fósforo sobre NEGRO PURO, estética de consola ----
use egui::Color32;
const BG: Color32 = Color32::BLACK;
const PANEL: Color32 = Color32::BLACK;
const CARD: Color32 = Color32::from_rgb(0x01, 0x04, 0x02);
const CARD_HI: Color32 = Color32::from_rgb(0x07, 0x16, 0x0C);
const BORDER: Color32 = Color32::from_rgb(0x14, 0x3B, 0x23);
// Texto principal BLANCO suave (legible, fino); los acentos van en verde.
const TEXT: Color32 = Color32::from_rgb(0xEC, 0xF4, 0xEE);
const MUTED: Color32 = Color32::from_rgb(0x74, 0xA2, 0x86);
const ACCENT: Color32 = Color32::from_rgb(0x00, 0xE6, 0x5A);
const ACCENT_HI: Color32 = Color32::from_rgb(0x53, 0xFF, 0x9C);
const GREEN: Color32 = Color32::from_rgb(0x2E, 0xFF, 0x7B);
const KEYC: Color32 = Color32::from_rgb(0x3D, 0xFF, 0x8E);
const REDC: Color32 = Color32::from_rgb(0xFF, 0x5C, 0x5C);
/// Brillo neón (sombra verde) para tarjetas y CTAs.
const GLOW: Color32 = Color32::from_rgba_premultiplied(0, 60, 24, 40);

/// Cursor de terminal: parpadeo ~1 Hz sincronizado con el reloj de egui.
fn blink_on(ui: &egui::Ui) -> bool {
    ui.input(|i| i.time) % 1.1 < 0.62
}

fn setup_style(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, Margin, Rounding, Stroke, TextStyle};
    load_system_fonts(ctx);
    // Tema SIEMPRE oscuro: sin esto egui sigue el tema del sistema y en Windows
    // claro pinta los controles (inputs/botones) en blanco sobre las tarjetas oscuras.
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut style = (*ctx.style()).clone();
    let mut v = egui::Visuals::dark();
    // Esquinas casi rectas: estética de terminal, no de web app.
    let rnd = Rounding::same(3.0);
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::BLACK; // inputs: negro puro, los delimita el borde
    v.faint_bg_color = CARD;
    v.hyperlink_color = ACCENT_HI;
    v.selection.bg_fill = Color32::from_rgb(0x0C, 0x3A, 0x1E);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    let set = |w: &mut egui::style::WidgetVisuals, fill: Color32, stroke: Color32, fg: Color32| {
        w.bg_fill = fill; w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, stroke);
        w.fg_stroke = Stroke::new(1.0, fg);
        w.rounding = rnd;
    };
    set(&mut v.widgets.noninteractive, PANEL, BORDER, TEXT);
    set(&mut v.widgets.inactive, CARD_HI, BORDER, ACCENT_HI);
    set(&mut v.widgets.hovered, Color32::from_rgb(0x11, 0x2E, 0x1B), ACCENT, ACCENT_HI);
    set(&mut v.widgets.active, ACCENT, ACCENT, Color32::BLACK);
    set(&mut v.widgets.open, CARD_HI, BORDER, TEXT);
    style.visuals = v;
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 32.0;
    style.spacing.window_margin = Margin::same(0.0);
    // Texto normal en la proporcional FINA (blanca); lo mono queda para los
    // acentos de terminal (títulos [ ], claves, // comentarios, estado).
    style.text_styles.insert(TextStyle::Heading, FontId::new(19.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Button, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Small, FontId::new(12.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace));
    ctx.set_style(style);
}

/// Tipografías: proporcional FINA (Segoe UI Semilight/Light) para el texto y
/// Cascadia/Consolas para los acentos mono de terminal. Si algún archivo no
/// existe se cae al siguiente candidato (y al final a la fuente de egui).
fn load_system_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily};
    let mut fonts = FontDefinitions::default();
    let win = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let mut any = false;
    let mut load_first = |name: &str, files: &[&str], family: FontFamily| {
        for file in files {
            let path = std::path::Path::new(&win).join("Fonts").join(file);
            if let Ok(bytes) = std::fs::read(&path) {
                fonts.font_data.insert(name.to_string(), FontData::from_owned(bytes));
                if let Some(fam) = fonts.families.get_mut(&family) {
                    fam.insert(0, name.to_string());
                    any = true;
                }
                return;
            }
        }
    };
    // OJO con los nombres de archivo de Segoe: segoeuisl = Semilight recto;
    // seguisli = Semilight ITALIC (no confundir, inclina toda la UI).
    load_first("ui-thin", &["segoeuisl.ttf", "segoeuil.ttf", "segoeui.ttf"], FontFamily::Proportional);
    load_first("term", &["CascadiaMono.ttf", "CascadiaCode.ttf", "consola.ttf"], FontFamily::Monospace);
    if any {
        ctx.set_fonts(fonts);
    }
}

/// Normaliza lo tecleado a la forma XXX-XXX-XXX (alfanumérico, mayúsculas, 9).
fn normalize_key_input(s: &str) -> String {
    let clean: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .take(9)
        .collect();
    format_key(&clean)
}

/// Panel de terminal: fondo casi negro, borde verde de 1 px y glow neón.
fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::same(18.0))
        .shadow(egui::epaint::Shadow {
            offset: egui::vec2(0.0, 0.0),
            blur: 18.0,
            spread: 1.0,
            color: GLOW,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// Encabezado de sección estilo consola: `[ TITULO ]` mono verde + comentario
/// `//` mono apagado (el cuerpo de las tarjetas va en la proporcional fina).
fn card_title(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(
        egui::RichText::new(format!("[ {} ]", title.to_uppercase()))
            .monospace()
            .size(13.5)
            .strong()
            .color(ACCENT_HI),
    );
    ui.label(egui::RichText::new(format!("// {subtitle}")).monospace().size(10.5).color(MUTED));
    ui.add_space(12.0);
}

/// Botón secundario estilo consola: contorno verde fino, sin relleno.
fn primary(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let btn = egui::Button::new(
        egui::RichText::new(format!("[ {text} ]")).monospace().size(11.5).color(if enabled { ACCENT_HI } else { MUTED }),
    )
    .fill(Color32::TRANSPARENT)
    .stroke(egui::Stroke::new(1.0, if enabled { ACCENT } else { BORDER }))
    .rounding(egui::Rounding::same(3.0))
    .min_size(egui::vec2(0.0, 28.0));
    ui.add_enabled(enabled, btn)
}

/// Botón primario a lo ancho de la tarjeta (acción principal). Deshabilitado se
/// pinta verde apagado con borde (se lee "botón, aún sin datos", no un input).
/// El layout centrado es clave: sin él, egui ancla el texto a la izquierda y el
/// botón ancho parece un campo de texto.
fn primary_wide(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let (fill, fg, border) = if enabled {
        (ACCENT, Color32::BLACK, ACCENT_HI)
    } else {
        (Color32::from_rgb(0x0A, 0x1C, 0x11), Color32::from_rgb(0x3E, 0x6B, 0x4E), BORDER)
    };
    let btn = egui::Button::new(egui::RichText::new(text).monospace().size(13.5).color(fg).strong())
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, border))
        .rounding(egui::Rounding::same(3.0));
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 40.0),
        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        |ui| ui.add_enabled(enabled, btn),
    )
    .inner
}

/// Fila con resaltado al pasar el mouse (para listas tipo "Mis PCs").
fn hover_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| add(ui));
        });
    let rect = inner.response.rect;
    if ui.rect_contains_pointer(rect) {
        ui.painter().set(
            bg,
            egui::Shape::rect_filled(rect, egui::Rounding::same(8.0), CARD_HI),
        );
    }
}

fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(11.0, 11.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

fn muted(s: &str) -> egui::RichText {
    egui::RichText::new(s).color(MUTED)
}

/// Icono de la ventana: "pantalla" negra con marco y prompt verde neón (>_).
fn app_icon() -> egui::IconData {
    let (w, h) = (64usize, 64usize);
    let mut rgba = vec![0u8; w * h * 4];
    let set = |rgba: &mut Vec<u8>, x: usize, y: usize, c: [u8; 3]| {
        let i = (y * w + x) * 4;
        rgba[i] = c[0]; rgba[i + 1] = c[1]; rgba[i + 2] = c[2]; rgba[i + 3] = 255;
    };
    for y in 0..h {
        for x in 0..w {
            let border = x < 4 || y < 4 || x >= w - 4 || y >= h - 4;
            set(&mut rgba, x, y, if border { [0x00, 0xE6, 0x5A] } else { [0x03, 0x08, 0x05] });
        }
    }
    // ">" (dos trazos diagonales gruesos) + "_" (guion bajo), verde neón.
    for t in 0..14usize {
        for dy in 0..5 {
            for dx in 0..5 {
                set(&mut rgba, 14 + t + dx, 18 + t + dy, [0x3D, 0xFF, 0x8E]); // \
                set(&mut rgba, 14 + t + dx, 46 - t + dy - 5, [0x3D, 0xFF, 0x8E]); // /
            }
        }
    }
    for x in 38..54 {
        for y in 44..49 {
            set(&mut rgba, x, y, [0x3D, 0xFF, 0x8E]);
        }
    }
    egui::IconData { rgba, width: w as u32, height: h as u32 }
}

/// Icono en la bandeja del sistema. Se mantiene vivo mientras exista; al soltarlo
/// (Drop) el icono desaparece de la bandeja.
struct TrayState {
    _tray: TrayIcon,
}

/// Bandera compartida: la bandeja pide mostrar la ventana. La GUI la lee para
/// sincronizar su estado con egui (la ventana ya se mostró vía Win32).
type TrayQueue = Arc<Mutex<bool>>;

/// Busca la ventana top-level "Remotix". Con `own_process` solo mira las de
/// ESTE proceso (bandeja/visibilidad); si es false exige que el proceso dueño
/// sea un remotix*.exe — así una ventana ajena que casualmente se titule
/// "Remotix" (p. ej. el Explorador en una carpeta con ese nombre) jamás
/// engaña al guard de instancia única.
#[cfg(windows)]
fn find_remotix_window(own_process: bool) -> windows_sys::Win32::Foundation::HWND {
    use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowThreadProcessId,
    };

    struct Search {
        own: bool,
        found: HWND,
    }

    unsafe extern "system" fn cb(hwnd: HWND, l: LPARAM) -> BOOL {
        let s = unsafe { &mut *(l as *mut Search) };
        let mut title = [0u16; 32];
        let n = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
        if n <= 0 || String::from_utf16_lossy(&title[..n as usize]) != "Remotix" {
            return 1;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        let matches = if s.own {
            pid == unsafe { GetCurrentProcessId() }
        } else {
            let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if h.is_null() {
                return 1;
            }
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            let ok = unsafe { QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) };
            unsafe { CloseHandle(h) };
            ok != 0 && {
                let path = String::from_utf16_lossy(&buf[..len as usize]).to_ascii_lowercase();
                path.rsplit('\\').next().map(|f| f.starts_with("remotix")).unwrap_or(false)
            }
        };
        if matches {
            s.found = hwnd;
            0
        } else {
            1
        }
    }

    let mut s = Search { own: own_process, found: std::ptr::null_mut() };
    unsafe { EnumWindows(Some(cb), &mut s as *mut Search as LPARAM) };
    s.found
}

/// Muestra y trae al frente una ventana (Win32). Se usa desde la bandeja y
/// desde el guard de instancia única: con la ventana oculta eframe NO ejecuta
/// su bucle, así que hay que hacerlo con la API del SO.
#[cfg(windows)]
fn show_window(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    };
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
    }
}

/// Muestra la ventana principal de ESTE proceso (handler de la bandeja).
#[cfg(windows)]
fn show_main_window() {
    let hwnd = find_remotix_window(true);
    if !hwnd.is_null() {
        show_window(hwnd);
    }
}
#[cfg(not(windows))]
fn show_main_window() {}

/// ¿La ventana principal está visible AHORA? (estado real del SO, no un flag:
/// cubre el caso "otra instancia me mostró la ventana por Win32" para que la
/// auto-actualización nunca cierre la app mientras alguien la usa.)
#[cfg(windows)]
fn own_window_visible() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    let hwnd = find_remotix_window(true);
    !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } != 0
}
#[cfg(not(windows))]
fn own_window_visible() -> bool {
    true
}

/// Pinta OSCURA la barra de título de las ventanas "Remotix*" de este proceso
/// (DWM). Corre en un hilo de fondo toda la vida del proceso porque la ventana
/// principal se crea después y la del visor puede aparecer en cualquier
/// momento; sin esto la barra sale blanca y desentona con la app.
#[cfg(windows)]
fn spawn_dark_titlebar_thread() {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW};

    unsafe extern "system" fn apply(hwnd: HWND, _l: LPARAM) -> BOOL {
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        // Solo ventanas de ESTE proceso: sin el check se oscurecería la barra
        // de cualquier ventana ajena titulada "Remotix…" (p. ej. el Explorador).
        let mut pid = 0u32;
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid != unsafe { GetCurrentProcessId() } {
            return 1;
        }
        let mut buf = [0u16; 64];
        let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        if n > 0 && String::from_utf16_lossy(&buf[..n as usize]).starts_with("Remotix") {
            let dark: i32 = 1;
            unsafe {
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                    &dark as *const i32 as *const _,
                    std::mem::size_of::<i32>() as u32,
                );
            }
        }
        1
    }
    std::thread::spawn(|| loop {
        unsafe { EnumWindows(Some(apply), 0) };
        std::thread::sleep(Duration::from_millis(800));
    });
}
#[cfg(not(windows))]
fn spawn_dark_titlebar_thread() {}

fn make_tray_icon() -> Option<tray_icon::Icon> {
    let (w, h) = (32usize, 32usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let border = x < 2 || y < 2 || x >= w - 2 || y >= h - 2;
            let c: [u8; 3] = if border { [0x00, 0xE6, 0x5A] } else { [0x03, 0x08, 0x05] };
            rgba[i] = c[0]; rgba[i + 1] = c[1]; rgba[i + 2] = c[2]; rgba[i + 3] = 255;
        }
    }
    // ">" verde centrado.
    for t in 0..7usize {
        for dy in 0..3 {
            for dx in 0..3 {
                let (x1, y1) = (8 + t + dx, 9 + t + dy);
                let (x2, y2) = (8 + t + dx, 23 - t + dy - 3);
                for (x, y) in [(x1, y1), (x2, y2)] {
                    let i = (y * w + x) * 4;
                    rgba[i] = 0x3D; rgba[i + 1] = 0xFF; rgba[i + 2] = 0x8E; rgba[i + 3] = 255;
                }
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, w as u32, h as u32).ok()
}

/// Crea el icono de la bandeja. Sus handlers actúan de inmediato con Win32
/// (mostrar) o `exit` (salir), sin depender del bucle de eframe —que no corre
/// mientras la ventana está oculta—. Por eso ahora "Abrir" y "Salir" funcionan.
fn create_tray(ctx: &egui::Context) -> Option<(TrayState, TrayQueue)> {
    let menu = Menu::new();
    let show = MenuItem::new("Abrir Remotix", true, None);
    let quit = MenuItem::new("Salir", true, None);
    menu.append(&show).ok()?;
    menu.append(&quit).ok()?;
    let show_id = show.id().clone();
    let quit_id = quit.id().clone();

    let mut builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Remotix");
    if let Some(icon) = make_tray_icon() {
        builder = builder.with_icon(icon);
    }
    let tray = builder.build().ok()?;

    // Bandera Show para sincronizar el estado de egui en el siguiente frame. El
    // handler debe ser Send+Sync, por eso Arc<Mutex<…>> (no un canal).
    let queue: TrayQueue = Arc::new(Mutex::new(false));

    // Menú: "Abrir" muestra la ventana (Win32) y "Salir" cierra de verdad (exit).
    let ctx_m = ctx.clone();
    let q_m = queue.clone();
    MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
        if ev.id == show_id {
            show_main_window();
            *q_m.lock() = true;
            ctx_m.request_repaint();
        } else if ev.id == quit_id {
            std::process::exit(0);
        }
    }));

    // Clic izquierdo (simple o doble) en el icono → abrir. El derecho abre el menú.
    let ctx_t = ctx.clone();
    let q_t = queue.clone();
    TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| {
        let open = matches!(
            ev,
            TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }
        );
        if open {
            show_main_window();
            *q_t.lock() = true;
            ctx_t.request_repaint();
        }
    }));

    Some((TrayState { _tray: tray }, queue))
}

/// Modo headless: corre solo el visor contra un código y reporta si llegan frames
/// decodificados (para E2E automatizado, sin abrir ventana).
fn run_viewer_console(server: &str, code: &str) -> Result<()> {
    if code.is_empty() {
        eprintln!("uso: remotix console <code>");
        std::process::exit(2);
    }
    let rt = tokio::runtime::Runtime::new()?;
    let (shared, _input_tx, input_rx) = viewer::new_session();
    let http = to_http(server);
    let ws = to_ws(server, "/ws/signal");
    let shared2 = shared.clone();
    let codeb = code.to_string();
    rt.spawn(async move {
        if let Err(e) = viewer::run_viewer_session(&http, &ws, &codeb, shared2, input_rx).await {
            eprintln!("viewer error: {e:#}");
        }
    });

    let mut seen = 0usize;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(500));
        let dims = shared.frame.lock().as_ref().map(|f| (f.w, f.h));
        let st = shared.status.lock().clone();
        match dims {
            Some((w, h)) => { seen += 1; println!("FRAME {w}x{h} status={st}"); }
            None => println!("STATUS {st}"),
        }
        if shared.closed.load(Ordering::SeqCst) { println!("CLOSED"); break; }
    }
    println!("DONE frames_seen={seen}");
    std::process::exit(if seen > 0 { 0 } else { 1 });
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,remotix_agent=info")),
        )
        .init();

    let server = resolve_server();
    let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Equipo".into());

    // Modo headless de prueba del visor: `remotix console <code>` (E2E sin GUI).
    let raw_args: Vec<String> = std::env::args().collect();
    if let Some(pos) = raw_args.iter().position(|a| a == "console") {
        let code = raw_args.get(pos + 1).cloned().unwrap_or_default();
        return run_viewer_console(&server, &code);
    }

    // Instancia única (mutex con nombre, sin carreras): si ya hay un Remotix
    // corriendo (aunque esté oculto en la bandeja), traemos SU ventana al frente
    // y salimos. Así "abrir Remotix" desde el menú Inicio / barra de tareas
    // restaura la instancia existente en vez de duplicar proceso y bandeja.
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let name: Vec<u16> = "Local\\Remotix-App-SingleInstance\0".encode_utf16().collect();
        // El handle se conserva toda la vida del proceso (lo libera el SO al salir).
        let _mutex = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            let existing = find_remotix_window(false);
            if !existing.is_null() {
                show_window(existing);
            }
            return Ok(());
        }
    }

    // Barra de título oscura (DWM) para la ventana principal y el visor.
    spawn_dark_titlebar_thread();

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let handle = rt.handle().clone();

    // Rol host: siempre activo (registra + presencia + acepta conexiones).
    let (host_tx, host_rx) = std::sync::mpsc::channel::<LiteEvent>();
    rt.spawn(run_lite_unattended(server.clone(), name.clone(), host_tx));

    // Sesión persistida (login del usuario en este equipo).
    let saved = LiteConfig::load();
    let token = saved.as_ref().and_then(|c| c.session_token.clone());
    let email0 = saved.as_ref().and_then(|c| c.user_email.clone()).unwrap_or_default();
    let account = Arc::new(tokio::sync::Mutex::new(Account::new(&server, token.clone())));
    let ui = Arc::new(UiShared::default());

    // `--tray`: arranca oculto en la bandeja (lo usa el relanzamiento tras una
    // auto-actualización silenciosa, para no plantar la ventana de repente).
    let start_hidden = raw_args.iter().any(|a| a == "--tray");

    // Refresca la entrada de autoarranque (ruta del exe + --tray): migra
    // instalaciones viejas que arrancaban con la ventana visible en cada login.
    if autostart::is_autostart() {
        let _ = autostart::set_autostart(true);
    }

    // Revalida el token persistido en segundo plano.
    if token.is_some() {
        let account = account.clone();
        let ui = ui.clone();
        handle.spawn(async move {
            let acc = account.lock().await;
            match acc.me().await {
                Ok(Some(user)) => {
                    let devs = acc.devices().await.unwrap_or_default();
                    drop(acc);
                    *ui.user.lock() = Some(user);
                    *ui.devices.lock() = devs;
                }
                // Solo el rechazo EXPLÍCITO del servidor borra el token; un fallo
                // de red al arrancar no debe desloguear al usuario.
                Ok(None) => LiteConfig::set_session(None, None),
                Err(_) => {}
            }
        });
    }

    // Comprobación periódica de nueva versión. Es el respaldo del push del
    // servidor (LiteEvent::UpdateAvailable); si la app está inactiva se
    // actualiza sola, si no deja la tarjeta "Actualizar ahora".
    {
        let ui = ui.clone();
        let server = server.clone();
        handle.spawn(async move {
            loop {
                check_update_and_maybe_apply(&server, &ui).await;
                tokio::time::sleep(Duration::from_secs(1800)).await;
            }
        });
    }

    let mut app = RemotixApp {
        rt: handle,
        account,
        ui,
        host_rx,
        host_code: None,
        host_status: "Iniciando…".into(),
        autostart: autostart::is_autostart(),
        email: email0,
        password: String::new(),
        key_input: String::new(),
        key_copied_at: None,
        filter: String::new(),
        viewer: None,
        last_refresh: Instant::now(),
        server,
        tray: None,
        tray_queue: None,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([780.0, 680.0])
            .with_min_inner_size([560.0, 520.0])
            .with_icon(std::sync::Arc::new(app_icon()))
            .with_visible(!start_hidden)
            .with_title("Remotix"),
        ..Default::default()
    };
    eframe::run_native("Remotix", options, Box::new(move |cc| {
        setup_style(&cc.egui_ctx);
        // La bandeja se crea aquí (hilo principal, con el contexto ya disponible
        // para despertar el bucle al recibir sus eventos).
        if let Some((tray, queue)) = create_tray(&cc.egui_ctx) {
            app.tray = Some(tray);
            app.tray_queue = Some(queue);
        }
        Ok(Box::new(app))
    }))
    .map_err(|e| anyhow::anyhow!("error de la ventana: {e}"))?;
    // `rt` se mantiene vivo hasta aquí (no soltar antes de cerrar la ventana).
    drop(rt);
    Ok(())
}

/// Estado compartido entre la GUI (hilo principal) y las tareas async.
#[derive(Default)]
struct UiShared {
    user: Mutex<Option<UserInfo>>,
    devices: Mutex<Vec<DeviceInfo>>,
    error: Mutex<Option<String>>,
    pending_viewer: Mutex<Option<ActiveViewer>>,
    busy: AtomicBool,
    /// Versión más nueva detectada en el servidor (auto-actualización).
    update: Mutex<Option<UpdateInfo>>,
    /// Hay una ventana de visor abierta (no auto-actualizar en medio).
    viewer_active: AtomicBool,
}

/// Consulta si hay versión nueva y, si la app está INACTIVA (oculta en la
/// bandeja, sin visor abierto y sin sesión remota en curso), la aplica sola y
/// reinicia. Si está en uso, solo deja la tarjeta "Actualizar ahora" a la vista.
/// Con `mandatory: true` en el manifiesto se aplica aunque la ventana esté
/// visible (pero nunca con una sesión remota o un visor activos). La
/// visibilidad se consulta al SO en vivo (no a un flag) para no cerrarle la app
/// a alguien que la reabrió por una vía inesperada.
async fn check_update_and_maybe_apply(server: &str, ui: &Arc<UiShared>) {
    let Some(info) = update::check_latest(server).await else { return };
    *ui.update.lock() = Some(info.clone());
    let idle = !update::session_active()
        && !ui.viewer_active.load(Ordering::SeqCst)
        && (!own_window_visible() || info.mandatory);
    if idle && update::download_and_apply(server, &info.url).await.is_ok() {
        // El instalador cierra este proceso, reemplaza el exe y relanza con --tray.
        std::process::exit(0);
    }
}

/// Sesión de visor activa (una ventana de pantalla remota).
struct ActiveViewer {
    shared: Arc<ViewerShared>,
    input_tx: UnboundedSender<InputEvent>,
    name: String,
    tex: Option<egui::TextureHandle>,
    size: [usize; 2],
    prev_mods: egui::Modifiers,
    fullscreen: bool,
}

enum Action {
    Login,
    Logout,
    Connect(DeviceInfo),
    ConnectByKey(String),
    ToggleAutostart(bool),
    Update(UpdateInfo),
}

struct RemotixApp {
    rt: tokio::runtime::Handle,
    account: Arc<tokio::sync::Mutex<Account>>,
    ui: Arc<UiShared>,
    host_rx: Receiver<LiteEvent>,
    host_code: Option<String>,
    host_status: String,
    autostart: bool,
    email: String,
    password: String,
    key_input: String,
    /// Momento del último "copiar clave" (para el feedback "Copiada ✔").
    key_copied_at: Option<Instant>,
    /// Filtro de búsqueda de la lista "Mis PCs".
    filter: String,
    viewer: Option<ActiveViewer>,
    last_refresh: Instant,
    server: String,
    tray: Option<TrayState>,
    tray_queue: Option<TrayQueue>,
}

impl RemotixApp {
    fn do_login(&self) {
        let email = self.email.trim().to_string();
        let password = self.password.clone();
        let access_key = self.host_code.clone();
        let account = self.account.clone();
        let ui = self.ui.clone();
        self.rt.spawn(async move {
            ui.busy.store(true, Ordering::SeqCst);
            *ui.error.lock() = None;
            let mut acc = account.lock().await;
            match acc.login(&email, &password).await {
                Ok(user) => {
                    LiteConfig::set_session(acc.token(), Some(user.email.clone()));
                    if let Some(k) = &access_key {
                        let _ = acc.claim(k).await; // reclama este equipo si está libre
                    }
                    let devs = acc.devices().await.unwrap_or_default();
                    drop(acc);
                    *ui.user.lock() = Some(user);
                    *ui.devices.lock() = devs;
                }
                Err(e) => { *ui.error.lock() = Some(e.to_string()); }
            }
            ui.busy.store(false, Ordering::SeqCst);
        });
    }

    // Descarga el instalador de la versión nueva y lo ejecuta; el instalador
    // reemplaza esta versión en sitio y relanza la app. Cerramos esta instancia
    // para liberar el exe.
    fn do_update(&self, info: UpdateInfo) {
        let server = self.server.clone();
        let ui = self.ui.clone();
        self.rt.spawn(async move {
            *ui.error.lock() = None;
            match update::download_and_apply(&server, &info.url).await {
                Ok(()) => std::process::exit(0),
                Err(e) => { *ui.error.lock() = Some(format!("No se pudo actualizar: {e}")); }
            }
        });
    }

    fn do_logout(&self) {
        let account = self.account.clone();
        let ui = self.ui.clone();
        self.rt.spawn(async move {
            let mut acc = account.lock().await;
            let _ = acc.logout().await;
            drop(acc);
            LiteConfig::set_session(None, None);
            *ui.user.lock() = None;
            *ui.devices.lock() = Vec::new();
        });
    }

    fn refresh_devices(&self) {
        let account = self.account.clone();
        let ui = self.ui.clone();
        self.rt.spawn(async move {
            let acc = account.lock().await;
            match acc.devices().await {
                Ok(devs) => {
                    drop(acc);
                    *ui.devices.lock() = devs;
                }
                // El servidor invalidó la sesión: volver al login en vez de
                // quedarse con una libreta congelada que ya no funciona.
                Err(e) if e.to_string().contains("(401)") => {
                    drop(acc);
                    LiteConfig::set_session(None, None);
                    *ui.user.lock() = None;
                    *ui.devices.lock() = Vec::new();
                    *ui.error.lock() = Some("Tu sesión expiró; vuelve a iniciar sesión.".into());
                }
                Err(_) => {}
            }
        });
    }

    // Conectar a un equipo de la libreta (por id, con acceso).
    fn do_connect(&self, dev: DeviceInfo) {
        let account = self.account.clone();
        let ui = self.ui.clone();
        let server = self.server.clone();
        self.rt.spawn(async move {
            let acc = account.lock().await;
            let res = acc.connect(&dev.id).await;
            drop(acc);
            match res {
                Ok(code) => spawn_viewer(&ui, &server, code, dev.name.clone()),
                Err(e) => { *ui.error.lock() = Some(e.to_string()); }
            }
        });
    }

    // Conectar por clave (ad-hoc): equipos sin dueño se aceptan solo con la clave.
    fn do_connect_by_key(&self, key: String) {
        let account = self.account.clone();
        let ui = self.ui.clone();
        let server = self.server.clone();
        self.rt.spawn(async move {
            let acc = account.lock().await;
            let res = acc.connect_by_key(&key).await;
            drop(acc);
            match res {
                Ok((code, name)) => {
                    let label = if name.is_empty() { format!("clave {key}") } else { name };
                    spawn_viewer(&ui, &server, code, label);
                }
                Err(e) => { *ui.error.lock() = Some(e.to_string()); }
            }
        });
    }

    // ---- Tarjetas de la pantalla principal. Separadas en métodos para poder
    // maquetarlas en columna fija (izquierda) / elástica (derecha) sin duplicar
    // contenido entre el modo ancho y el apilado. ----

    /// [ CONECTAR A OTRA PC ]: clave del equipo remoto + CTA.
    fn ui_connect_card(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        card(ui, |ui| {
            card_title(ui, "Conectar a otra PC", "Escribe la clave del equipo remoto para verlo y controlarlo");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.key_input)
                    .hint_text("ABC-234-XYZ")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Margin::symmetric(12.0, 10.0)),
            );
            if resp.changed() {
                let fixed = normalize_key_input(&self.key_input);
                if fixed != self.key_input {
                    self.key_input = fixed;
                    // Reponer el cursor al final: al reescribir el texto
                    // (guiones/mayúsculas) el índice guardado queda desfasado y
                    // lo tecleado se insertaría desordenado.
                    if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                        let end = egui::text::CCursor::new(self.key_input.chars().count());
                        st.cursor.set_char_range(Some(egui::text_selection::CCursorRange::one(end)));
                        st.store(ui.ctx(), resp.id);
                    }
                }
            }
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(10.0);
            let ready = self.key_input.chars().filter(|c| c.is_ascii_alphanumeric()).count() == 9;
            if primary_wide(ui, "▶  CONECTAR", ready).clicked() || (enter && ready) {
                action = Some(Action::ConnectByKey(self.key_input.trim().to_string()));
            }
            ui.add_space(6.0);
            ui.label(egui::RichText::new("// P2P · cifrado de extremo a extremo").monospace().size(11.0).color(MUTED));
        });
        action
    }

    /// [ ESTE EQUIPO ]: la clave propia (prompt con cursor) + autoarranque.
    fn ui_host_card(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        card(ui, |ui| {
            card_title(ui, "Este equipo", "Comparte esta clave con quien deba conectarse aquí");
            match &self.host_code {
                Some(c) => {
                    let key = format_key(c);
                    // Prompt con la clave y cursor de bloque parpadeante.
                    let mut copy_now = false;
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(">").monospace().size(26.0).color(MUTED));
                        let klabel = ui
                            .add(
                                egui::Label::new(
                                    egui::RichText::new(&key).monospace().size(29.0).strong().color(KEYC),
                                )
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("Clic para copiar");
                        copy_now = klabel.clicked();
                        let cur = if blink_on(ui) { KEYC } else { Color32::TRANSPARENT };
                        ui.label(egui::RichText::new("█").monospace().size(26.0).color(cur));
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let copied = self.key_copied_at.map(|t| t.elapsed().as_millis() < 1600).unwrap_or(false);
                        let label = if copied { "[ COPIADA ✓ ]" } else { "[ COPIAR CLAVE ]" };
                        let btn = egui::Button::new(egui::RichText::new(label).monospace().size(11.5).color(ACCENT_HI))
                            .fill(Color32::TRANSPARENT)
                            .stroke(egui::Stroke::new(1.0, BORDER))
                            .rounding(egui::Rounding::same(3.0))
                            .min_size(egui::vec2(130.0, 26.0));
                        copy_now |= ui.add(btn).clicked();
                    });
                    if copy_now {
                        ui.ctx().copy_text(key);
                        self.key_copied_at = Some(Instant::now());
                    }
                }
                None => {
                    ui.label(egui::RichText::new("> ···-···-···").monospace().size(28.0).color(MUTED));
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("// obteniendo clave…").monospace().size(10.5).color(MUTED));
                }
            }
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(4.0);
            if ui.checkbox(&mut self.autostart, "Iniciar con Windows").changed() {
                action = Some(Action::ToggleAutostart(self.autostart));
            }
            ui.label(egui::RichText::new("// al cerrar la ventana sigue activo en la bandeja").monospace().size(10.5).color(MUTED));
        });
        action
    }

    /// [ MIS PCS ] (o [ CUENTA ] sin sesión). `min_h`: altura interna mínima —
    /// así el panel llena todo el alto disponible cuando la ventana crece.
    fn ui_right_card(
        &mut self,
        ui: &mut egui::Ui,
        logged_in: bool,
        devices: &[DeviceInfo],
        busy: bool,
        min_h: Option<f32>,
    ) -> Option<Action> {
        let mut action = None;
        card(ui, |ui| {
            if let Some(h) = min_h {
                ui.set_min_height(h);
            }
            if logged_in {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("[ MIS PCS ]").monospace().size(13.5).strong().color(ACCENT_HI));
                    let online = devices.iter().filter(|d| d.online).count();
                    ui.label(egui::RichText::new(format!("{online}/{} online", devices.len())).monospace().size(11.0).color(MUTED));
                });
                // Visible también si hay un filtro puesto, para poder quitarlo
                // aunque la lista haya bajado de tamaño.
                if devices.len() > 5 || !self.filter.is_empty() {
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("buscar…").desired_width(f32::INFINITY));
                }
                ui.add_space(10.0);
                if devices.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(30.0);
                        ui.label(egui::RichText::new(">_").monospace().size(26.0).color(MUTED));
                        ui.label(muted("// sin PCs guardadas"));
                        ui.label(egui::RichText::new("Instala Remotix en otro equipo e inicia sesión ahí, o agrégala por su clave en el portal web.").size(11.5).color(MUTED));
                    });
                }
                let f = self.filter.trim().to_lowercase();
                let shown: Vec<&DeviceInfo> = devices
                    .iter()
                    .filter(|d| f.is_empty() || d.name.to_lowercase().contains(&f))
                    .collect();
                if shown.is_empty() && !devices.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(muted(&format!("Sin resultados para \"{}\".", self.filter.trim())));
                        if ui.small_button("Limpiar").clicked() {
                            self.filter.clear();
                        }
                    });
                }
                for d in shown {
                    let (name_c, meta_c) = if d.online { (TEXT, MUTED) } else {
                        // Fila apagada: sin conexión = no interactiva.
                        (MUTED, Color32::from_rgb(0x3E, 0x62, 0x4B))
                    };
                    hover_row(ui, |ui| {
                        dot(ui, if d.online { GREEN } else { MUTED });
                        // La derecha primero (acción/estado), luego el nombre
                        // truncado en el resto: jamás se enciman.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if d.online {
                                if primary(ui, "CONECTAR", true).clicked() {
                                    action = Some(Action::Connect(d.clone()));
                                }
                            } else {
                                ui.label(egui::RichText::new("offline").monospace().size(11.0).color(meta_c));
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                // Fila de dos líneas: nombre + badge / clave · SO.
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.horizontal(|ui| {
                                        ui.add(egui::Label::new(egui::RichText::new(&d.name).size(14.0).strong().color(name_c)).truncate());
                                        let role = if d.role == "owner" { "dueño" } else { "compartida" };
                                        egui::Frame::none()
                                            .stroke(egui::Stroke::new(1.0, BORDER))
                                            .rounding(egui::Rounding::same(3.0))
                                            .inner_margin(egui::Margin::symmetric(6.0, 1.0))
                                            .show(ui, |ui| {
                                                ui.label(egui::RichText::new(role).monospace().size(9.5).color(meta_c));
                                            });
                                    });
                                    let mut meta: Vec<String> = Vec::new();
                                    if !d.access_key.is_empty() {
                                        meta.push(format_key(&d.access_key));
                                    }
                                    if let Some(os) = &d.os {
                                        meta.push(os.clone());
                                    }
                                    if !meta.is_empty() {
                                        ui.label(egui::RichText::new(meta.join(" · ")).monospace().size(10.5).color(meta_c));
                                    }
                                });
                            });
                        });
                    });
                    ui.add_space(2.0);
                }
            } else {
                // ---- Iniciar sesión ----
                card_title(ui, "Cuenta", "Inicia sesión para ver tus PCs guardadas y conectarte con un clic");
                ui.add(egui::TextEdit::singleline(&mut self.email).hint_text("correo electrónico").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(12.0, 9.0)));
                ui.add_space(6.0);
                let pw = ui.add(egui::TextEdit::singleline(&mut self.password).password(true).hint_text("contraseña").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(12.0, 9.0)));
                let enter_login = pw.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(10.0);
                if primary_wide(ui, if busy { "AUTENTICANDO…" } else { "INICIAR SESIÓN" }, !busy).clicked() || (enter_login && !busy) {
                    action = Some(Action::Login);
                }
            }
        });
        action
    }
}

/// Lanza una sesión de visor (corre dentro del runtime tokio) y la deja lista para
/// que la GUI abra su ventana.
fn spawn_viewer(ui: &Arc<UiShared>, server: &str, code: String, name: String) {
    let (shared, input_tx, input_rx) = viewer::new_session();
    let http = to_http(server);
    let ws = to_ws(server, "/ws/signal");
    let shared2 = shared.clone();
    tokio::spawn(async move {
        if let Err(e) = viewer::run_viewer_session(&http, &ws, &code, shared2, input_rx).await {
            warn!("visor: {e:#}");
        }
    });
    *ui.pending_viewer.lock() = Some(ActiveViewer {
        shared, input_tx, name, tex: None, size: [0, 0], prev_mods: egui::Modifiers::default(),
        fullscreen: false,
    });
}

/// Botones de transferencia de archivos (reutilizados en ventana y pantalla completa).
fn file_buttons(ui: &mut egui::Ui, av: &ActiveViewer, rt: &tokio::runtime::Handle) {
    let files_dc = av.shared.files_dc.lock().clone();
    let on = files_dc.is_some();
    if ui.add_enabled(on, egui::Button::new("📥 Pedir archivo")).clicked() {
        if let Some(dc) = files_dc.clone() {
            rt.spawn(async move { files::request_file(dc).await; });
        }
    }
    if ui.add_enabled(on, egui::Button::new("📤 Enviar archivo")).clicked() {
        if let Some(dc) = files_dc {
            rt.spawn(async move { files::pick_and_send(dc); });
        }
    }
}

/// Selector de monitor (solo si el host tiene más de uno).
fn monitor_buttons(ui: &mut egui::Ui, av: &ActiveViewer, rt: &tokio::runtime::Handle) {
    let n = av.shared.monitors.load(Ordering::SeqCst);
    if n <= 1 {
        return;
    }
    let active = av.shared.active_monitor.load(Ordering::SeqCst);
    ui.label("Monitor:");
    for i in 0..n {
        if ui.selectable_label(i == active, format!("{}", i + 1)).clicked() {
            av.shared.active_monitor.store(i, Ordering::SeqCst);
            let meta = av.shared.meta_dc.lock().clone();
            if let Some(dc) = meta {
                let msg = format!("{{\"select\":{i}}}");
                rt.spawn(async move { let _ = dc.send_text(msg).await; });
            }
        }
    }
    ui.separator();
}

impl eframe::App for RemotixApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Eventos del rol host.
        while let Ok(ev) = self.host_rx.try_recv() {
            match ev {
                LiteEvent::Code(c) => self.host_code = Some(c),
                LiteEvent::Status(s) => self.host_status = s,
                // Push del servidor: versión nueva publicada → aplicar si inactiva.
                LiteEvent::UpdateAvailable => {
                    let server = self.server.clone();
                    let ui = self.ui.clone();
                    self.rt.spawn(async move { check_update_and_maybe_apply(&server, &ui).await; });
                }
            }
        }

        // Bandeja: si pidió "Abrir", sincroniza el estado de egui (la ventana ya
        // se hizo visible vía Win32 dentro del handler de la bandeja).
        let show_requested = self
            .tray_queue
            .as_ref()
            .map(|q| std::mem::replace(&mut *q.lock(), false))
            .unwrap_or(false);
        if show_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        // Cerrar la ventana (X) = minimizar a la bandeja (no salir de la app).
        if ctx.input(|i| i.viewport().close_requested()) && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        // Recoge una sesión de visor recién creada por la tarea async.
        if let Some(av) = self.ui.pending_viewer.lock().take() {
            self.viewer = Some(av);
        }
        // La auto-actualización consulta este flag para no cortar un visor abierto.
        self.ui.viewer_active.store(self.viewer.is_some(), Ordering::SeqCst);

        let logged_in = self.ui.user.lock().is_some();

        // Refresco periódico de la libreta.
        if logged_in && self.last_refresh.elapsed() > Duration::from_secs(4) {
            self.last_refresh = Instant::now();
            self.refresh_devices();
        }

        // Snapshots para pintar sin mantener locks dentro de la UI.
        let user = self.ui.user.lock().clone();
        let devices = self.ui.devices.lock().clone();
        let error = self.ui.error.lock().clone();
        let busy = self.ui.busy.load(Ordering::SeqCst);
        let update_avail = self.ui.update.lock().clone();

        let mut action: Option<Action> = None;
        let mut clear_error = false;

        // ---- Encabezado: marca tipo prompt + versión + cuenta ----
        egui::TopBottomPanel::top("hdr")
            .frame(egui::Frame::none().fill(PANEL).inner_margin(egui::Margin::symmetric(18.0, 12.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (lr, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                    ui.painter().rect_filled(lr, egui::Rounding::same(3.0), Color32::from_rgb(0x02, 0x06, 0x04));
                    ui.painter().rect_stroke(lr, egui::Rounding::same(3.0), egui::Stroke::new(1.0, ACCENT));
                    ui.painter().text(
                        lr.center(),
                        egui::Align2::CENTER_CENTER,
                        ">_",
                        egui::FontId::monospace(10.0),
                        ACCENT_HI,
                    );
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("REMOTIX").monospace().size(17.0).strong().color(ACCENT_HI));
                    // Cursor de terminal parpadeando junto al wordmark.
                    let cur = if blink_on(ui) { KEYC } else { Color32::TRANSPARENT };
                    ui.label(egui::RichText::new("█").monospace().size(13.0).color(cur));
                    ui.label(egui::RichText::new(format!("v{}", update::CURRENT_VERSION)).monospace().size(10.5).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(u) = &user {
                            // Botón fantasma: salir no debe competir con los CTAs reales.
                            let ghost = egui::Button::new(egui::RichText::new("[cerrar_sesión]").monospace().size(11.0).color(MUTED))
                                .fill(Color32::TRANSPARENT)
                                .stroke(egui::Stroke::NONE);
                            if ui.add(ghost).on_hover_text("Salir de la cuenta en este equipo").clicked() {
                                action = Some(Action::Logout);
                            }
                            ui.add_space(6.0);
                            ui.add(
                                egui::Label::new(egui::RichText::new(format!("usr: {}", u.name)).monospace().size(11.5).color(TEXT)).truncate(),
                            );
                        }
                    });
                });
                // Línea divisoria de consola al borde inferior del encabezado.
                let r = ui.max_rect();
                ui.painter().hline(
                    egui::Rangef::new(r.left() - 18.0, r.right() + 18.0),
                    r.bottom() + 11.0,
                    egui::Stroke::new(1.0, BORDER),
                );
            });

        // ---- Aviso de actualización: franja fija bajo el encabezado ----
        if let Some(info) = &update_avail {
            egui::TopBottomPanel::top("upd")
                .frame(egui::Frame::none().fill(Color32::from_rgb(0x06, 0x1E, 0x10)).inner_margin(egui::Margin::symmetric(18.0, 8.0)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("[UPDATE] v{} disponible", info.version)).monospace().strong().color(KEYC));
                        ui.label(egui::RichText::new("// se instala y reinicia sola").monospace().size(10.5).color(MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if primary(ui, "ACTUALIZAR", true).clicked() {
                                action = Some(Action::Update(info.clone()));
                            }
                        });
                    });
                });
        }

        // ---- Pie: barra de estado tipo terminal ----
        egui::TopBottomPanel::bottom("foot")
            .frame(egui::Frame::none().fill(PANEL).inner_margin(egui::Margin::symmetric(18.0, 7.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
    let s = self.host_status.clone();
                    let online = s.contains("línea") || s.contains("Conectado") || s.contains("ompart");
                    dot(ui, if online { GREEN } else { MUTED });
                    // Sin redundancia: el "En línea · " del estado ya lo dice ONLINE.
                    let detail = s.strip_prefix("En línea · ").unwrap_or(&s);
                    ui.label(
                        egui::RichText::new(format!("{} :: {}", if online { "ONLINE" } else { "OFFLINE" }, detail))
                            .monospace()
                            .size(11.0)
                            .color(if online { ACCENT_HI } else { MUTED }),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(format!("@{}", server_host(&self.server))).monospace().size(11.0).color(MUTED));
                    });
                });
                // Línea divisoria de consola al borde superior del pie.
                let r = ui.max_rect();
                ui.painter().hline(
                    egui::Rangef::new(r.left() - 18.0, r.right() + 18.0),
                    r.top() - 6.0,
                    egui::Stroke::new(1.0, BORDER),
                );
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::same(18.0)))
            .show(ctx, |ui| {
                // Scanlines sutiles de CRT sobre el fondo (las tarjetas van encima).
                {
                    let r = ui.clip_rect();
                    let p = ui.painter();
                    let mut y = r.top();
                    while y < r.bottom() {
                        p.hline(r.x_range(), y, egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(0, 255, 120, 4)));
                        y += 3.0;
                    }
                }
                // Borde inferior visible del panel: referencia para que [MIS PCS]
                // llene el alto disponible al maximizar.
                let panel_bottom = ui.max_rect().bottom();
                egui::ScrollArea::vertical()
                    // Barra siempre visible cuando hay overflow: una fila cortada
                    // sin indicio de scroll se percibe como bug.
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                    .show(ui, |ui| {
                    // ---- Franja de error global (se descarta con ✕) ----
                    if let Some(e) = &error {
                        egui::Frame::none()
                            .fill(Color32::from_rgb(0x24, 0x09, 0x09))
                            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(0x8A, 0x2A, 0x2A)))
                            .rounding(egui::Rounding::same(3.0))
                            .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(format!("[!] {e}")).monospace().color(REDC));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.small_button("✕").clicked() { clear_error = true; }
                                    });
                                });
                            });
                        ui.add_space(12.0);
                    }

                    // ---- Columnas: izquierda FIJA (acciones), derecha ELÁSTICA.
                    // Al maximizar, [CONECTAR] y [ESTE EQUIPO] conservan su
                    // tamaño; [MIS PCS] crece a lo ancho y a lo alto.
                    const LEFT_W: f32 = 340.0;
                    let wide = ui.available_width() >= 640.0;
                    if wide {
                        ui.horizontal_top(|ui| {
                            let row_top = ui.cursor().top();
                            // Columna fija: layout VERTICAL explícito (el padre es
                            // horizontal y sin esto las tarjetas fluirían en fila).
                            let avail_h = (panel_bottom - row_top).max(100.0);
                            let left = ui.allocate_ui_with_layout(
                                egui::vec2(LEFT_W, avail_h),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(LEFT_W);
                                    if let Some(a) = self.ui_connect_card(ui) { action = Some(a); }
                                    ui.add_space(14.0);
                                    if let Some(a) = self.ui_host_card(ui) { action = Some(a); }
                                },
                            );
                            ui.add_space(4.0);
                            // El panel derecho llena el alto visible (crece al
                            // maximizar) y nunca queda más corto que la izquierda.
                            let left_h = left.response.rect.height();
                            let min_h = (panel_bottom - row_top).max(left_h) - 36.0;
                            ui.vertical(|ui| {
                                if let Some(a) = self.ui_right_card(ui, logged_in, &devices, busy, Some(min_h)) {
                                    action = Some(a);
                                }
                            });
                        });
                    } else {
                        if let Some(a) = self.ui_connect_card(ui) { action = Some(a); }
                        ui.add_space(14.0);
                        if let Some(a) = self.ui_host_card(ui) { action = Some(a); }
                        ui.add_space(14.0);
                        if let Some(a) = self.ui_right_card(ui, logged_in, &devices, busy, None) { action = Some(a); }
                    }
                });
            });

        if clear_error {
            *self.ui.error.lock() = None;
        }

        // Ejecuta la acción fuera del closure (evita conflictos de préstamo).
        // Higiene de sesión: al quedar logueado, la contraseña ya no pinta nada
        // en memoria del formulario; y el filtro no debe sobrevivir a la cuenta.
        if logged_in && !self.password.is_empty() {
            self.password.clear();
        }

        match action {
            Some(Action::Login) => self.do_login(),
            Some(Action::Logout) => {
                self.password.clear();
                self.filter.clear();
                self.do_logout();
            }
            Some(Action::Connect(d)) => self.do_connect(d),
            Some(Action::ConnectByKey(k)) => { self.key_input.clear(); self.do_connect_by_key(k); }
            Some(Action::ToggleAutostart(v)) => { let _ = autostart::set_autostart(v); }
            Some(Action::Update(info)) => self.do_update(info),
            None => {}
        }

        // --- Ventana del visor nativo (si hay sesión activa) ---
        let mut close_viewer = false;
        let rt = self.rt.clone();
        if let Some(av) = self.viewer.as_mut() {
            if av.shared.closed.load(Ordering::SeqCst) {
                close_viewer = true;
            } else {
                let title = format!("Remotix — {}", av.name);
                let builder = egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size([1280.0, 760.0]);
                ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("remotix-viewer"), builder, |vctx, _class| {
                    // Maximizar la ventana → pantalla completa (estilo TeamViewer).
                    let maximized = vctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    if maximized && !av.fullscreen {
                        av.fullscreen = true;
                        vctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                    }

                    if av.fullscreen {
                        // Barra flotante arriba: restaurar / minimizar / archivos.
                        egui::Area::new(egui::Id::new("vp-bar"))
                            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 6.0))
                            .show(vctx, |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(av.shared.status.lock().clone()).small().weak());
                                        ui.separator();
                                        if ui.button("🗗 Restaurar").clicked() {
                                            av.fullscreen = false;
                                            vctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                                            vctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                                        }
                                        if ui.button("➖ Minimizar").clicked() {
                                            vctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                                        }
                                        ui.separator();
                                        monitor_buttons(ui, av, &rt);
                                        file_buttons(ui, av, &rt);
                                    });
                                });
                            });
                    } else {
                        egui::TopBottomPanel::top("vstat").show(vctx, |ui| {
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(av.shared.status.lock().clone()).weak());
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button("⛶ Pantalla completa").clicked() {
                                        av.fullscreen = true;
                                        vctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                                    }
                                    file_buttons(ui, av, &rt);
                                    monitor_buttons(ui, av, &rt);
                                });
                            });
                            ui.add_space(2.0);
                        });
                    }

                    egui::CentralPanel::default().show(vctx, |ui| {
                        render_viewer(ui, vctx, av);
                    });
                    if vctx.input(|i| i.viewport().close_requested()) {
                        close_viewer = true;
                    }
                    vctx.request_repaint();
                });
            }
        }
        if close_viewer {
            self.viewer = None;
        }

        ctx.request_repaint_after(Duration::from_millis(if self.viewer.is_some() { 16 } else { 200 }));
    }
}

/// Pinta el frame remoto como textura y reenvía el input local por el canal.
fn render_viewer(ui: &mut egui::Ui, vctx: &egui::Context, av: &mut ActiveViewer) {
    // Sube el último frame decodificado a la textura (reusándola si el tamaño no cambió).
    if let Some(frame) = av.shared.frame.lock().take() {
        let img = egui::ColorImage::from_rgba_unmultiplied([frame.w, frame.h], &frame.rgba);
        match &mut av.tex {
            Some(t) if av.size == [frame.w, frame.h] => t.set(img, egui::TextureOptions::LINEAR),
            _ => {
                av.tex = Some(vctx.load_texture("remote-screen", img, egui::TextureOptions::LINEAR));
                av.size = [frame.w, frame.h];
            }
        }
    }

    let Some(tex) = av.tex.as_ref() else {
        ui.centered_and_justified(|ui| { ui.label(av.shared.status.lock().clone()); });
        return;
    };
    let (tw, th) = (av.size[0] as f32, av.size[1] as f32);
    if tw <= 0.0 || th <= 0.0 { return; }
    let avail = ui.available_size();
    let scale = (avail.x / tw).min(avail.y / th).max(0.01);
    let disp = egui::vec2(tw * scale, th * scale);
    let sized = egui::load::SizedTexture::new(tex.id(), egui::vec2(tw, th));
    let resp = ui.add(egui::Image::new(sized).fit_to_exact_size(disp).sense(egui::Sense::click_and_drag()));
    let rect = resp.rect;
    let norm = |p: egui::Pos2| -> Option<(f64, f64)> {
        if rect.width() <= 0.0 || rect.height() <= 0.0 { return None; }
        Some((
            ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64,
            ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0) as f64,
        ))
    };

    let (pointer, scroll, events, mods) = ui.input(|i| (i.pointer.clone(), i.raw_scroll_delta, i.events.clone(), i.modifiers));

    // Movimiento del ratón.
    if let Some(p) = resp.hover_pos().or_else(|| resp.interact_pointer_pos()) {
        if let Some((x, y)) = norm(p) { let _ = av.input_tx.send(InputEvent::Move { x, y }); }
    }
    // Botones.
    if let Some(p) = resp.interact_pointer_pos().or_else(|| resp.hover_pos()) {
        if let Some((x, y)) = norm(p) {
            for (btn, idx) in [
                (egui::PointerButton::Primary, 0i32),
                (egui::PointerButton::Middle, 1),
                (egui::PointerButton::Secondary, 2),
            ] {
                if pointer.button_pressed(btn) { let _ = av.input_tx.send(InputEvent::Down { x, y, button: idx }); }
                if pointer.button_released(btn) { let _ = av.input_tx.send(InputEvent::Up { x, y, button: idx }); }
            }
        }
    }
    // Rueda.
    if scroll.x != 0.0 || scroll.y != 0.0 {
        if let Some((x, y)) = resp.hover_pos().and_then(norm) {
            let _ = av.input_tx.send(InputEvent::Wheel { x, y, dx: scroll.x as f64, dy: scroll.y as f64 });
        }
    }
    // Modificadores (Shift/Ctrl/Alt) como transiciones, para soportar atajos.
    emit_mod_transitions(av, mods);
    // Teclado.
    for ev in events {
        match ev {
            egui::Event::Text(t) => {
                for c in t.chars() {
                    let _ = av.input_tx.send(InputEvent::Key { down: true, code: String::new(), key: c.to_string() });
                    let _ = av.input_tx.send(InputEvent::Key { down: false, code: String::new(), key: c.to_string() });
                }
            }
            egui::Event::Key { key, pressed, .. } => {
                if let Some(code) = named_code(key) {
                    let _ = av.input_tx.send(InputEvent::Key { down: pressed, code: code.to_string(), key: String::new() });
                } else if mods.ctrl || mods.alt {
                    if let Some(c) = key_char(key) {
                        let _ = av.input_tx.send(InputEvent::Key { down: pressed, code: String::new(), key: c.to_string() });
                    }
                }
            }
            _ => {}
        }
    }
}

fn emit_mod_transitions(av: &mut ActiveViewer, mods: egui::Modifiers) {
    let tx = &av.input_tx;
    let send = |code: &str, down: bool| {
        let _ = tx.send(InputEvent::Key { down, code: code.to_string(), key: String::new() });
    };
    if mods.shift != av.prev_mods.shift { send("ShiftLeft", mods.shift); }
    if mods.ctrl != av.prev_mods.ctrl { send("ControlLeft", mods.ctrl); }
    if mods.alt != av.prev_mods.alt { send("AltLeft", mods.alt); }
    av.prev_mods = mods;
}

fn named_code(key: egui::Key) -> Option<&'static str> {
    use egui::Key::*;
    Some(match key {
        Enter => "Enter",
        Tab => "Tab",
        Space => "Space",
        Backspace => "Backspace",
        Escape => "Escape",
        Delete => "Delete",
        Insert => "Insert",
        Home => "Home",
        End => "End",
        PageUp => "PageUp",
        PageDown => "PageDown",
        ArrowUp => "ArrowUp",
        ArrowDown => "ArrowDown",
        ArrowLeft => "ArrowLeft",
        ArrowRight => "ArrowRight",
        F1 => "F1", F2 => "F2", F3 => "F3", F4 => "F4", F5 => "F5", F6 => "F6",
        F7 => "F7", F8 => "F8", F9 => "F9", F10 => "F10", F11 => "F11", F12 => "F12",
        _ => return None,
    })
}

fn key_char(key: egui::Key) -> Option<char> {
    use egui::Key::*;
    Some(match key {
        A => 'a', B => 'b', C => 'c', D => 'd', E => 'e', F => 'f', G => 'g', H => 'h', I => 'i',
        J => 'j', K => 'k', L => 'l', M => 'm', N => 'n', O => 'o', P => 'p', Q => 'q', R => 'r',
        S => 's', T => 't', U => 'u', V => 'v', W => 'w', X => 'x', Y => 'y', Z => 'z',
        Num0 => '0', Num1 => '1', Num2 => '2', Num3 => '3', Num4 => '4',
        Num5 => '5', Num6 => '6', Num7 => '7', Num8 => '8', Num9 => '9',
        _ => return None,
    })
}
