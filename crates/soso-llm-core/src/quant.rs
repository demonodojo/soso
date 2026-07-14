//! Bloques de cuantización Q8_0 y Q4_K (layout autocontenido).

pub const Q8_BLOCK: usize = 256;

pub struct Q8Block {
    pub scale: f32,
    pub qs: [i8; Q8_BLOCK],
}

impl Q8Block {
    pub fn dequant(&self, out: &mut [f32]) {
        for (o, &q) in out.iter_mut().zip(self.qs.iter()) {
            *o = q as f32 * self.scale;
        }
    }
}

pub fn quantize_q8_0(src: &[f32]) -> Q8Block {
    let mut max = 0.0f32;
    for &v in src {
        max = max.max(v.abs());
    }
    let scale = if max > 0.0 { max / 127.0 } else { 1.0 };
    let mut qs = [0i8; Q8_BLOCK];
    for (q, &v) in qs.iter_mut().zip(src) {
        *q = libm::roundf(v / scale).clamp(-127.0, 127.0) as i8;
    }
    Q8Block { scale, qs }
}
