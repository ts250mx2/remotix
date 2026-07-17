//! Remotix Agent (completo) — cliente de chat del equipo (ventana nativa) +
//! control remoto. Se enrola con el UUID del proyecto, se conecta al chat y,
//! cuando un técnico lo solicita, comparte la pantalla y permite control.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::Result;

use remotix_agent::capture;
use remotix_agent::chat::{self, UiAction, UiEvent};
use remotix_agent::ui::AgentApp;

fn main() -> Result<()> {
    remotix_agent::logging::init();

    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "selftest") {
        return capture::self_test();
    }
    if let Some(i) = args.iter().position(|a| a == "chattest") {
        let server = args.get(i + 1).cloned().unwrap_or_default();
        let uuid = args.get(i + 2).cloned().unwrap_or_default();
        let rt = tokio::runtime::Runtime::new()?;
        return rt.block_on(chat::self_test_chat(&server, &uuid));
    }

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let (ui_tx, ui_rx) = std::sync::mpsc::channel::<UiEvent>();
    let (action_tx, action_rx) = tokio::sync::mpsc::unbounded_channel::<UiAction>();
    rt.spawn(async move { chat::run_agent(ui_tx, action_rx).await });

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([440.0, 580.0])
            .with_min_inner_size([360.0, 420.0])
            .with_title("Remotix"),
        ..Default::default()
    };
    eframe::run_native(
        "Remotix",
        options,
        Box::new(move |_cc| Ok(Box::new(AgentApp::new(action_tx, ui_rx)))),
    )
    .map_err(|e| anyhow::anyhow!("error de la ventana: {e}"))?;

    Ok(())
}
