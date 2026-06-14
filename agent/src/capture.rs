//! Captura de pantalla (DXGI Desktop Duplication vía scrap), codificación H.264
//! y envío de samples al track WebRTC.

use std::sync::atomic::{AtomicBool, Ordering};
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
pub fn start(track: Arc<TrackLocalStaticSample>, fps: u32) -> CaptureHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let (tx, mut rx) = mpsc::channel::<(Bytes, Duration)>(4);

    std::thread::Builder::new()
        .name("screen-capture".into())
        .spawn(move || {
            if let Err(e) = capture_loop(tx, stop_thread, fps) {
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

fn capture_loop(tx: mpsc::Sender<(Bytes, Duration)>, stop: Arc<AtomicBool>, fps: u32) -> Result<()> {
    let display = Display::primary()?;
    let mut capturer = Capturer::new(display)?;
    let cap_w = capturer.width();
    let cap_h = capturer.height();

    let mut encoder = H264Encoder::new(cap_w, cap_h)?;
    let (enc_w, enc_h) = encoder.dims();
    info!(
        "captura {}x{} → H.264 {}x{} @ {} fps",
        cap_w, cap_h, enc_w, enc_h, fps
    );

    let frame_dur = Duration::from_millis(1000 / fps.max(1) as u64);

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let tick = Instant::now();
        match capturer.frame() {
            Ok(frame) => {
                let stride = frame.len() / cap_h;
                let nal = encoder.encode_bgra(&frame, stride)?;
                drop(frame); // soltar el lock del frame DXGI cuanto antes
                if !nal.is_empty() && tx.blocking_send((Bytes::from(nal), frame_dur)).is_err() {
                    break; // el receptor se cerró
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            Err(e) => return Err(e.into()),
        }
        let elapsed = tick.elapsed();
        if elapsed < frame_dur {
            std::thread::sleep(frame_dur - elapsed);
        }
    }
    info!("captura detenida");
    Ok(())
}
