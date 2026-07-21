//! Remotix Lite — acceso remoto desatendido estilo TeamViewer. Al abrirse muestra
//! una CLAVE FIJA permanente (se guarda y no cambia), se arranca con Windows, y
//! el técnico se conecta por esa clave —por internet— para ver, controlar y
//! transferir archivos. Sin instalación, sin cuentas, sin proyecto.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::Result;
use eframe::egui;

use remotix_agent::autostart;
use remotix_agent::config::LiteConfig;
use remotix_agent::device::run_lite_unattended;
use remotix_agent::session::LiteEvent;

/// Subcomandos reservados: nunca deben confundirse con una URL de servidor.
const SUBCOMMANDS: &[&str] = &["console", "helper", "service", "install", "uninstall"];

fn server() -> String {
    let baked = option_env!("REMOTIX_DEFAULT_SERVER").unwrap_or("ws://localhost:8080");
    // Si ya está registrado, respeta el servidor guardado.
    if let Some(c) = LiteConfig::load() {
        return c.server;
    }
    std::env::var("REMOTIX_SERVER")
        .ok()
        .or_else(|| {
            std::env::args()
                .skip(1)
                .find(|a| !SUBCOMMANDS.contains(&a.as_str()))
        })
        .unwrap_or_else(|| baked.to_string())
}

fn main() -> Result<()> {
    remotix_agent::logging::init();

    // Subcomandos del modo desatendido (servicio de Windows + ayudante).
    #[cfg(windows)]
    {
        match std::env::args().nth(1).as_deref() {
            Some("service") => return remotix_agent::winsvc::run(), // lo arranca el SCM
            Some("install") => return remotix_agent::winsvc::install(),
            Some("uninstall") => return remotix_agent::winsvc::uninstall(),
            Some("helper") => {
                let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Equipo".into());
                return remotix_agent::tray::run_helper(server(), name);
            }
            _ => {}
        }
    }

    let console = std::env::args().any(|a| a == "console");
    let server = server();
    let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Equipo".into());

    if console {
        let rt = tokio::runtime::Runtime::new()?;
        let (tx, rx) = std::sync::mpsc::channel::<LiteEvent>();
        std::thread::spawn(move || {
            for ev in rx {
                match ev {
                    LiteEvent::Code(c) => println!("CODE {c}"),
                    LiteEvent::Status(s) => println!("STATUS {s}"),
                    LiteEvent::UpdateAvailable => println!("UPDATE available"),
                    LiteEvent::ConfirmMode(v) => println!("CONFIRM {v}"),
                }
            }
        });
        rt.block_on(run_lite_unattended(server, name, tx));
        return Ok(());
    }

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let (tx, rx) = std::sync::mpsc::channel::<LiteEvent>();
    rt.spawn(run_lite_unattended(server, name, tx.clone()));
    let handle = rt.handle().clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([400.0, 390.0])
            .with_resizable(false)
            .with_title("Remotix"),
        ..Default::default()
    };
    eframe::run_native("Remotix", options, Box::new(move |_cc| Ok(Box::new(LiteApp::new(rx, tx, handle)))))
        .map_err(|e| anyhow::anyhow!("error de la ventana: {e}"))?;
    Ok(())
}

fn format_key(k: &str) -> String {
    k.chars().collect::<Vec<_>>().chunks(3).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

struct LiteApp {
    rx: Receiver<LiteEvent>,
    /// Para auto-enviarse eventos (p. ej. revertir el checkbox si el POST falla).
    tx: std::sync::mpsc::Sender<LiteEvent>,
    rt: tokio::runtime::Handle,
    code: Option<String>,
    status: String,
    autostart: bool,
    require_confirm: bool,
}

impl LiteApp {
    fn new(rx: Receiver<LiteEvent>, tx: std::sync::mpsc::Sender<LiteEvent>, rt: tokio::runtime::Handle) -> Self {
        Self {
            rx,
            tx,
            rt,
            code: None,
            status: "Iniciando…".into(),
            autostart: autostart::is_autostart(),
            require_confirm: false,
        }
    }
}

impl eframe::App for LiteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                LiteEvent::Code(c) => self.code = Some(c),
                LiteEvent::Status(s) => self.status = s,
                LiteEvent::UpdateAvailable => {} // el servicio (canal host) actualiza
                LiteEvent::ConfirmMode(v) => self.require_confirm = v,
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.heading("Remotix");
                ui.label(egui::RichText::new("Acceso remoto").weak());
                ui.add_space(20.0);
                ui.label(egui::RichText::new("TU CLAVE DE ACCESO").small().weak());
                ui.add_space(6.0);
                match &self.code {
                    Some(c) => {
                        ui.label(egui::RichText::new(format_key(c)).size(38.0).strong().monospace()
                            .color(egui::Color32::from_rgb(255, 210, 102)));
                    }
                    None => { ui.label(egui::RichText::new("·········").size(38.0).weak().monospace()); }
                }
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Esta clave es fija: siempre la misma.").small().weak());
                ui.add_space(16.0);
                ui.label(egui::RichText::new(&self.status).color(egui::Color32::from_rgb(150, 163, 171)));
                ui.add_space(14.0);
                if ui.checkbox(&mut self.autostart, "Iniciar con Windows").changed() {
                    let _ = autostart::set_autostart(self.autostart);
                }
                // Toggle opcional por equipo: por defecto APAGADO (acceso
                // desatendido puro). El valor real vive en el servidor; si el
                // POST falla se revierte el checkbox al valor anterior.
                if ui.checkbox(&mut self.require_confirm, "Pedir permiso antes de conectar").changed() {
                    let value = self.require_confirm;
                    let tx = self.tx.clone();
                    self.rt.spawn(async move {
                        if remotix_agent::device::set_confirm_mode(value).await.is_err() {
                            let _ = tx.send(LiteEvent::ConfirmMode(!value));
                        }
                    });
                }
            });
        });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}
