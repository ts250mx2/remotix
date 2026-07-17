//! Cliente de chat del agente: conexión WS persistente a /ws/chat (auth como PC),
//! puente con la GUI por canales, y lanzamiento de sesiones de control remoto.

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use crate::config::AgentConfig;
use crate::session::run_remote_session;

// ---- puente GUI ----

#[derive(Clone)]
pub struct ChannelInfo { pub id: String, pub name: String }

#[derive(Clone)]
pub struct MsgInfo {
    pub id: String,
    pub channel_id: String,
    pub sender_id: String,
    pub sender_kind: String,
    pub body: String,
}

pub enum UiEvent {
    Status(String),
    Channels(Vec<ChannelInfo>),
    History(String, Vec<MsgInfo>),
    Message(MsgInfo),
    RemoteInvite { code: String, from: String },
    RemoteStatus(String),
    EnrollOk,
    EnrollError(String),
    Bound(String),
    Unbound,
    LoginError(String),
}

pub enum UiAction {
    Enroll { server: String, uuid: String, name: String },
    Send { channel_id: String, body: String },
    RequestSupport { channel_id: String },
    LoadHistory { channel_id: String },
    AcceptRemote { code: String },
    Login { email: String, password: String },
    Logout,
}

// ---- mensajes entrantes del hub (claves camelCase de Node) ----

#[derive(Deserialize)]
struct WireChannel { id: String, name: String }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireMsg {
    id: String,
    channel_id: String,
    sender_id: String,
    sender_kind: String,
    #[serde(default)]
    body: String,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ChatIn {
    #[serde(rename = "ready")]
    Ready { #[serde(default)] channels: Vec<WireChannel> },
    #[serde(rename = "history")]
    History { #[serde(rename = "channelId")] channel_id: String, #[serde(default)] messages: Vec<WireMsg> },
    #[serde(rename = "message")]
    Message { message: WireMsg },
    #[serde(rename = "remote-invite")]
    RemoteInvite { code: String, #[serde(default)] from: Option<String> },
    #[serde(other)]
    Other,
}

fn to_msg(w: WireMsg) -> MsgInfo {
    MsgInfo { id: w.id, channel_id: w.channel_id, sender_id: w.sender_id, sender_kind: w.sender_kind, body: w.body }
}

/// Enrola el equipo con el UUID del proyecto y devuelve la config.
async fn enroll(server: &str, uuid: &str, name: &str) -> anyhow::Result<AgentConfig> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct JoinResp { equipo_id: String, agent_secret: String, project_id: String }
    let url = format!("{}/api/agent/join", crate::config::to_http(server));
    let resp = reqwest::Client::new().post(&url)
        .json(&serde_json::json!({ "projectId": uuid.trim(), "name": name }))
        .send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("UUID inválido o servidor no disponible ({})", resp.status());
    }
    let j: JoinResp = resp.json().await?;
    Ok(AgentConfig {
        server: server.trim().to_string(),
        equipo_id: j.equipo_id,
        agent_secret: j.agent_secret,
        project_id: j.project_id,
        name: name.to_string(),
    })
}

/// Prueba headless del camino de chat del agente: enrola, conecta, autentica,
/// recibe `ready`, envía un mensaje y verifica que lo recibe de vuelta.
pub async fn self_test_chat(server: &str, uuid: &str) -> anyhow::Result<()> {
    use std::time::Duration;
    let cfg = enroll(server, uuid, "AgentSelfTest").await?;
    println!("enrolado: equipo={}", cfg.equipo_id);
    let (ws, _) = tokio_tungstenite::connect_async(cfg.ws_chat_url()).await?;
    let (mut w, mut r) = ws.split();
    w.send(Message::Text(serde_json::json!({ "type": "auth", "equipoId": cfg.equipo_id, "agentSecret": cfg.agent_secret }).to_string())).await?;

    let mut general: Option<String> = None;
    let mut sent = false;
    let probe = "ping-agente-selftest";
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() { anyhow::bail!("timeout en self_test_chat"); }
        let Ok(Some(msg)) = tokio::time::timeout(remaining, r.next()).await else { anyhow::bail!("timeout/cierre"); };
        let text = match msg? { Message::Text(t) => t, _ => continue };
        match serde_json::from_str::<ChatIn>(&text)? {
            ChatIn::Ready { channels } => {
                println!("ready: {} canal(es)", channels.len());
                general = channels.into_iter().next().map(|c| c.id);
                if let Some(id) = &general {
                    w.send(Message::Text(serde_json::json!({ "type": "message", "channelId": id, "body": probe }).to_string())).await?;
                    sent = true;
                }
            }
            ChatIn::Message { message } if sent && message.body == probe && message.sender_kind == "pc" => {
                println!("OK: mensaje del agente recibido de vuelta (senderKind=pc)");
                return Ok(());
            }
            _ => {}
        }
    }
}

/// Login: casa al usuario con este PC. Devuelve el nombre del usuario.
async fn bind_user(cfg: &AgentConfig, email: &str, password: &str) -> anyhow::Result<String> {
    #[derive(Deserialize)]
    struct BindResp { name: String }
    let url = format!("{}/api/agent/bind", cfg.http_base());
    let resp = reqwest::Client::new().post(&url)
        .json(&serde_json::json!({ "equipoId": cfg.equipo_id, "agentSecret": cfg.agent_secret, "email": email.trim(), "password": password }))
        .send().await?;
    if !resp.status().is_success() { anyhow::bail!("Credenciales inválidas"); }
    Ok(resp.json::<BindResp>().await?.name)
}

async fn unbind_user(cfg: &AgentConfig) -> anyhow::Result<()> {
    let url = format!("{}/api/agent/unbind", cfg.http_base());
    reqwest::Client::new().post(&url)
        .json(&serde_json::json!({ "equipoId": cfg.equipo_id, "agentSecret": cfg.agent_secret }))
        .send().await?;
    Ok(())
}

/// Orquestador: enrola si hace falta y luego corre el chat (con reconexión).
pub async fn run_agent(
    ui_tx: std::sync::mpsc::Sender<UiEvent>,
    mut action_rx: UnboundedReceiver<UiAction>,
) {
    let cfg = match AgentConfig::load() {
        Some(c) => c,
        None => {
            // Esperar a que el usuario complete el enrolamiento desde la GUI.
            loop {
                match action_rx.recv().await {
                    Some(UiAction::Enroll { server, uuid, name }) => match enroll(&server, &uuid, &name).await {
                        Ok(cfg) => { cfg.save(); let _ = ui_tx.send(UiEvent::EnrollOk); break cfg; }
                        Err(e) => { let _ = ui_tx.send(UiEvent::EnrollError(e.to_string())); }
                    },
                    Some(_) => {}
                    None => return,
                }
            }
        }
    };

    loop {
        let _ = ui_tx.send(UiEvent::Status("Conectando al chat…".into()));
        if let Err(e) = connect_once(&cfg, &ui_tx, &mut action_rx).await {
            warn!("chat desconectado: {e}");
        }
        let _ = ui_tx.send(UiEvent::Status("Sin conexión. Reintentando…".into()));
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
    }
}

async fn connect_once(
    cfg: &AgentConfig,
    ui_tx: &std::sync::mpsc::Sender<UiEvent>,
    action_rx: &mut UnboundedReceiver<UiAction>,
) -> anyhow::Result<()> {
    let (ws, _) = tokio_tungstenite::connect_async(cfg.ws_chat_url()).await?;
    let (mut write, mut read) = ws.split();

    // Autenticarse como PC.
    let auth = serde_json::json!({ "type": "auth", "equipoId": cfg.equipo_id, "agentSecret": cfg.agent_secret });
    write.send(Message::Text(auth.to_string())).await?;

    loop {
        tokio::select! {
            incoming = read.next() => {
                let Some(incoming) = incoming else { break; };
                let text = match incoming? { Message::Text(t) => t, Message::Close(_) => break, _ => continue };
                match serde_json::from_str::<ChatIn>(&text) {
                    Ok(ChatIn::Ready { channels }) => {
                        let _ = ui_tx.send(UiEvent::Status("Conectado".into()));
                        let chans: Vec<ChannelInfo> = channels.into_iter().map(|c| ChannelInfo { id: c.id, name: c.name }).collect();
                        let _ = ui_tx.send(UiEvent::Channels(chans));
                    }
                    Ok(ChatIn::History { channel_id, messages }) => {
                        let _ = ui_tx.send(UiEvent::History(channel_id, messages.into_iter().map(to_msg).collect()));
                    }
                    Ok(ChatIn::Message { message }) => { let _ = ui_tx.send(UiEvent::Message(to_msg(message))); }
                    Ok(ChatIn::RemoteInvite { code, from }) => {
                        let _ = ui_tx.send(UiEvent::RemoteInvite { code, from: from.unwrap_or_else(|| "El técnico".into()) });
                    }
                    _ => {}
                }
            }
            action = action_rx.recv() => {
                let Some(action) = action else { break; };
                match action {
                    UiAction::Enroll { .. } => {} // ya enrolado
                    UiAction::Send { channel_id, body } => {
                        let m = serde_json::json!({ "type": "message", "channelId": channel_id, "body": body });
                        write.send(Message::Text(m.to_string())).await?;
                    }
                    UiAction::RequestSupport { channel_id } => {
                        let m = serde_json::json!({ "type": "support", "channelId": channel_id });
                        write.send(Message::Text(m.to_string())).await?;
                    }
                    UiAction::LoadHistory { channel_id } => {
                        let m = serde_json::json!({ "type": "history", "channelId": channel_id });
                        write.send(Message::Text(m.to_string())).await?;
                    }
                    UiAction::AcceptRemote { code } => {
                        let signal_url = cfg.ws_signal_url();
                        let name = cfg.name.clone();
                        let tx = ui_tx.clone();
                        tokio::spawn(async move {
                            let _ = tx.send(UiEvent::RemoteStatus("Compartiendo pantalla con el técnico…".into()));
                            if let Err(e) = run_remote_session(&signal_url, &name, &code, None).await {
                                let _ = tx.send(UiEvent::RemoteStatus(format!("Sesión terminada: {e}")));
                            } else {
                                let _ = tx.send(UiEvent::RemoteStatus("Sesión de control finalizada.".into()));
                            }
                        });
                    }
                    UiAction::Login { email, password } => {
                        match bind_user(cfg, &email, &password).await {
                            Ok(name) => { let _ = ui_tx.send(UiEvent::Bound(name)); }
                            Err(e) => { let _ = ui_tx.send(UiEvent::LoginError(e.to_string())); }
                        }
                    }
                    UiAction::Logout => {
                        let _ = unbind_user(cfg).await;
                        let _ = ui_tx.send(UiEvent::Unbound);
                    }
                }
            }
        }
    }
    info!("conexión de chat cerrada");
    Ok(())
}
