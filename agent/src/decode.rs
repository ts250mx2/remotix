//! Decodificación H.264 (OpenH264) → RGBA, para el visor nativo (rol operador).
//!
//! Espejo de `encode.rs`: usa el MISMO crate `openh264` (que trae decoder además
//! del encoder) para no añadir dependencias. Recibe Access Units Annex-B
//! (reensamblados desde RTP) y produce frames RGBA listos para subir a una textura.

use anyhow::Result;
use openh264::decoder::Decoder;
use openh264::formats::YUVSource; // aporta dimensions() sobre DecodedYUV

pub struct H264Decoder {
    dec: Decoder,
    rgba: Vec<u8>,
    dims: (usize, usize),
}

impl H264Decoder {
    pub fn new() -> Result<Self> {
        Ok(Self { dec: Decoder::new()?, rgba: Vec::new(), dims: (0, 0) })
    }

    pub fn dims(&self) -> (usize, usize) {
        self.dims
    }

    /// Decodifica un Access Unit (Annex-B, puede incluir SPS/PPS+IDR). Devuelve
    /// `Some((w, h, rgba))` cuando sale un frame; `None` si el decoder necesita
    /// más datos (p. ej. al arrancar antes del primer keyframe).
    pub fn decode(&mut self, au: &[u8]) -> Result<Option<(usize, usize, &[u8])>> {
        match self.dec.decode(au)? {
            Some(yuv) => {
                let (w, h) = yuv.dimensions();
                let need = w * h * 4;
                if self.rgba.len() != need {
                    self.rgba.resize(need, 0);
                }
                yuv.write_rgba8(&mut self.rgba);
                self.dims = (w, h);
                Ok(Some((w, h, &self.rgba)))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::H264Encoder;

    // Genera un frame BGRA con un patrón que cambia con el tiempo `t`.
    fn synth_bgra(w: usize, h: usize, t: usize) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                v[i] = ((x + t * 4) % 256) as u8;       // B
                v[i + 1] = ((y + t * 2) % 256) as u8;   // G
                v[i + 2] = ((x + y + t) % 256) as u8;   // R
                v[i + 3] = 255;                          // A
            }
        }
        v
    }

    // Valida: (1) openh264 expone decoder, (2) decode() produce frames,
    // (3) dimensiones correctas, (4) el flushing no descarta casi todo.
    #[test]
    fn roundtrip_encode_decode() {
        let (w, h) = (320usize, 240usize);
        let mut enc = H264Encoder::new(w, h).unwrap();
        let mut dec = H264Decoder::new().unwrap();
        let frames = 30usize;
        let mut decoded = 0usize;
        for t in 0..frames {
            let bgra = synth_bgra(w, h, t);
            let au = enc.encode_bgra(&bgra, w * 4).unwrap();
            if au.is_empty() {
                continue;
            }
            if let Some((dw, dh, rgba)) = dec.decode(&au).unwrap() {
                assert_eq!((dw, dh), (w, h), "dimensiones decodificadas");
                assert_eq!(rgba.len(), w * h * 4, "buffer RGBA");
                decoded += 1;
            }
        }
        eprintln!("[dectest] {} frames codificados, {} decodificados", frames, decoded);
        assert!(decoded >= frames - 3, "el decoder produjo muy pocos frames: {}/{}", decoded, frames);
    }
}
