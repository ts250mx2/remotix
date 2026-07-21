//! Visor nativo (rol OPERADOR): se une a una sala por código, recibe el vídeo
//! H.264 del host, lo decodifica y lo expone como frames RGBA para la GUI; y
//! reenvía el input local (mouse/teclado) por el DataChannel `control`.
//!
//! Es el espejo "answerer" de `session.rs` (que es el host/offerer):
//!   operador → { t:'join', code }  ← { t:'joined' } / { t:'waiting' }
//!   host     → offer (signal)      → operador responde answer (signal)
//!   vídeo H.264 entrante (on_track) → depaquetizar RTP → decode → RGBA → GUI
//!   input local → JSON InputEvent → DataChannel `control` (lo inyecta el host)

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, warn};

use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::{RTCIceCandidate, RTCIceCandidateInit};
use webrtc::ice_transport::ice_credential_type::RTCIceCredentialType;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use webrtc::rtp::codecs::h264::H264Packet;
use webrtc::rtp::packetizer::Depacketizer;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;
use webrtc::track::track_remote::TrackRemote;

use crate::decode::H264Decoder;
use crate::input::InputEvent;
use crate::proto::{IceCandidate, IceServer, Incoming, Outgoing, Sdp, SignalPayload, UrlsField};

/// Tiempo máximo, desde que el operador se une, para que la conexión llegue a
/// `Connected`. Si expira (el equipo no hospedó o el ICE no cruzó), el visor se
/// cierra con un mensaje claro en vez de quedarse en "Conectando…" para siempre.
const VIEWER_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Espera máxima a que el host vuelva tras `host-reconnecting` (actualización en
/// caliente). Un poco mayor que la gracia del servidor (90 s), que es quien
/// normalmente corta antes con `peer-left` si el equipo no regresó.
const HOST_RESUME_TIMEOUT: Duration = Duration::from_secs(100);

/// Frame decodificado listo para subir a una textura (RGBA8, sin premultiplicar).
pub struct DecodedFrame {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// Estado compartido entre la tarea del visor y la GUI.
#[derive(Default)]
pub struct ViewerShared {
    /// Último frame decodificado (latest-wins: la GUI hace `take()` al pintar).
    pub frame: Mutex<Option<DecodedFrame>>,
    pub status: Mutex<String>,
    pub closed: AtomicBool,
    /// Canal de archivos (lo crea el host); la GUI lo usa para enviar/pedir.
    pub files_dc: Mutex<Option<Arc<RTCDataChannel>>>,
    /// Estado del explorador de archivos (listados remotos, progreso, avisos).
    pub files_ui: Arc<crate::files::FilesUi>,
    /// Multimonitor: nº de monitores del host, monitor activo y canal de control.
    pub monitors: AtomicUsize,
    pub active_monitor: AtomicUsize,
    pub meta_dc: Mutex<Option<Arc<RTCDataChannel>>>,
}

impl ViewerShared {
    /// Pública: la GUI también la usa para mostrar errores de la sesión (si la
    /// tarea del visor muere antes de arrancar, el estado no debe quedarse
    /// congelado en "Obteniendo configuración…" sin explicación).
    pub fn set_status(&self, s: impl Into<String>) {
        *self.status.lock() = s.into();
    }
}

/// Crea el estado y el canal de input. La GUI conserva `(shared, input_tx)`; la
/// tarea recibe `(shared, input_rx)` vía `run_viewer_session`.
pub fn new_session() -> (Arc<ViewerShared>, mpsc::UnboundedSender<InputEvent>, mpsc::UnboundedReceiver<InputEvent>) {
    let shared = Arc::new(ViewerShared::default());
    let (tx, rx) = mpsc::unbounded_channel::<InputEvent>();
    (shared, tx, rx)
}

fn send(out_tx: &mpsc::UnboundedSender<String>, msg: &Outgoing) {
    if let Ok(text) = serde_json::to_string(msg) {
        let _ = out_tx.send(text);
    }
}

/// Pide las credenciales ICE/TURN al servidor (el operador no las recibe por la
/// señalización, a diferencia del host). Con timeout y un reintento; si aun así
/// falla, devuelve STUN público de respaldo: sin TURN quizá no cruce redes muy
/// restrictivas, pero con srflx la mayoría conecta — con SOLO candidatos host
/// (lo que pasaba antes al fallar esto) jamás se cruza entre redes distintas.
async fn fetch_ice(server_http: &str) -> Vec<IceServer> {
    #[derive(Deserialize)]
    struct Resp {
        #[serde(default, rename = "iceServers")]
        ice_servers: Vec<IceServer>,
    }
    let url = format!("{}/api/turn-credentials", server_http.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    for intento in 1..=2u32 {
        match client.get(&url).send().await {
            Ok(r) => match r.json::<Resp>().await {
                Ok(x) if !x.ice_servers.is_empty() => return x.ice_servers,
                Ok(_) => {
                    warn!("el servidor devolvió una lista ICE vacía");
                    break;
                }
                Err(e) => warn!("credenciales TURN ilegibles (intento {intento}): {e}"),
            },
            Err(e) => warn!("no se pudieron obtener credenciales TURN (intento {intento}): {e}"),
        }
    }
    warn!("sin ICE del servidor: usando STUN público de respaldo (sin TURN)");
    vec![
        IceServer { urls: UrlsField::One("stun:stun.l.google.com:19302".into()), username: None, credential: None },
        IceServer { urls: UrlsField::One("stun:stun1.l.google.com:19302".into()), username: None, credential: None },
    ]
}

/// Envía un PLI (Picture Loss Indication) por RTCP para pedir un keyframe.
async fn request_keyframe(pc: &Weak<RTCPeerConnection>, ssrc: u32) {
    if let Some(pc) = pc.upgrade() {
        let pli = PictureLossIndication { sender_ssrc: 0, media_ssrc: ssrc };
        let pkts: Vec<Box<dyn webrtc::rtcp::packet::Packet + Send + Sync>> = vec![Box::new(pli)];
        let _ = pc.write_rtcp(&pkts).await;
    }
}

/// Lee RTP del track de vídeo, reensambla Access Units H.264 (Annex-B) y los
/// manda al hilo decodificador.
async fn rtp_reader_loop(track: Arc<TrackRemote>, au_tx: std::sync::mpsc::Sender<Vec<u8>>, pc: Weak<RTCPeerConnection>) {
    let ssrc = track.ssrc();
    // Pide keyframe al arrancar y unas cuantas veces por si llegamos a mitad del GOP.
    {
        let pc2 = pc.clone();
        tokio::spawn(async move {
            for _ in 0..4 {
                request_keyframe(&pc2, ssrc).await;
                tokio::time::sleep(Duration::from_millis(1200)).await;
            }
        });
    }

    let mut depacketizer = H264Packet::default(); // is_avc = false → start codes Annex-B
    let mut au: Vec<u8> = Vec::new();
    let mut last_ts: Option<u32> = None;

    loop {
        match track.read_rtp().await {
            Ok((pkt, _)) => {
                // Cambio de timestamp RTP = frontera de frame.
                if let Some(prev) = last_ts {
                    if pkt.header.timestamp != prev && !au.is_empty() {
                        let _ = au_tx.send(std::mem::take(&mut au));
                    }
                }
                last_ts = Some(pkt.header.timestamp);

                if let Ok(nal) = depacketizer.depacketize(&pkt.payload) {
                    if !nal.is_empty() {
                        au.extend_from_slice(&nal);
                    }
                }
                // El marker bit cierra el Access Unit.
                if pkt.header.marker && !au.is_empty() {
                    let _ = au_tx.send(std::mem::take(&mut au));
                }
            }
            Err(_) => break,
        }
    }
}

/// Hilo decodificador: AUs Annex-B → RGBA → `shared.frame` (latest-wins).
fn spawn_decoder(au_rx: std::sync::mpsc::Receiver<Vec<u8>>, shared: Arc<ViewerShared>) {
    std::thread::Builder::new()
        .name("h264-decoder".into())
        .spawn(move || {
            let mut dec = match H264Decoder::new() {
                Ok(d) => d,
                Err(e) => {
                    error!("no se pudo crear el decoder H.264: {e:#}");
                    return;
                }
            };
            while let Ok(au) = au_rx.recv() {
                match dec.decode(&au) {
                    Ok(Some((w, h, rgba))) => {
                        *shared.frame.lock() = Some(DecodedFrame { w, h, rgba: rgba.to_vec() });
                    }
                    Ok(None) => {}
                    Err(e) => warn!("decode: {e}"),
                }
            }
        })
        .expect("spawn decoder thread");
}

type ControlSlot = Arc<Mutex<Option<Arc<RTCDataChannel>>>>;

async fn build_viewer_peer(
    ice_servers: &[IceServer],
    out_tx: &mpsc::UnboundedSender<String>,
    state_tx: &mpsc::UnboundedSender<RTCPeerConnectionState>,
    au_tx: std::sync::mpsc::Sender<Vec<u8>>,
    control_slot: ControlSlot,
    shared: Arc<ViewerShared>,
) -> Result<Arc<RTCPeerConnection>> {
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

    // Declarar que queremos RECIBIR vídeo.
    pc.add_transceiver_from_kind(
        RTPCodecType::Video,
        Some(RTCRtpTransceiverInit { direction: RTCRtpTransceiverDirection::Recvonly, send_encodings: vec![] }),
    ).await?;

    // Track de vídeo entrante.
    {
        let au_tx = au_tx.clone();
        let weak = Arc::downgrade(&pc);
        pc.on_track(Box::new(move |track, _receiver, _transceiver| {
            let au_tx = au_tx.clone();
            let weak = weak.clone();
            Box::pin(async move {
                if track.kind() == RTPCodecType::Video {
                    tokio::spawn(rtp_reader_loop(track, au_tx, weak));
                }
            })
        }));
    }

    // DataChannels creados por el host (el operador NO los crea, los recibe).
    {
        let slot = control_slot.clone();
        let shared = shared.clone();
        pc.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
            let slot = slot.clone();
            let shared = shared.clone();
            Box::pin(async move {
                match dc.label() {
                    "control" => { *slot.lock() = Some(dc); }
                    "files" => {
                        *shared.files_dc.lock() = Some(dc.clone());
                        crate::files::wire_files_channel_ui(dc, shared.files_ui.clone());
                    }
                    "meta" => {
                        *shared.meta_dc.lock() = Some(dc.clone());
                        let sh = shared.clone();
                        dc.on_message(Box::new(move |msg: DataChannelMessage| {
                            let sh = sh.clone();
                            Box::pin(async move {
                                if msg.is_string {
                                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&msg.data) {
                                        if let Some(n) = v.get("monitors").and_then(|x| x.as_u64()) {
                                            sh.monitors.store(n as usize, Ordering::SeqCst);
                                        }
                                        if let Some(a) = v.get("active").and_then(|x| x.as_u64()) {
                                            sh.active_monitor.store(a as usize, Ordering::SeqCst);
                                        }
                                    }
                                }
                            })
                        }));
                    }
                    _ => {}
                }
            })
        }));
    }

    // Candidatos ICE locales → al host por la señalización.
    {
        let out = out_tx.clone();
        pc.on_ice_candidate(Box::new(move |cand: Option<RTCIceCandidate>| {
            let out = out.clone();
            Box::pin(async move {
                if let Some(c) = cand {
                    if let Ok(init) = c.to_json() {
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

    Ok(pc)
}

/// Procesa una oferta (del host) o un candidato ICE entrante.
async fn handle_signal(
    pc: &Arc<RTCPeerConnection>,
    payload: SignalPayload,
    out_tx: &mpsc::UnboundedSender<String>,
    remote_set: &mut bool,
    pending: &mut Vec<RTCIceCandidateInit>,
) {
    if let Some(sdp) = payload.sdp {
        if sdp.kind == "offer" {
            match RTCSessionDescription::offer(sdp.sdp) {
                Ok(desc) => {
                    if let Err(e) = pc.set_remote_description(desc).await {
                        error!("set_remote(offer): {e}");
                        return;
                    }
                    *remote_set = true;
                    for init in pending.drain(..) {
                        let _ = pc.add_ice_candidate(init).await;
                    }
                    if let Ok(answer) = pc.create_answer(None).await {
                        if pc.set_local_description(answer.clone()).await.is_ok() {
                            send(out_tx, &Outgoing::Signal { payload: SignalPayload {
                                sdp: Some(Sdp { kind: "answer".into(), sdp: answer.sdp }), candidate: None } });
                        }
                    }
                }
                Err(e) => error!("offer inválida: {e}"),
            }
        }
    } else if let Some(c) = payload.candidate {
        let init = RTCIceCandidateInit {
            candidate: c.candidate, sdp_mid: c.sdp_mid, sdp_mline_index: c.sdp_mline_index, username_fragment: c.username_fragment,
        };
        if *remote_set {
            let _ = pc.add_ice_candidate(init).await;
        } else {
            pending.push(init); // aún no tenemos la offer como remote description
        }
    }
}

/// Corre una sesión de visor hasta que el host cierra o se pierde la conexión.
pub async fn run_viewer_session(
    server_http: &str,
    signal_ws_url: &str,
    code: &str,
    shared: Arc<ViewerShared>,
    mut input_rx: mpsc::UnboundedReceiver<InputEvent>,
) -> Result<()> {
    shared.set_status("Obteniendo configuración…");
    let ice_servers = fetch_ice(server_http).await;

    // Con timeout explícito: un WS que no responde no debe dejar la ventana
    // congelada en el estado anterior sin dar señales de vida.
    shared.set_status("Conectando al servidor…");
    let (ws_stream, _) = tokio::time::timeout(
        Duration::from_secs(15),
        tokio_tungstenite::connect_async(signal_ws_url),
    )
    .await
    .map_err(|_| anyhow::anyhow!("el servidor de señalización no respondió en 15 s (revisa tu internet o firewall)"))?
    .context("no se pudo conectar a la señalización")?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if ws_write.send(Message::Text(text)).await.is_err() { break; }
        }
    });
    let (state_tx, mut state_rx) = mpsc::unbounded_channel::<RTCPeerConnectionState>();

    // Hilo decodificador + canal de Access Units.
    let (au_tx, au_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    spawn_decoder(au_rx, shared.clone());

    // Slot del DataChannel de control + tarea que drena el input de la GUI.
    let control_slot: ControlSlot = Arc::new(Mutex::new(None));
    {
        let slot = control_slot.clone();
        tokio::spawn(async move {
            while let Some(evt) = input_rx.recv().await {
                let dc = slot.lock().clone();
                if let Some(dc) = dc {
                    if let Ok(json) = serde_json::to_vec(&evt) {
                        let _ = dc.send(&Bytes::from(json)).await;
                    }
                }
            }
        });
    }

    // Unirse a la sala.
    send(&out_tx, &Outgoing::Join { code: code.to_string() });
    shared.set_status("Conectando…");

    let mut pc = build_viewer_peer(&ice_servers, &out_tx, &state_tx, au_tx.clone(), control_slot.clone(), shared.clone()).await?;
    let mut remote_set = false;
    let mut pending: Vec<RTCIceCandidateInit> = Vec::new();
    // Fecha límite de conexión: activa hasta llegar a `Connected`.
    let mut connect_deadline: Option<tokio::time::Instant> =
        Some(tokio::time::Instant::now() + VIEWER_CONNECT_TIMEOUT);

    loop {
        tokio::select! {
            msg = ws_read.next() => {
                let Some(msg) = msg else { break; };
                let msg = match msg { Ok(m) => m, Err(_) => break };
                let text = match msg { Message::Text(t) => t, Message::Close(_) => break, _ => continue };
                let incoming: Incoming = match serde_json::from_str(&text) { Ok(v) => v, Err(_) => continue };
                match incoming {
                    Incoming::Joined { .. } => shared.set_status("Equipo encontrado, negociando…"),
                    // El equipo se une SOLO al recibir el `start` (sin intervención
                    // de nadie); esto solo es el instante entre reservar y unirse.
                    Incoming::Waiting => shared.set_status("Conectando con el equipo…"),
                    Incoming::Signal { payload } => handle_signal(&pc, payload, &out_tx, &mut remote_set, &mut pending).await,
                    // El host murió a mitad de sesión (actualización en caliente):
                    // el servidor mantiene la sala. Se descarta el peer viejo (su
                    // DTLS murió con aquel proceso) y se espera al nuevo con un
                    // peer LIMPIO; al volver, el host manda una oferta nueva y la
                    // sesión renegocia sola. El último frame queda congelado en
                    // pantalla mientras tanto.
                    Incoming::HostReconnecting => {
                        shared.set_status("El equipo remoto se está actualizando… esperando a que vuelva");
                        let _ = pc.close().await;
                        // Canal de estado NUEVO: los Closed/Failed del peer viejo
                        // no deben matar la espera.
                        let (ntx, nrx) = mpsc::unbounded_channel::<RTCPeerConnectionState>();
                        state_rx = nrx;
                        pc = build_viewer_peer(&ice_servers, &out_tx, &ntx, au_tx.clone(), control_slot.clone(), shared.clone()).await?;
                        remote_set = false;
                        pending.clear();
                        // Una transferencia de archivos a medias no sobrevive.
                        *shared.files_ui.progress.lock() = None;
                        connect_deadline = Some(tokio::time::Instant::now() + HOST_RESUME_TIMEOUT);
                    }
                    Incoming::PeerLeft => { shared.set_status("El equipo cerró la sesión."); break; }
                    Incoming::Error { code } => {
                        let msg = match code.as_str() {
                            // Modo "pedir permiso": el usuario del equipo dijo que no
                            // (o nadie respondió al diálogo a tiempo).
                            "declined" => "El usuario del equipo rechazó la conexión.".to_string(),
                            c => format!("Error: {c}"),
                        };
                        shared.set_status(msg);
                        break;
                    }
                    _ => {}
                }
            }
            Some(state) = state_rx.recv() => {
                match state {
                    RTCPeerConnectionState::Connected => { connect_deadline = None; shared.set_status("Conectado"); }
                    RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => { shared.set_status("Conexión cerrada."); break; }
                    RTCPeerConnectionState::Disconnected => {
                        // Da un margen para recuperar antes de rendirse.
                        connect_deadline = Some(tokio::time::Instant::now() + VIEWER_CONNECT_TIMEOUT);
                        shared.set_status("Reconectando…");
                    }
                    _ => {}
                }
            }
            // Guardia de tiempo: si no se llega a `Connected`, cerrar con mensaje.
            () = async {
                match connect_deadline {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                warn!("visor: timeout de conexión ({}s) sin vídeo; cerrando", VIEWER_CONNECT_TIMEOUT.as_secs());
                shared.set_status("No se pudo conectar con el equipo (revisa la red o intenta de nuevo).");
                break;
            }
        }
    }

    shared.closed.store(true, Ordering::SeqCst);
    let _ = pc.close().await;
    Ok(())
}
