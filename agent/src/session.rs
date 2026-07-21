//! Sesión de control remoto: el agente hospeda en /ws/signal con un código dado
//! (reservado por el técnico desde el chat), comparte pantalla por WebRTC y
//! recibe input. Termina cuando el técnico se desconecta.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};

use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::ice_transport::ice_candidate::{RTCIceCandidate, RTCIceCandidateInit};
use webrtc::ice_transport::ice_credential_type::RTCIceCredentialType;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

use crate::capture::{self, CaptureHandle};
use crate::input::{self, InputEvent};
use crate::proto::{IceCandidate, IceServer, Incoming, Outgoing, Sdp, SignalPayload};

const TARGET_FPS: u32 = 20;

/// Tiempo máximo para que la negociación WebRTC llegue a `Connected` desde que el
/// técnico se une. Si expira, la sesión se aborta (libera el equipo para reintentar)
/// en vez de quedarse colgada mostrando "conectado" sin vídeo — el síntoma clásico
/// de un TURN inalcanzable en internet público.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(25);

/// Margen para recuperar una conexión que pasó a `Disconnected` (bache de red)
/// antes de darla por perdida.
const RECONNECT_GRACE: Duration = Duration::from_secs(12);

/// Tipo de candidato ICE a partir de su cadena SDP (para diagnosticar por qué no
/// conecta: sin candidatos `relay` en ambos lados, un NAT restrictivo no cruza).
fn cand_type(sdp: &str) -> &'static str {
    if sdp.contains(" typ relay") {
        "relay"
    } else if sdp.contains(" typ srflx") {
        "srflx"
    } else if sdp.contains(" typ prflx") {
        "prflx"
    } else if sdp.contains(" typ host") {
        "host"
    } else {
        "?"
    }
}

/// Eventos de la sesión "lite" hacia la GUI.
pub enum LiteEvent {
    Code(String),
    Status(String),
    /// El servidor avisó (push por /ws/device) que hay una versión más nueva.
    UpdateAvailable,
    /// Valor actual (según el servidor) del toggle "pedir permiso antes de
    /// conectar". Llega en el `ready` y cada vez que alguien lo cambia.
    ConfirmMode(bool),
}

/// Sesión estilo TeamViewer QuickSupport: hospeda SIN código (el server genera
/// la clave), la reporta a la GUI, comparte pantalla + control + archivos, y
/// sigue activa entre conexiones (la clave no cambia hasta cerrar).
pub async fn run_lite_session(signal_ws_url: &str, name: &str, ui: std::sync::mpsc::Sender<LiteEvent>) -> Result<()> {
    let selected = Arc::new(AtomicUsize::new(0));
    let force_idr = Arc::new(AtomicBool::new(false));
    let (iw, ih) = capture::monitor_dims(0).unwrap_or((1920, 1080));
    let monitor_rect: input::MonitorRect = Arc::new(Mutex::new(crate::monitors::rect_for(0, iw, ih)));
    let input_tx = input::spawn_injector(monitor_rect.clone());

    let (ws_stream, _) = tokio_tungstenite::connect_async(signal_ws_url).await
        .context("no se pudo conectar al servidor")?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if ws_write.send(Message::Text(text)).await.is_err() { break; }
        }
    });
    let (state_tx, mut state_rx) = mpsc::unbounded_channel::<RTCPeerConnectionState>();

    // host SIN código → el servidor genera la clave.
    send(&out_tx, &Outgoing::Host { name: Some(name.to_string()), mode: "agent", caps: vec!["control"], code: None });

    let mut ice_servers: Vec<IceServer> = Vec::new();
    let mut pc: Option<Arc<RTCPeerConnection>> = None;
    let mut track: Option<Arc<TrackLocalStaticSample>> = None;
    let mut control_dc: Option<Arc<webrtc::data_channel::RTCDataChannel>> = None;
    let mut capture_handle: Option<CaptureHandle> = None;

    loop {
        tokio::select! {
            msg = ws_read.next() => {
                let Some(msg) = msg else { break; };
                let msg = match msg { Ok(m) => m, Err(_) => break };
                let text = match msg { Message::Text(t) => t, Message::Close(_) => break, _ => continue };
                let incoming: Incoming = match serde_json::from_str(&text) { Ok(v) => v, Err(_) => continue };
                match incoming {
                    Incoming::Hosted { code, ice_servers: ice } => {
                        ice_servers = ice;
                        let _ = ui.send(LiteEvent::Code(code));
                        let _ = ui.send(LiteEvent::Status("Esperando a que el técnico se conecte…".into()));
                    }
                    Incoming::PeerJoined => {
                        let _ = ui.send(LiteEvent::Status("Técnico conectándose…".into()));
                        match build_peer(&ice_servers, &out_tx, &state_tx, &input_tx, selected.clone(), monitor_rect.clone(), force_idr.clone()).await {
                            Ok((new_pc, new_track, dc)) => {
                                if let Ok(offer) = new_pc.create_offer(None).await {
                                    if new_pc.set_local_description(offer.clone()).await.is_ok() {
                                        send(&out_tx, &Outgoing::Signal { payload: SignalPayload {
                                            sdp: Some(Sdp { kind: "offer".into(), sdp: offer.sdp }), candidate: None } });
                                    }
                                }
                                pc = Some(new_pc); track = Some(new_track); control_dc = Some(dc);
                            }
                            Err(e) => error!("peer: {e:#}"),
                        }
                    }
                    Incoming::Signal { payload } => { if let Some(p) = pc.as_ref() { handle_signal(p, payload).await; } }
                    Incoming::PeerLeft => {
                        // El técnico se fue: dejamos de compartir pero seguimos hospedando (misma clave).
                        capture_handle = None;
                        if let Some(p) = pc.take() { let _ = p.close().await; }
                        track = None; control_dc = None;
                        let _ = ui.send(LiteEvent::Status("Esperando a que el técnico se conecte…".into()));
                    }
                    Incoming::Error { code } => { warn!("signal: {code}"); }
                    _ => {}
                }
            }
            Some(state) = state_rx.recv() => {
                match state {
                    RTCPeerConnectionState::Connected => {
                        if capture_handle.is_none() {
                            if let Some(t) = track.clone() { capture_handle = Some(capture::start(t, TARGET_FPS, selected.clone(), force_idr.clone())); }
                        }
                        let _ = ui.send(LiteEvent::Status("Conectado · compartiendo tu pantalla".into()));
                    }
                    RTCPeerConnectionState::Failed => {
                        capture_handle = None;
                        if let Some(p) = pc.take() { let _ = p.close().await; }
                        track = None; control_dc = None;
                        let _ = ui.send(LiteEvent::Status("Conexión perdida. Esperando al técnico…".into()));
                    }
                    _ => {}
                }
            }
        }
    }
    let _ = control_dc;
    Ok(())
}

/// Hospeda una sesión de control con `code` y comparte pantalla hasta que el
/// técnico se desconecta (peer-left) o se cierra la conexión.
///
/// `ui` (opcional) recibe el estado REAL de la conexión: "Estableciendo…" al
/// negociar y "Conectado" solo cuando WebRTC lo confirma —no antes—. Si la
/// negociación no llega a `Connected` dentro de `CONNECT_TIMEOUT`, la sesión se
/// aborta para no dejar el equipo pegado en `busy` (así el técnico puede reintentar).
pub async fn run_remote_session(
    signal_ws_url: &str,
    name: &str,
    code: &str,
    ui: Option<std::sync::mpsc::Sender<LiteEvent>>,
) -> Result<()> {
    let notify = |s: &str| {
        if let Some(tx) = &ui {
            let _ = tx.send(LiteEvent::Status(s.to_string()));
        }
    };

    let selected = Arc::new(AtomicUsize::new(0));
    let force_idr = Arc::new(AtomicBool::new(false));
    let (iw, ih) = capture::monitor_dims(0).unwrap_or((1920, 1080));
    let monitor_rect: input::MonitorRect = Arc::new(Mutex::new(crate::monitors::rect_for(0, iw, ih)));
    let input_tx = input::spawn_injector(monitor_rect.clone());

    let (ws_stream, _) = tokio_tungstenite::connect_async(signal_ws_url)
        .await
        .context("no se pudo conectar a la señalización")?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if ws_write.send(Message::Text(text)).await.is_err() { break; }
        }
    });

    let (state_tx, mut state_rx) = mpsc::unbounded_channel::<RTCPeerConnectionState>();

    // host con el código reservado, en modo agente (control total).
    send(&out_tx, &Outgoing::Host {
        name: Some(name.to_string()),
        mode: "agent",
        caps: vec!["control"],
        code: Some(code.to_string()),
    });

    let mut ice_servers: Vec<IceServer> = Vec::new();
    let mut pc: Option<Arc<RTCPeerConnection>> = None;
    let mut track: Option<Arc<TrackLocalStaticSample>> = None;
    let mut control_dc: Option<Arc<webrtc::data_channel::RTCDataChannel>> = None;
    let mut capture_handle: Option<CaptureHandle> = None;
    // Fecha límite para llegar a `Connected`. Se arma desde el arranque (cubre el
    // caso "el técnico nunca llega a unirse"), se re-arma al unirse el técnico y
    // tras un `Disconnected`, y se limpia (`None`) una vez conectados.
    let mut connect_deadline: Option<tokio::time::Instant> =
        Some(tokio::time::Instant::now() + CONNECT_TIMEOUT);

    loop {
        tokio::select! {
            msg = ws_read.next() => {
                let Some(msg) = msg else { break; };
                let msg = match msg { Ok(m) => m, Err(e) => { warn!("WS: {e}"); break; } };
                let text = match msg {
                    Message::Text(t) => t,
                    Message::Close(_) => break,
                    _ => continue,
                };
                let incoming: Incoming = match serde_json::from_str(&text) { Ok(v) => v, Err(_) => continue };
                match incoming {
                    Incoming::Hosted { ice_servers: ice, .. } => {
                        let relay = ice.iter().filter(|s| s.urls.clone().into_vec().iter().any(|u| u.starts_with("turn"))).count();
                        info!("señalización lista: {} ICE server(s), {} con TURN", ice.len(), relay);
                        if relay == 0 {
                            warn!("el servidor no entregó TURN: en internet público con NAT restrictivo la conexión puede no establecerse");
                        }
                        ice_servers = ice;
                    }
                    Incoming::PeerJoined => {
                        info!("técnico se unió; negociando WebRTC");
                        notify("Estableciendo conexión con el técnico…");
                        match build_peer(&ice_servers, &out_tx, &state_tx, &input_tx, selected.clone(), monitor_rect.clone(), force_idr.clone()).await {
                            Ok((new_pc, new_track, dc)) => {
                                if let Ok(offer) = new_pc.create_offer(None).await {
                                    if new_pc.set_local_description(offer.clone()).await.is_ok() {
                                        send(&out_tx, &Outgoing::Signal { payload: SignalPayload {
                                            sdp: Some(Sdp { kind: "offer".into(), sdp: offer.sdp }), candidate: None } });
                                    }
                                }
                                pc = Some(new_pc); track = Some(new_track); control_dc = Some(dc);
                                connect_deadline = Some(tokio::time::Instant::now() + CONNECT_TIMEOUT);
                            }
                            Err(e) => {
                                error!("no se pudo crear el peer: {e:#}");
                                notify("Error preparando la conexión.");
                                break;
                            }
                        }
                    }
                    Incoming::Signal { payload } => { if let Some(p) = pc.as_ref() { handle_signal(p, payload).await; } }
                    Incoming::PeerLeft => { info!("técnico desconectado, fin de sesión"); break; }
                    // p. ej. 'taken': otro proceso de este equipo (GUI/ayudante) ya
                    // atendió el start. Terminar libera el flag `busy` del device.
                    Incoming::Error { code } => { warn!("signal error: {code}; fin de sesión"); break; }
                    _ => {}
                }
            }
            Some(state) = state_rx.recv() => {
                info!("estado WebRTC: {state:?}");
                match state {
                    RTCPeerConnectionState::Connected => {
                        connect_deadline = None;
                        if capture_handle.is_none() {
                            if let Some(t) = track.clone() { capture_handle = Some(capture::start(t, TARGET_FPS, selected.clone(), force_idr.clone())); }
                        }
                        notify("Conectado · compartiendo tu pantalla");
                    }
                    RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => {
                        warn!("conexión WebRTC {state:?}; fin de sesión");
                        break;
                    }
                    RTCPeerConnectionState::Disconnected => {
                        // Bache de red: paramos captura y damos un margen para recuperar.
                        capture_handle = None;
                        connect_deadline = Some(tokio::time::Instant::now() + RECONNECT_GRACE);
                        notify("Reconectando…");
                    }
                    _ => {}
                }
            }
            // Guardia de tiempo: si `connect_deadline` está activo y expira sin
            // llegar a `Connected`, abortamos (evita el cuelgue "conectado sin vídeo"
            // y libera el equipo). Con `None`, este futuro nunca resuelve.
            () = async {
                match connect_deadline {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                warn!("timeout de conexión WebRTC ({}s) sin llegar a Connected; abortando para liberar el equipo", CONNECT_TIMEOUT.as_secs());
                notify("No se pudo establecer la conexión (revisa red/TURN).");
                break;
            }
        }
    }

    drop(capture_handle);
    let _ = control_dc;
    if let Some(p) = pc.take() { let _ = p.close().await; }
    Ok(())
}

async fn handle_signal(pc: &Arc<RTCPeerConnection>, payload: SignalPayload) {
    if let Some(sdp) = payload.sdp {
        if sdp.kind == "answer" {
            if let Ok(desc) = RTCSessionDescription::answer(sdp.sdp) {
                if let Err(e) = pc.set_remote_description(desc).await { error!("set_remote: {e}"); }
            }
        }
    } else if let Some(c) = payload.candidate {
        info!("candidato remoto ICE: {}", cand_type(&c.candidate));
        let init = RTCIceCandidateInit { candidate: c.candidate, sdp_mid: c.sdp_mid, sdp_mline_index: c.sdp_mline_index, username_fragment: c.username_fragment };
        let _ = pc.add_ice_candidate(init).await;
    }
}

async fn build_peer(
    ice_servers: &[IceServer],
    out_tx: &mpsc::UnboundedSender<String>,
    state_tx: &mpsc::UnboundedSender<RTCPeerConnectionState>,
    input_tx: &std::sync::mpsc::Sender<InputEvent>,
    selected: Arc<AtomicUsize>,
    monitor_rect: input::MonitorRect,
    force_idr: Arc<AtomicBool>,
) -> Result<(Arc<RTCPeerConnection>, Arc<TrackLocalStaticSample>, Arc<webrtc::data_channel::RTCDataChannel>)> {
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m)?;
    let api = APIBuilder::new().with_media_engine(m).with_interceptor_registry(registry).build();

    let servers: Vec<RTCIceServer> = ice_servers.iter().map(|s| RTCIceServer {
        urls: s.urls.clone().into_vec(),
        username: s.username.clone().unwrap_or_default(),
        credential: s.credential.clone().unwrap_or_default(),
        credential_type: RTCIceCredentialType::Password,
    }).collect();

    let pc = Arc::new(api.new_peer_connection(RTCConfiguration { ice_servers: servers, ..Default::default() }).await?);

    let track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(), clock_rate: 90000, channels: 0,
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_owned(),
            rtcp_feedback: vec![],
        },
        "video".to_owned(), "remotix-screen".to_owned(),
    ));
    let sender = pc.add_track(track.clone() as Arc<dyn TrackLocal + Send + Sync>).await?;

    // El visor pide keyframes por RTCP (PLI/FIR) cuando pierde paquetes o se
    // une a mitad de stream; se lo pasamos al encoder vía el flag `force_idr`.
    {
        let force_idr = force_idr.clone();
        tokio::spawn(async move {
            while let Ok((pkts, _)) = sender.read_rtcp().await {
                let wants_idr = pkts.iter().any(|p| {
                    let any = p.as_any();
                    any.downcast_ref::<webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication>().is_some()
                        || any.downcast_ref::<webrtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest>().is_some()
                });
                if wants_idr {
                    force_idr.store(true, Ordering::SeqCst);
                }
            }
        });
    }

    let dc = pc.create_data_channel("control", None).await?;
    {
        let in_tx = input_tx.clone();
        dc.on_message(Box::new(move |msg: DataChannelMessage| {
            let in_tx = in_tx.clone();
            Box::pin(async move { if let Ok(evt) = serde_json::from_slice::<InputEvent>(&msg.data) { let _ = in_tx.send(evt); } })
        }));
    }

    let files_dc = pc.create_data_channel("files", None).await?;
    crate::files::wire_files_channel(files_dc);

    // Canal "meta": informa los monitores al operador y recibe la selección.
    let meta_dc = pc.create_data_channel("meta", None).await?;
    {
        let dc_open = meta_dc.clone();
        meta_dc.on_open(Box::new(move || {
            let dc = dc_open.clone();
            Box::pin(async move {
                let n = crate::capture::monitor_count();
                let msg = serde_json::json!({ "monitors": n, "active": 0 }).to_string();
                let _ = dc.send_text(msg).await;
            })
        }));
        let sel = selected.clone();
        let rect = monitor_rect.clone();
        meta_dc.on_message(Box::new(move |msg: DataChannelMessage| {
            let sel = sel.clone();
            let rect = rect.clone();
            Box::pin(async move {
                if !msg.is_string {
                    return;
                }
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&msg.data) {
                    if let Some(idx) = v.get("select").and_then(|x| x.as_u64()) {
                        let idx = idx as usize;
                        sel.store(idx, Ordering::SeqCst);
                        if let Some((w, h)) = crate::capture::monitor_dims(idx) {
                            *rect.lock() = crate::monitors::rect_for(idx, w, h);
                        }
                    }
                }
            })
        }));
    }

    {
        let out = out_tx.clone();
        pc.on_ice_candidate(Box::new(move |cand: Option<RTCIceCandidate>| {
            let out = out.clone();
            Box::pin(async move {
                if let Some(c) = cand {
                    if let Ok(init) = c.to_json() {
                        info!("candidato local ICE: {}", cand_type(&init.candidate));
                        let payload = SignalPayload { sdp: None, candidate: Some(IceCandidate {
                            candidate: init.candidate, sdp_mid: init.sdp_mid, sdp_mline_index: init.sdp_mline_index, username_fragment: init.username_fragment }) };
                        if let Ok(text) = serde_json::to_string(&Outgoing::Signal { payload }) { let _ = out.send(text); }
                    }
                }
            })
        }));
    }
    {
        let st = state_tx.clone();
        pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
            let st = st.clone();
            Box::pin(async move { let _ = st.send(s); })
        }));
    }

    Ok((pc, track, dc))
}

fn send(out_tx: &mpsc::UnboundedSender<String>, msg: &Outgoing) {
    if let Ok(text) = serde_json::to_string(msg) { let _ = out_tx.send(text); }
}
