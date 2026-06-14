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

fn server() -> String {
    let baked = option_env!("REMOTIX_DEFAULT_SERVER").unwrap_or("ws://localhost:8080");
    // Si ya está registrado, respeta el servidor guardado.
    if let Some(c) = LiteConfig::load() {
        return c.server;
    }
    std::env::var("REMOTIX_SERVER")
        .ok()
        .or_else(|| std::env::args().skip(1).find(|a| *a != "console"))
        .unwrap_or_else(|| baked.to_string())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,remotix_agent=info")),
        )
        .init();

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
                }
            }
        });
        rt.block_on(run_lite_unattended(server, name, tx));
        return Ok(());
    }

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let (tx, rx) = std::sync::mpsc::channel::<LiteEvent>();
    rt.spawn(run_lite_unattended(server, name, tx));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([400.0, 360.0])
            .with_resizable(false)
            .with_title("Remotix"),
        ..Default::default()
    };
    eframe::run_native("Remotix", options, Box::new(move |_cc| Ok(Box::new(LiteApp::new(rx)))))
        .map_err(|e| anyhow::anyhow!("error de la ventana: {e}"))?;
    Ok(())
}

fn format_key(k: &str) -> String {
    k.chars().collect::<Vec<_>>().chunks(3).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

struct LiteApp {
    rx: Receiver<LiteEvent>,
    code: Option<String>,
    status: String,
    autostart: bool,
}

impl LiteApp {
    fn new(rx: Receiver<LiteEvent>) -> Self {
        Self { rx, code: None, status: "Iniciando…".into(), autostart: autostart::is_autostart() }
    }
}

impl eframe::App for LiteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                LiteEvent::Code(c) => self.code = Some(c),
                LiteEvent::Status(s) => self.status = s,
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
            });
        });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}
