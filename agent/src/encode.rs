//! Conversión de frames BGRA (de la captura) a I420 y codificación H.264 con OpenH264.

use anyhow::Result;
use openh264::encoder::Encoder;
use openh264::formats::YUVSource;

/// Buffer I420 (YUV 4:2:0 planar) reutilizable entre frames.
pub struct I420 {
    w: usize,
    h: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl I420 {
    pub fn new(w: usize, h: usize) -> Self {
        let cw = w / 2;
        let ch = h / 2;
        Self {
            w,
            h,
            y: vec![0; w * h],
            u: vec![0; cw * ch],
            v: vec![0; cw * ch],
        }
    }

    /// Rellena el I420 a partir de un frame BGRA con `stride` bytes por fila
    /// (BT.601, rango limitado). Sólo procesa `self.w x self.h` píxeles.
    pub fn fill_from_bgra(&mut self, bgra: &[u8], stride: usize) {
        let (w, h) = (self.w, self.h);
        let cw = w / 2;

        // Plano Y: un valor por píxel.
        for row in 0..h {
            let src = row * stride;
            let dst = row * w;
            for col in 0..w {
                let i = src + col * 4;
                let b = bgra[i] as i32;
                let g = bgra[i + 1] as i32;
                let r = bgra[i + 2] as i32;
                self.y[dst + col] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
            }
        }

        // Planos U/V: promedio de bloques 2x2.
        for cy in 0..(h / 2) {
            for cx in 0..cw {
                let x0 = cx * 2;
                let y0 = cy * 2;
                let mut rs = 0i32;
                let mut gs = 0i32;
                let mut bs = 0i32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let i = (y0 + dy) * stride + (x0 + dx) * 4;
                        bs += bgra[i] as i32;
                        gs += bgra[i + 1] as i32;
                        rs += bgra[i + 2] as i32;
                    }
                }
                let r = rs / 4;
                let g = gs / 4;
                let b = bs / 4;
                let idx = cy * cw + cx;
                self.u[idx] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128) as u8;
                self.v[idx] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128) as u8;
            }
        }
    }
}

impl YUVSource for I420 {
    fn dimensions(&self) -> (usize, usize) {
        (self.w, self.h)
    }
    fn strides(&self) -> (usize, usize, usize) {
        (self.w, self.w / 2, self.w / 2)
    }
    fn y(&self) -> &[u8] {
        &self.y
    }
    fn u(&self) -> &[u8] {
        &self.u
    }
    fn v(&self) -> &[u8] {
        &self.v
    }
}

/// Codificador H.264 que toma frames BGRA y emite NAL units Annex-B.
pub struct H264Encoder {
    enc: Encoder,
    buf: I420,
}

impl H264Encoder {
    pub fn new(width: usize, height: usize) -> Result<Self> {
        // Dimensiones pares (requisito de 4:2:0).
        let w = width & !1;
        let h = height & !1;
        let enc = Encoder::new()?;
        Ok(Self {
            enc,
            buf: I420::new(w, h),
        })
    }

    pub fn dims(&self) -> (usize, usize) {
        (self.buf.w, self.buf.h)
    }

    /// Codifica un frame BGRA y devuelve el bitstream (vacío si no hubo salida).
    pub fn encode_bgra(&mut self, bgra: &[u8], stride: usize) -> Result<Vec<u8>> {
        self.buf.fill_from_bgra(bgra, stride);
        let bitstream = self.enc.encode(&self.buf)?;
        Ok(bitstream.to_vec())
    }
}
