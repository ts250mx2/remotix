//! Captura de pantalla (DXGI Desktop Duplication vía scrap), codificación H.264
//! y envío de samples al track WebRTC.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use bytes::Bytes;
use scrap::{Capturer, Display};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use webrtc::media::Sample;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use crate::encode::H264Encoder;

pub struct CaptureHandle {
    stop: Arc<AtomicBool>,
}

impl CaptureHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Dimensiones del monitor primario (para mapear coordenadas de control).
pub fn primary_dims() -> Result<(i32, i32)> {
    let d = Display::primary()?;
    Ok((d.width() as i32, d.height() as i32))
}

/// Número de monitores conectados.
pub fn monitor_count() -> usize {
    Display::all().map(|v| v.len().max(1)).unwrap_or(1)
}

/// Dimensiones (w, h) del monitor `idx`, o None si no existe.
pub fn monitor_dims(idx: usize) -> Option<(i32, i32)> {
    let d = Display::all().ok()?.into_iter().nth(idx)?;
    Some((d.width() as i32, d.height() as i32))
}

/// Abre el `Capturer` del monitor `idx` (cae al primario si el índice no existe).
fn open_display(idx: usize) -> Result<Capturer> {
    let displays = Display::all()?;
    let display = match displays.into_iter().nth(idx) {
        Some(d) => d,
        None => Display::primary()?,
    };
    Ok(Capturer::new(display)?)
}

/// Prueba local: captura un frame y lo codifica, reportando tamaños.
/// Valida que la captura DXGI y el codificador OpenH264 funcionan en esta máquina.
pub fn self_test() -> Result<()> {
    let display = Display::primary()?;
    let mut capturer = Capturer::new(display)?;
    let (cap_w, cap_h) = (capturer.width(), capturer.height());
    let mut encoder = H264Encoder::new(cap_w, cap_h)?;
    info!("self-test: monitor {cap_w}x{cap_h}");

    for attempt in 0..200 {
        match capturer.frame() {
            Ok(frame) => {
                let stride = frame.len() / cap_h;
                let nal = encoder.encode_bgra(&frame, stride)?;
                info!(
                    "self-test OK: frame {} bytes (stride {}), NAL H.264 {} bytes (intento {})",
                    frame.len(),
                    stride,
                    nal.len(),
                    attempt
                );
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!("no se obtuvo ningún frame de la pantalla (DXGI WouldBlock)")
}

/// Arranca captura + codificación + envío al `track`. Devuelve un handle que,
/// al hacer `stop()` o al soltarse (Drop), detiene el hilo de captura.
pub fn start(track: Arc<TrackLocalStaticSample>, fps: u32, selected: Arc<AtomicUsize>) -> CaptureHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let (tx, mut rx) = mpsc::channel::<(Bytes, Duration)>(4);

    std::thread::Builder::new()
        .name("screen-capture".into())
        .spawn(move || {
            if let Err(e) = capture_loop(tx, stop_thread, fps, selected) {
                error!("captura terminó con error: {e:#}");
            }
        })
        .expect("spawn capture thread");

    tokio::spawn(async move {
        while let Some((data, duration)) = rx.recv().await {
            let sample = Sample {
                data,
                duration,
                ..Default::default()
            };
            if let Err(e) = track.write_sample(&sample).await {
                warn!("write_sample falló: {e}");
            }
        }
    });

    CaptureHandle { stop }
}

fn capture_loop(
    tx: mpsc::Sender<(Bytes, Duration)>,
    stop: Arc<AtomicBool>,
    fps: u32,
    selected: Arc<AtomicUsize>,
) -> Result<()> {
    let frame_dur = Duration::from_millis(1000 / fps.max(1) as u64);
    let mut cur_idx = usize::MAX;
    let mut capturer: Option<Capturer> = None;
    let mut cap_h = 0usize;
    let mut encoder: Option<H264Encoder> = None;

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        // (Re)abrir captura+encoder si cambió el monitor seleccionado.
        let want = selected.load(Ordering::SeqCst);
        if want != cur_idx || capturer.is_none() {
            match open_display(want) {
                Ok(cap) => {
                    cap_h = cap.height();
                    match H264Encoder::new(cap.width(), cap.height()) {
                        Ok(enc) => {
                            let (ew, eh) = enc.dims();
                            info!("captura monitor {want}: {}x{} → H.264 {}x{} @ {} fps", cap.width(), cap.height(), ew, eh, fps);
                            capturer = Some(cap);
                            encoder = Some(enc);
                            cur_idx = want;
                        }
                        Err(e) => { warn!("encoder monitor {want}: {e}"); std::thread::sleep(Duration::from_millis(200)); continue; }
                    }
                }
                Err(e) => { warn!("no se pudo abrir monitor {want}: {e}"); std::thread::sleep(Duration::from_millis(200)); continue; }
            }
        }

        let tick = Instant::now();
        let cap = capturer.as_mut().unwrap();
        let enc = encoder.as_mut().unwrap();
        match cap.frame() {
            Ok(frame) => {
                let stride = frame.len() / cap_h.max(1);
                let nal = enc.encode_bgra(&frame, stride)?;
                drop(frame);
                if !nal.is_empty() && tx.blocking_send((Bytes::from(nal), frame_dur)).is_err() {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            Err(e) => {
                // Al cambiar de monitor el capturer viejo puede fallar; recrear.
                warn!("captura: {e}; reabriendo");
                capturer = None;
                encoder = None;
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        }
        let elapsed = tick.elapsed();
        if elapsed < frame_dur {
            std::thread::sleep(frame_dur - elapsed);
        }
    }
    info!("captura detenida");
    Ok(())
}
