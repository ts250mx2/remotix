//! Comparativa de calidad de codificación para elegir la config del host.
//! Codifica el mismo "escritorio sintético" 1080p (texto de bordes duros +
//! ventana en movimiento que luego se detiene) con varias configuraciones y
//! mide bitrate efectivo, frames saltados y PSNR (global y en el tramo
//! estático final, que es lo que el usuario percibe al leer la pantalla).
//!
//! Es una medición manual, no una regresión de CI:
//!   cargo test --test quality_compare -- --ignored --nocapture

use openh264::encoder::{
    BitRate, Encoder as RawEncoder, EncoderConfig, FrameRate, QpRange, RateControlMode, UsageType,
};
use openh264::OpenH264API;

use remotix_agent::decode::H264Decoder;
use remotix_agent::encode::{H264Encoder, I420};

const W: usize = 1920;
const H: usize = 1080;
const FPS: u32 = 20;
const FRAMES: usize = 40;
const STATIC_FROM: usize = 20; // desde aquí la escena queda quieta

/// LCG determinista (sin crates externos) para colocar los "glifos".
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

/// Frame BGRA tipo escritorio: fondo claro, barra de tareas, y miles de trazos
/// oscuros finos que imitan texto (bordes de alto contraste, lo que más sufre
/// con poco bitrate). La "ventana" se mueve hasta STATIC_FROM y luego se queda
/// quieta (como un escritorio real en el que se lee).
fn desktop_frame(t: usize) -> Vec<u8> {
    let mut v = vec![0u8; W * H * 4];
    for px in v.chunks_exact_mut(4) {
        px[0] = 0xF0; px[1] = 0xF2; px[2] = 0xF4; px[3] = 0xFF;
    }
    for y in (H - 48)..H {
        for x in 0..W {
            let i = (y * W + x) * 4;
            v[i] = 0x60; v[i + 1] = 0x30; v[i + 2] = 0x10; v[i + 3] = 0xFF;
        }
    }
    // "Texto": trazos horizontales de 1 px, como renglones de letras.
    let mut rng = Rng(0xC0FFEE);
    for row in 0..60 {
        let y0 = 20 + row * 16;
        if y0 + 10 >= H - 48 { break; }
        let mut x = 40usize;
        while x < W - 60 {
            let wlen = 3 + (rng.next() % 9) as usize;
            for dx in 0..wlen {
                let i = (y0 * W + x + dx) * 4;
                v[i] = 0x20; v[i + 1] = 0x20; v[i + 2] = 0x20;
                let i2 = ((y0 + 6) * W + x + dx) * 4;
                v[i2] = 0x20; v[i2 + 1] = 0x20; v[i2 + 2] = 0x20;
            }
            x += wlen + 2 + (rng.next() % 4) as usize;
        }
    }
    // Ventana blanca con borde; se desplaza hasta quedarse quieta.
    let wx = 200 + t.min(STATIC_FROM) * 12;
    for y in 300..700 {
        for x in wx..(wx + 500).min(W) {
            let i = (y * W + x) * 4;
            let border = y < 304 || y > 696 || x < wx + 4 || x + 4 > wx + 500;
            let c: [u8; 3] = if border { [0x90, 0x50, 0x20] } else { [0xFF, 0xFF, 0xFF] };
            v[i] = c[0]; v[i + 1] = c[1]; v[i + 2] = c[2];
        }
    }
    v
}

/// PSNR (dB) entre el frame fuente BGRA y el decodificado RGBA.
fn psnr_bgra_vs_rgba(src_bgra: &[u8], dec_rgba: &[u8]) -> f64 {
    assert_eq!(src_bgra.len(), dec_rgba.len());
    let mut se = 0f64;
    let n = src_bgra.len() / 4;
    for i in 0..n {
        let s = &src_bgra[i * 4..i * 4 + 4];
        let d = &dec_rgba[i * 4..i * 4 + 4];
        for (a, b) in [(s[2], d[0]), (s[1], d[1]), (s[0], d[2])] {
            let e = a as f64 - b as f64;
            se += e * e;
        }
    }
    let mse = se / (n as f64 * 3.0);
    if mse <= 0.0 { return 99.0; }
    10.0 * (255.0f64 * 255.0 / mse).log10()
}

#[derive(Default)]
struct Stats {
    bytes_total: usize,
    skipped: usize,
    decoded: usize,
    psnr_sum: f64,
    tail_n: usize,
    tail_psnr_sum: f64,
}

/// Codifica FRAMES frames con `encode` y los decodifica, acumulando métricas.
fn measure(mut encode: impl FnMut(&[u8]) -> Vec<u8>) -> Stats {
    let mut dec = H264Decoder::new().expect("decoder");
    let mut st = Stats::default();
    for t in 0..FRAMES {
        let src = desktop_frame(t);
        let au = encode(&src);
        if au.is_empty() {
            st.skipped += 1;
            continue;
        }
        st.bytes_total += au.len();
        if let Ok(Some((dw, dh, rgba))) = dec.decode(&au) {
            assert_eq!((dw, dh), (W, H), "la resolución decodificada debe ser la nativa");
            let p = psnr_bgra_vs_rgba(&src, rgba);
            st.decoded += 1;
            st.psnr_sum += p;
            if t >= STATIC_FROM + 5 {
                st.tail_n += 1;
                st.tail_psnr_sum += p; // calidad percibida con la pantalla quieta
            }
        }
    }
    st
}

fn report(label: &str, st: &Stats) {
    let kbps = (st.bytes_total as f64 * 8.0 * FPS as f64) / (FRAMES as f64 * 1000.0);
    println!(
        "{label:<38} {kbps:>6.0} kbps · saltados {:>2}/{FRAMES} · PSNR medio {:>5.1} dB · estático {:>5.1} dB",
        st.skipped,
        if st.decoded > 0 { st.psnr_sum / st.decoded as f64 } else { 0.0 },
        if st.tail_n > 0 { st.tail_psnr_sum / st.tail_n as f64 } else { 0.0 },
    );
}

/// Codificador crudo del crate con una config arbitraria + conversión BGRA.
fn raw_measure(config: EncoderConfig) -> Stats {
    let mut enc = RawEncoder::with_api_config(OpenH264API::from_source(), config).expect("encoder");
    let mut buf = I420::new(W & !1, H & !1);
    measure(|bgra| {
        buf.fill_from_bgra(bgra, W * 4);
        enc.encode(&buf).expect("encode").to_vec()
    })
}

fn screen_cfg() -> EncoderConfig {
    EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .rate_control_mode(RateControlMode::Bitrate)
        .bitrate(BitRate::from_bps((W * H * FPS as usize / 8) as u32))
        .max_frame_rate(FrameRate::from_hz(FPS as f32))
}

#[test]
#[ignore = "medición manual; correr con --ignored --nocapture"]
fn encoder_config_matrix() {
    println!("--- 1920x1080 @ {FPS} fps · {FRAMES} frames · escena quieta desde el {STATIC_FROM} ---");

    report("A. VIEJA (default del crate)", &raw_measure(EncoderConfig::new()));
    report("B. pantalla+bitrate (qp libre)", &raw_measure(screen_cfg()));
    report("C. pantalla+bitrate qp<=30", &raw_measure(screen_cfg().qp(QpRange::new(10, 30))));
    report("D. pantalla+bitrate qp<=26", &raw_measure(screen_cfg().qp(QpRange::new(10, 26))));
    report("E. pantalla+QUALITY qp<=30", &raw_measure(
        screen_cfg().rate_control_mode(RateControlMode::Quality).qp(QpRange::new(10, 30)),
    ));

    // La config REAL de producción (encode.rs), para confirmar qué sale al aire.
    let mut prod = H264Encoder::new(W, H, FPS).expect("encoder prod");
    report("P. PRODUCCIÓN (encode.rs actual)", &measure(|bgra| {
        prod.encode_bgra(bgra, W * 4).expect("encode prod")
    }));
}

/// Frame de "vídeo a pantalla completa": bloques pseudoaleatorios que cambian
/// cada frame. Peor caso de complejidad: valida que acotar el QP no dispare el
/// bitrate más allá del presupuesto ni provoque saltos masivos de frames.
fn noise_frame(t: usize) -> Vec<u8> {
    let mut v = vec![0u8; W * H * 4];
    let mut rng = Rng(0xBADC0DE ^ (t as u64) << 32);
    const B: usize = 16;
    for by in 0..H.div_ceil(B) {
        for bx in 0..W.div_ceil(B) {
            let r = rng.next();
            let (cb, cg, cr) = ((r & 0xFF) as u8, ((r >> 8) & 0xFF) as u8, ((r >> 16) & 0xFF) as u8);
            for y in (by * B)..((by + 1) * B).min(H) {
                for x in (bx * B)..((bx + 1) * B).min(W) {
                    let i = (y * W + x) * 4;
                    v[i] = cb; v[i + 1] = cg; v[i + 2] = cr; v[i + 3] = 0xFF;
                }
            }
        }
    }
    v
}

/// Mide una config contra el peor caso (todo el frame cambia siempre).
fn measure_noise(mut encode: impl FnMut(&[u8]) -> Vec<u8>) -> (f64, usize) {
    let mut bytes = 0usize;
    let mut skipped = 0usize;
    for t in 0..FRAMES {
        let src = noise_frame(t);
        let au = encode(&src);
        if au.is_empty() { skipped += 1; } else { bytes += au.len(); }
    }
    ((bytes as f64 * 8.0 * FPS as f64) / (FRAMES as f64 * 1000.0), skipped)
}

#[test]
#[ignore = "medición manual; correr con --ignored --nocapture"]
fn encoder_worst_case_motion() {
    println!("--- peor caso: 1080p todo cambiando cada frame ---");
    for (label, qp) in [("qp libre", QpRange::new(0, 51)), ("qp<=30", QpRange::new(10, 30)), ("qp<=26", QpRange::new(10, 26))] {
        let mut enc = RawEncoder::with_api_config(OpenH264API::from_source(), screen_cfg().qp(qp)).expect("encoder");
        let mut buf = I420::new(W & !1, H & !1);
        let (kbps, skipped) = measure_noise(|bgra| {
            buf.fill_from_bgra(bgra, W * 4);
            enc.encode(&buf).expect("encode").to_vec()
        });
        println!("{label:<10} {kbps:>7.0} kbps · saltados {skipped}/{FRAMES}");
    }
}
