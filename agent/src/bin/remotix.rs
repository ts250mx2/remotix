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
use remotix_agent::viewer::{self, ViewerShared};

fn resolve_server() -> String {
    let baked = option_env!("REMOTIX_DEFAULT_SERVER").unwrap_or("ws://localhost:8080");
    if let Some(c) = LiteConfig::load() {
        if !c.server.is_empty() {
            return c.server;
        }
    }
    std::env::var("REMOTIX_SERVER").ok()
        .or_else(|| std::env::args().skip(1).find(|a| *a != "console"))
        .unwrap_or_else(|| baked.to_string())
}

fn format_key(k: &str) -> String {
    k.chars().collect::<Vec<_>>().chunks(3).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

// ---- Tema premium (oscuro de marca) ----
use egui::Color32;
const BG: Color32 = Color32::from_rgb(0x0D, 0x13, 0x1E);
const PANEL: Color32 = Color32::from_rgb(0x11, 0x18, 0x26);
const CARD: Color32 = Color32::from_rgb(0x17, 0x20, 0x31);
const CARD_HI: Color32 = Color32::from_rgb(0x20, 0x2C, 0x42);
const BORDER: Color32 = Color32::from_rgb(0x26, 0x33, 0x4A);
const TEXT: Color32 = Color32::from_rgb(0xE7, 0xED, 0xF6);
const MUTED: Color32 = Color32::from_rgb(0x8A, 0x97, 0xAD);
const ACCENT: Color32 = Color32::from_rgb(0x3B, 0x82, 0xF6);
const ACCENT_HI: Color32 = Color32::from_rgb(0x55, 0x96, 0xFF);
const GREEN: Color32 = Color32::from_rgb(0x34, 0xD3, 0x99);
const KEYC: Color32 = Color32::from_rgb(0x6F, 0xD8, 0xFF);
const REDC: Color32 = Color32::from_rgb(0xF8, 0x71, 0x71);

fn setup_style(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, Margin, Rounding, Stroke, TextStyle};
    let mut style = (*ctx.style()).clone();
    let mut v = egui::Visuals::dark();
    let rnd = Rounding::same(8.0);
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(0x0B, 0x10, 0x1A); // fondo de inputs
    v.faint_bg_color = CARD;
    v.hyperlink_color = ACCENT_HI;
    v.selection.bg_fill = Color32::from_rgb(0x1E, 0x3A, 0x60);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    let set = |w: &mut egui::style::WidgetVisuals, fill: Color32, stroke: Color32, fg: Color32| {
        w.bg_fill = fill; w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, stroke);
        w.fg_stroke = Stroke::new(1.0, fg);
        w.rounding = rnd;
    };
    set(&mut v.widgets.noninteractive, PANEL, BORDER, TEXT);
    set(&mut v.widgets.inactive, CARD_HI, BORDER, TEXT);
    set(&mut v.widgets.hovered, Color32::from_rgb(0x29, 0x37, 0x52), ACCENT, Color32::WHITE);
    set(&mut v.widgets.active, ACCENT, ACCENT, Color32::WHITE);
    set(&mut v.widgets.open, CARD_HI, BORDER, TEXT);
    style.visuals = v;
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.window_margin = Margin::same(0.0);
    style.text_styles.insert(TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Button, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Small, FontId::new(12.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Monospace, FontId::new(14.0, FontFamily::Monospace));
    ctx.set_style(style);
}

/// Tarjeta redondeada con borde sutil.
fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .rounding(egui::Rounding::same(12.0))
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// Botón primario de acento.
fn primary(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let btn = egui::Button::new(egui::RichText::new(text).color(Color32::WHITE).strong())
        .fill(if enabled { ACCENT } else { CARD_HI })
        .rounding(egui::Rounding::same(8.0))
        .min_size(egui::vec2(0.0, 34.0));
    ui.add_enabled(enabled, btn)
}

fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(11.0, 11.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

fn muted(s: &str) -> egui::RichText {
    egui::RichText::new(s).color(MUTED)
}

/// Icono de la ventana: cuadrado con degradado azul→cian.
fn app_icon() -> egui::IconData {
    let (w, h) = (64usize, 64usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let t = (x + y) as f32 / (w + h) as f32;
            rgba[i] = (0x3B as f32 * (1.0 - t) + 0x22 as f32 * t) as u8;
            rgba[i + 1] = (0x82 as f32 * (1.0 - t) + 0xC7 as f32 * t) as u8;
            rgba[i + 2] = (0xF6 as f32 * (1.0 - t) + 0xE8 as f32 * t) as u8;
            rgba[i + 3] = 255;
        }
    }
    egui::IconData { rgba, width: w as u32, height: h as u32 }
}

/// Icono en la bandeja del sistema. Se mantiene vivo mientras exista; al soltarlo
/// (Drop) el icono desaparece de la bandeja.
struct TrayState {
    _tray: TrayIcon,
}

/// Orden que los handlers de la bandeja envían al bucle de la GUI.
enum TrayCmd {
    Show,
    Quit,
}

/// Cola compartida entre los handlers de la bandeja (hilos del SO) y la GUI.
type TrayQueue = Arc<Mutex<Vec<TrayCmd>>>;

fn make_tray_icon() -> Option<tray_icon::Icon> {
    let (w, h) = (32u32, 32u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        rgba.extend_from_slice(&[32, 118, 209, 255]); // azul Remotix
    }
    tray_icon::Icon::from_rgba(rgba, w, h).ok()
}

/// Crea el icono de la bandeja y registra handlers que traducen sus eventos a
/// comandos y DESPIERTAN el bucle de eframe (`request_repaint`). Esto es la clave
/// del fix: sin el wake-up explícito, con la ventana oculta el bucle duerme y los
/// clics/menú de la bandeja se pierden (por eso "no hacía nada" al reabrir).
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

    let queue: TrayQueue = Arc::new(Mutex::new(Vec::new()));

    // Menú de la bandeja (Abrir / Salir). El handler debe ser Send+Sync, por eso
    // usamos una cola Arc<Mutex<…>> en vez de un canal (Sender no es Sync).
    let ctx_m = ctx.clone();
    let q_m = queue.clone();
    MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
        if ev.id == show_id {
            q_m.lock().push(TrayCmd::Show);
        } else if ev.id == quit_id {
            q_m.lock().push(TrayCmd::Quit);
        }
        ctx_m.request_repaint();
    }));

    // Clic izquierdo (simple o doble) en el icono → abrir. El clic derecho abre el
    // menú (lo gestiona tray-icon) y no debe reabrir la ventana.
    let ctx_t = ctx.clone();
    let q_t = queue.clone();
    TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| {
        let open = matches!(
            ev,
            TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }
        );
        if open {
            q_t.lock().push(TrayCmd::Show);
        }
        ctx_t.request_repaint();
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

    // Revalida el token persistido en segundo plano.
    if token.is_some() {
        let account = account.clone();
        let ui = ui.clone();
        handle.spawn(async move {
            let acc = account.lock().await;
            if let Ok(user) = acc.me().await {
                let devs = acc.devices().await.unwrap_or_default();
                drop(acc);
                *ui.user.lock() = Some(user);
                *ui.devices.lock() = devs;
            } else {
                LiteConfig::set_session(None, None);
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
        viewer: None,
        last_refresh: Instant::now(),
        server,
        tray: None,
        tray_queue: None,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 660.0])
            .with_min_inner_size([460.0, 460.0])
            .with_icon(std::sync::Arc::new(app_icon()))
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
            if let Ok(devs) = acc.devices().await {
                drop(acc);
                *ui.devices.lock() = devs;
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
            }
        }

        // Bandeja del sistema: procesa los comandos que enviaron sus handlers.
        // (La bandeja se creó al arrancar y sus handlers despiertan el bucle con
        // request_repaint, así reabrir/salir funcionan aun con la ventana oculta.)
        let tray_cmds: Vec<TrayCmd> = self
            .tray_queue
            .as_ref()
            .map(|q| q.lock().drain(..).collect())
            .unwrap_or_default();
        for cmd in tray_cmds {
            match cmd {
                TrayCmd::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                TrayCmd::Quit => {
                    self.tray = None; // quita el icono de la bandeja
                    std::process::exit(0); // "Salir" cierra de verdad
                }
            }
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

        let mut action: Option<Action> = None;

        egui::TopBottomPanel::top("hdr")
            .frame(egui::Frame::none().fill(PANEL).inner_margin(egui::Margin::symmetric(18.0, 13.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (lr, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                    ui.painter().rect_filled(lr, egui::Rounding::same(6.0), ACCENT);
                    ui.painter().circle_filled(lr.center(), 3.2, Color32::WHITE);
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Remotix").size(19.0).strong().color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(u) = &user {
                            if ui.button("Salir").clicked() { action = Some(Action::Logout); }
                            ui.add_space(4.0);
                            ui.label(muted(&u.name));
                            dot(ui, GREEN);
                        }
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::same(18.0)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // --- Este equipo (host) ---
                    card(ui, |ui| {
                        ui.label(muted("ESTE EQUIPO · ACEPTA CONEXIONES"));
                        ui.add_space(10.0);
                        match &self.host_code {
                            Some(c) => { ui.label(egui::RichText::new(format_key(c)).monospace().size(30.0).strong().color(KEYC)); }
                            None => { ui.label(egui::RichText::new("· · · · · · · · ·").monospace().size(28.0).color(MUTED)); }
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let s = self.host_status.clone();
                            let online = s.contains("línea") || s.contains("Conectado") || s.contains("ompart");
                            dot(ui, if online { GREEN } else { MUTED });
                            ui.label(muted(&s));
                        });
                        ui.add_space(12.0);
                        if ui.checkbox(&mut self.autostart, "Iniciar con Windows").changed() {
                            action = Some(Action::ToggleAutostart(self.autostart));
                        }
                    });

                    ui.add_space(14.0);

                    // --- Conectar por clave ---
                    card(ui, |ui| {
                        ui.label(egui::RichText::new("Conectar a otra PC").size(15.0).strong().color(TEXT));
                        ui.label(muted("Escribe la clave del equipo remoto"));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.key_input)
                                    .hint_text("XXX-XXX-XXX")
                                    .desired_width(190.0)
                                    .margin(egui::Margin::symmetric(10.0, 8.0)),
                            );
                            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            let go = primary(ui, "Conectar", true).clicked() || enter;
                            if go && !self.key_input.trim().is_empty() {
                                action = Some(Action::ConnectByKey(self.key_input.trim().to_string()));
                            }
                        });
                    });

                    ui.add_space(14.0);

                    if user.is_some() {
                        // --- Libreta de PCs ---
                        card(ui, |ui| {
                            ui.label(egui::RichText::new("Mis PCs").size(15.0).strong().color(TEXT));
                            ui.label(muted("Equipos a los que tienes acceso"));
                            if let Some(e) = &error { ui.add_space(4.0); ui.colored_label(REDC, e); }
                            ui.add_space(8.0);
                            if devices.is_empty() {
                                ui.label(muted("Aún no tienes PCs. Inicia sesión en otro equipo con Remotix para que aparezca aquí."));
                            }
                            for (idx, d) in devices.iter().enumerate() {
                                if idx > 0 { ui.add_space(3.0); ui.separator(); ui.add_space(3.0); }
                                ui.horizontal(|ui| {
                                    dot(ui, if d.online { GREEN } else { MUTED });
                                    ui.label(egui::RichText::new(&d.name).strong().color(TEXT));
                                    ui.label(muted(if d.role == "owner" { "dueño" } else { "compartido" }));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if primary(ui, "Conectar", d.online).clicked() {
                                            action = Some(Action::Connect(d.clone()));
                                        }
                                    });
                                });
                            }
                        });
                    } else {
                        // --- Iniciar sesión ---
                        card(ui, |ui| {
                            ui.label(egui::RichText::new("Iniciar sesión").size(15.0).strong().color(TEXT));
                            ui.label(muted("Para ver tu libreta de PCs (opcional)"));
                            ui.add_space(10.0);
                            ui.add(egui::TextEdit::singleline(&mut self.email).hint_text("Email").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10.0, 8.0)));
                            ui.add_space(6.0);
                            ui.add(egui::TextEdit::singleline(&mut self.password).password(true).hint_text("Contraseña").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10.0, 8.0)));
                            if let Some(e) = &error { ui.add_space(4.0); ui.colored_label(REDC, e); }
                            ui.add_space(10.0);
                            if primary(ui, if busy { "Entrando…" } else { "Entrar" }, !busy).clicked() {
                                action = Some(Action::Login);
                            }
                        });
                    }
                });
            });

        // Ejecuta la acción fuera del closure (evita conflictos de préstamo).
        match action {
            Some(Action::Login) => self.do_login(),
            Some(Action::Logout) => self.do_logout(),
            Some(Action::Connect(d)) => self.do_connect(d),
            Some(Action::ConnectByKey(k)) => { self.key_input.clear(); self.do_connect_by_key(k); }
            Some(Action::ToggleAutostart(v)) => { let _ = autostart::set_autostart(v); }
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
