//! Layout de `GSCEN`: blit de quads al RT y present parcial.

pub const MAX_QUADS: usize = 19;
pub const QUAD_BYTES: usize = 36;
pub const HEADER_LEN: usize = 53;
pub const LEN: usize = HEADER_LEN + MAX_QUADS * QUAD_BYTES;

pub const FLAG_CLEAR_RT: u32 = 1;
/// El backend de present (GOP vs NVDisplay) lo elige el kernel (`scanout`); no hay
/// flag extra en `GSCEN`. Scanout forzado por HDMI usa el comando aparte `GNVD`.

#[derive(Clone, Copy, Debug, Default)]
pub struct Quad {
    pub tex_h: u64,
    pub dx: u32,
    pub dy: u32,
    pub w: u32,
    pub h: u32,
    pub tex_pitch: u32,
    pub sx: u32,
    pub sy: u32,
}

pub fn encode(
    rt_h: u64,
    fb_w: u32,
    fb_h: u32,
    pitch: u32,
    pres_x: u32,
    pres_y: u32,
    pres_w: u32,
    pres_h: u32,
    flags: u32,
    rgba_clear: u32,
    quads: &[Quad],
) -> [u8; LEN] {
    assert!(quads.len() <= MAX_QUADS);
    let mut cmd = [0u8; LEN];
    cmd[..5].copy_from_slice(b"GSCEN");
    cmd[5..13].copy_from_slice(&rt_h.to_le_bytes());
    cmd[13..17].copy_from_slice(&fb_w.to_le_bytes());
    cmd[17..21].copy_from_slice(&fb_h.to_le_bytes());
    cmd[21..25].copy_from_slice(&pitch.to_le_bytes());
    cmd[25..29].copy_from_slice(&pres_x.to_le_bytes());
    cmd[29..33].copy_from_slice(&pres_y.to_le_bytes());
    cmd[33..37].copy_from_slice(&pres_w.to_le_bytes());
    cmd[37..41].copy_from_slice(&pres_h.to_le_bytes());
    cmd[41..45].copy_from_slice(&flags.to_le_bytes());
    cmd[45..49].copy_from_slice(&rgba_clear.to_le_bytes());
    cmd[49..51].copy_from_slice(&(quads.len() as u16).to_le_bytes());
    for (i, q) in quads.iter().enumerate() {
        let off = HEADER_LEN + i * QUAD_BYTES;
        cmd[off..off + 8].copy_from_slice(&q.tex_h.to_le_bytes());
        cmd[off + 8..off + 12].copy_from_slice(&q.dx.to_le_bytes());
        cmd[off + 12..off + 16].copy_from_slice(&q.dy.to_le_bytes());
        cmd[off + 16..off + 20].copy_from_slice(&q.w.to_le_bytes());
        cmd[off + 20..off + 24].copy_from_slice(&q.h.to_le_bytes());
        cmd[off + 24..off + 28].copy_from_slice(&q.tex_pitch.to_le_bytes());
        cmd[off + 28..off + 32].copy_from_slice(&q.sx.to_le_bytes());
        cmd[off + 32..off + 36].copy_from_slice(&q.sy.to_le_bytes());
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_one_quad() {
        let q = Quad {
            tex_h: 9,
            dx: 10,
            dy: 20,
            w: 100,
            h: 50,
            tex_pitch: 400,
            sx: 4,
            sy: 8,
        };
        let cmd = encode(7, 640, 480, 2560, 0, 0, 640, 480, 0, 0xff302030, &[q]);
        assert_eq!(&cmd[..5], b"GSCEN");
        assert_eq!(u16::from_le_bytes(cmd[49..51].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(cmd[53..61].try_into().unwrap()), 9);
        assert_eq!(u32::from_le_bytes(cmd[53 + 28..53 + 32].try_into().unwrap()), 4);
        assert_eq!(u32::from_le_bytes(cmd[53 + 32..53 + 36].try_into().unwrap()), 8);
        assert_eq!(QUAD_BYTES, 36);
    }
}
