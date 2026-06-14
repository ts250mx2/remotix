//! Ventana de chat del agente (egui). Pantalla de enrolamiento por UUID y
//! pantalla de chat con canales, mensajes, "Pedir soporte" y consentimiento de
//! control remoto.

use std::collections::HashMap;

use eframe::egui;
use tokio::sync::mpsc::UnboundedSender;

use crate::chat::{ChannelInfo, MsgInfo, UiAction, UiEvent};
use crate::config::AgentConfig;

enum Screen { Enroll, Chat }

pub struct AgentApp {
    action_tx: UnboundedSender<UiAction>,
    ui_rx: std::sync::mpsc::Receiver<UiEvent>,
    screen: Screen,

    // enrolamiento
    server: String,
    uuid: String,
    name: String,
    joining: bool,
    enroll_error: Option<String>,

    // chat
    equipo_id: String,
    channels: Vec<ChannelInfo>,
    selected: Option<String>,
    msgs: HashMap<String, Vec<MsgInfo>>,
    input: String,
    status: String,
    invite: Option<(String, String)>, // (code, from)
    remote_status: Option<String>,

    // login (casar usuario↔PC)
    bound: Option<String>,
    login_email: String,
    login_pw: String,
    login_error: Option<String>,
}

impl AgentApp {
    pub fn new(action_tx: UnboundedSender<UiAction>, ui_rx: std::sync::mpsc::Receiver<UiEvent>) -> Self {
        let cfg = AgentConfig::load();
        let default_server = option_env!("REMOTIX_DEFAULT_SERVER").unwrap_or("http://localhost:8080").to_string();
        let default_name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Equipo".into());
        AgentApp {
            action_tx,
            ui_rx,
            screen: if cfg.is_some() { Screen::Chat } else { Screen::Enroll },
            server: cfg.as_ref().map(|c| c.server.clone()).unwrap_or(default_server),
            uuid: String::new(),
            name: cfg.as_ref().map(|c| c.name.clone()).unwrap_or(default_name),
            joining: false,
            enroll_error: None,
            equipo_id: cfg.as_ref().map(|c| c.equipo_id.clone()).unwrap_or_default(),
            channels: Vec::new(),
            selected: None,
            msgs: HashMap::new(),
            input: String::new(),
            status: String::new(),
            invite: None,
            remote_status: None,
            bound: None,
            login_email: String::new(),
            login_pw: String::new(),
            login_error: None,
        }
    }

    fn drain(&mut self) {
        while let Ok(ev) = self.ui_rx.try_recv() {
            match ev {
                UiEvent::Status(s) => self.status = s,
                UiEvent::Channels(chs) => {
                    self.channels = chs;
                    if self.selected.is_none() {
                        if let Some(first) = self.channels.first() {
                            self.selected = Some(first.id.clone());
                            let _ = self.action_tx.send(UiAction::LoadHistory { channel_id: first.id.clone() });
                        }
                    }
                }
                UiEvent::History(cid, list) => { self.msgs.insert(cid, list); }
                UiEvent::Message(m) => { self.msgs.entry(m.channel_id.clone()).or_default().push(m); }
                UiEvent::RemoteInvite { code, from } => self.invite = Some((code, from)),
                UiEvent::RemoteStatus(s) => self.remote_status = Some(s),
                UiEvent::EnrollOk => {
                    self.screen = Screen::Chat;
                    self.joining = false;
                    if let Some(c) = AgentConfig::load() { self.equipo_id = c.equipo_id; }
                }
                UiEvent::EnrollError(e) => { self.joining = false; self.enroll_error = Some(e); }
                UiEvent::Bound(name) => { self.bound = Some(name); self.login_error = None; self.login_pw.clear(); }
                UiEvent::Unbound => { self.bound = None; }
                UiEvent::LoginError(e) => { self.login_error = Some(e); }
            }
        }
    }

    fn enroll_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.heading("Remotix · Conectar al soporte");
            ui.add_space(8.0);
            ui.label("Introduce el código (UUID) del proyecto que te dio tu proveedor.");
            ui.add_space(16.0);
        });
        egui::Grid::new("enroll").num_columns(2).spacing([8.0, 10.0]).show(ui, |ui| {
            ui.label("Servidor");
            ui.text_edit_singleline(&mut self.server);
            ui.end_row();
            ui.label("UUID del proyecto");
            ui.text_edit_singleline(&mut self.uuid);
            ui.end_row();
            ui.label("Nombre de este equipo");
            ui.text_edit_singleline(&mut self.name);
            ui.end_row();
        });
        ui.add_space(12.0);
        if let Some(err) = &self.enroll_error {
            ui.colored_label(egui::Color32::from_rgb(226, 91, 91), err);
        }
        let can = !self.uuid.trim().is_empty() && !self.name.trim().is_empty() && !self.joining;
        if ui.add_enabled(can, egui::Button::new(if self.joining { "Conectando…" } else { "Conectar" })).clicked() {
            self.joining = true;
            self.enroll_error = None;
            let _ = self.action_tx.send(UiAction::Enroll {
                server: self.server.clone(),
                uuid: self.uuid.clone(),
                name: self.name.clone(),
            });
        }
    }

    fn chat_ui(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("channels").default_width(180.0).show(ctx, |ui| {
            ui.add_space(6.0);
            ui.heading("Remotix");
            ui.label(egui::RichText::new(&self.name).small().weak());
            ui.separator();
            ui.label(egui::RichText::new("CANALES").small().weak());
            let mut switch: Option<String> = None;
            for c in &self.channels {
                let sel = self.selected.as_deref() == Some(c.id.as_str());
                if ui.selectable_label(sel, format!("# {}", c.name)).clicked() && !sel {
                    switch = Some(c.id.clone());
                }
            }
            if let Some(id) = switch {
                self.selected = Some(id.clone());
                if !self.msgs.contains_key(&id) {
                    let _ = self.action_tx.send(UiAction::LoadHistory { channel_id: id });
                }
            }
            ui.add_space(8.0);
            ui.separator();
            // Login que "casa" usuario↔PC (opcional).
            if let Some(name) = self.bound.clone() {
                ui.horizontal(|ui| {
                    ui.label(format!("👤 {name}"));
                    if ui.small_button("Salir").clicked() { let _ = self.action_tx.send(UiAction::Logout); }
                });
            } else {
                ui.label(egui::RichText::new("Iniciar sesión (opcional)").small().weak());
                ui.add(egui::TextEdit::singleline(&mut self.login_email).hint_text("email"));
                ui.add(egui::TextEdit::singleline(&mut self.login_pw).password(true).hint_text("contraseña"));
                if ui.button("Entrar").clicked() && !self.login_email.trim().is_empty() {
                    self.login_error = None;
                    let _ = self.action_tx.send(UiAction::Login { email: self.login_email.clone(), password: self.login_pw.clone() });
                }
                if let Some(e) = &self.login_error {
                    ui.colored_label(egui::Color32::from_rgb(226, 91, 91), egui::RichText::new(e).small());
                }
            }
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&self.status).small().weak());
        });

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let name = self.selected.as_ref()
                    .and_then(|id| self.channels.iter().find(|c| &c.id == id))
                    .map(|c| c.name.clone()).unwrap_or_else(|| "—".into());
                ui.heading(format!("# {name}"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("🆘 Pedir soporte").clicked() {
                        if let Some(id) = &self.selected {
                            let _ = self.action_tx.send(UiAction::RequestSupport { channel_id: id.clone() });
                        }
                    }
                });
            });
        });

        // Banner de invitación a control remoto.
        if let Some((code, from)) = self.invite.clone() {
            egui::TopBottomPanel::top("invite").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("🖥️ {from} quiere ver tu pantalla para ayudarte."));
                    if ui.button("Permitir").clicked() {
                        let _ = self.action_tx.send(UiAction::AcceptRemote { code: code.clone() });
                        self.invite = None;
                    }
                    if ui.button("Rechazar").clicked() { self.invite = None; }
                });
            });
        }
        if let Some(rs) = &self.remote_status {
            egui::TopBottomPanel::top("rstatus").show(ctx, |ui| {
                ui.colored_label(egui::Color32::from_rgb(110, 231, 168), format!("🔴 {rs}"));
            });
        }

        egui::TopBottomPanel::bottom("composer").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let resp = ui.add_sized([ui.available_width() - 80.0, 24.0], egui::TextEdit::singleline(&mut self.input).hint_text("Escribe un mensaje…"));
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("Enviar").clicked() || enter) && !self.input.trim().is_empty() {
                    if let Some(id) = &self.selected {
                        let _ = self.action_tx.send(UiAction::Send { channel_id: id.clone(), body: self.input.trim().to_string() });
                    }
                    self.input.clear();
                    resp.request_focus();
                }
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let empty: Vec<MsgInfo> = Vec::new();
            let list = self.selected.as_ref().and_then(|id| self.msgs.get(id)).unwrap_or(&empty);
            egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                for m in list {
                    let author = if m.sender_kind == "system" { "Sistema".to_string() }
                        else if m.sender_id == self.equipo_id { format!("{} (tú)", self.name) }
                        else { format!("{}{}", if m.sender_kind == "pc" { "💻 " } else { "🧑‍💼 " }, short(&m.sender_id)) };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(author).strong());
                        ui.label(egui::RichText::new(&m.body));
                    });
                }
            });
        });
    }
}

fn short(id: &str) -> &str {
    if id.len() > 10 { &id[..10] } else { id }
}

impl eframe::App for AgentApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        match self.screen {
            Screen::Enroll => { egui::CentralPanel::default().show(ctx, |ui| self.enroll_ui(ui)); }
            Screen::Chat => self.chat_ui(ctx),
        }
        // Refrescar periódicamente para procesar eventos del runtime async.
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
    }
}
