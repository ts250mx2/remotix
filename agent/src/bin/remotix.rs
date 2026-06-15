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
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

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

/// Icono en la bandeja del sistema + ids de su menú.
struct TrayState {
    _tray: TrayIcon,
    show_id: MenuId,
    quit_id: MenuId,
}

fn make_tray_icon() -> Option<tray_icon::Icon> {
    let (w, h) = (32u32, 32u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        rgba.extend_from_slice(&[32, 118, 209, 255]); // azul Remotix
    }
    tray_icon::Icon::from_rgba(rgba, w, h).ok()
}

fn create_tray() -> Option<TrayState> {
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
    Some(TrayState { _tray: tray, show_id, quit_id })
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

    let app = RemotixApp {
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
        tray_tried: false,
        want_quit: false,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([580.0, 640.0])
            .with_min_inner_size([460.0, 420.0])
            .with_title("Remotix"),
        ..Default::default()
    };
    eframe::run_native("Remotix", options, Box::new(move |_cc| Ok(Box::new(app))))
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
    tray_tried: bool,
    want_quit: bool,
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
    });
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

        // Bandeja del sistema: crear (una vez) y atender sus eventos.
        if !self.tray_tried {
            self.tray = create_tray();
            self.tray_tried = true;
        }
        if let Some(tray) = &self.tray {
            let mut show = false;
            while let Ok(ev) = MenuEvent::receiver().try_recv() {
                if ev.id == tray.show_id {
                    show = true;
                } else if ev.id == tray.quit_id {
                    self.want_quit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                if matches!(ev, TrayIconEvent::DoubleClick { .. }) {
                    show = true;
                }
            }
            if show {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
        // Cerrar la ventana principal = minimizar a la bandeja (no salir), salvo "Salir".
        if ctx.input(|i| i.viewport().close_requested()) && !self.want_quit && self.tray.is_some() {
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

        egui::TopBottomPanel::top("hdr").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("Remotix");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(u) = &user {
                        if ui.button("Salir").clicked() { action = Some(Action::Logout); }
                        ui.label(egui::RichText::new(&u.name).strong());
                    }
                });
            });
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            // --- Rol host: clave + estado (siempre visible) ---
            ui.group(|ui| {
                ui.label(egui::RichText::new("ESTE EQUIPO (acepta conexiones)").small().weak());
                ui.horizontal(|ui| {
                    ui.label("Clave:");
                    match &self.host_code {
                        Some(c) => { ui.label(egui::RichText::new(format_key(c)).strong().monospace().size(20.0).color(egui::Color32::from_rgb(255, 210, 102))); }
                        None => { ui.label(egui::RichText::new("·········").weak().monospace().size(20.0)); }
                    }
                });
                ui.label(egui::RichText::new(&self.host_status).weak());
                if ui.checkbox(&mut self.autostart, "Iniciar con Windows").changed() {
                    action = Some(Action::ToggleAutostart(self.autostart));
                }
            });

            ui.add_space(12.0);

            // Conectar por clave — SIEMPRE disponible (con o sin login). Equipos sin
            // dueño se conectan solo con la clave; los que tienen dueño exigen login.
            ui.group(|ui| {
                ui.label(egui::RichText::new("Conectar a otra PC por su clave").strong());
                ui.horizontal(|ui| {
                    let resp = ui.add(egui::TextEdit::singleline(&mut self.key_input)
                        .hint_text("XXX-XXX-XXX").desired_width(150.0));
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button("Conectar").clicked() || enter) && !self.key_input.trim().is_empty() {
                        action = Some(Action::ConnectByKey(self.key_input.trim().to_string()));
                    }
                });
            });
            ui.add_space(12.0);

            if let Some(_u) = &user {
                // --- Libreta de PCs (con sesión) ---
                ui.heading("Mis PCs");
                ui.label(egui::RichText::new("Equipos a los que tienes acceso.").small().weak());
                ui.add_space(6.0);
                if let Some(e) = &error { ui.colored_label(egui::Color32::from_rgb(229, 115, 115), e); }
                if devices.is_empty() {
                    ui.label(egui::RichText::new("Aún no tienes PCs. Inicia sesión en otro equipo con Remotix para que aparezca aquí.").weak());
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for d in &devices {
                        ui.horizontal(|ui| {
                            let dot = if d.online { "🟢" } else { "⚪" };
                            ui.label(dot);
                            ui.label(egui::RichText::new(&d.name).strong());
                            ui.label(egui::RichText::new(if d.role == "owner" { "dueño" } else { "compartido" }).small().weak());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add_enabled(d.online, egui::Button::new("Conectar")).clicked() {
                                    action = Some(Action::Connect(d.clone()));
                                }
                            });
                        });
                        ui.separator();
                    }
                });
            } else {
                // --- Sin login: formulario ---
                ui.heading("Iniciar sesión");
                ui.label(egui::RichText::new("Entra con tu cuenta para conectarte a tus PCs. (Sin login, este equipo solo acepta conexiones.)").small().weak());
                ui.add_space(6.0);
                ui.horizontal(|ui| { ui.label("Email:"); ui.text_edit_singleline(&mut self.email); });
                ui.horizontal(|ui| {
                    ui.label("Clave:");
                    ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
                });
                if let Some(e) = &error { ui.colored_label(egui::Color32::from_rgb(229, 115, 115), e); }
                ui.add_space(4.0);
                if ui.add_enabled(!busy, egui::Button::new(if busy { "Entrando…" } else { "Entrar" })).clicked() {
                    action = Some(Action::Login);
                }
            }
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
                    egui::TopBottomPanel::top("vstat").show(vctx, |ui| {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(av.shared.status.lock().clone()).weak());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                            });
                        });
                        ui.add_space(2.0);
                    });
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
