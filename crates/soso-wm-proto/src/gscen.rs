//! Layout de `GSCEN`: blit de quads al RT y present parcial.

pub const MAX_QUADS: usize = 19;
pub const QUAD_BYTES: usize = 36;
pub const HEADER_LEN: usize = 53;
pub const LEN: usize = HEADER_LEN + MAX_QUADS * QUAD_BYTES;

pub const FLAG_CLEAR_RT: u32 = 1;
/// Bit `FLAG_BLEND_BASE + i` de `flags`: el quad `i` se **mezcla** con alfa (byte 3
/// del píxel, sin premultiplicar) sobre lo que ya hay en el RT en vez de copiarse.
/// Un kernel que no lo conoce lo ignora y copia el quad opaco, como antes.
pub const FLAG_BLEND_BASE: u32 = 8;

pub fn flag_blend(i: usize) -> u32 {
    1u32 << (FLAG_BLEND_BASE as usize + i)
}

pub fn quad_blend(flags: u32, i: usize) -> bool {
    i < MAX_QUADS && flags & flag_blend(i) != 0
}

/// Mezcla una fila de `src` sobre `dst` (píxeles de 4 bytes, alfa en el byte 3).
/// Alfa 0 no toca el destino; el resultado queda opaco. Es el mismo cálculo que
/// `comp_gscen_cpu` (Sal), que hace de oráculo en QEMU.
pub fn mezclar_fila(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let a = u32::from(s[3]);
        if a == 0 {
            continue;
        }
        if a == 255 {
            d[..3].copy_from_slice(&s[..3]);
        } else {
            for c in 0..3 {
                d[c] = ((u32::from(s[c]) * a + u32::from(d[c]) * (255 - a) + 127) / 255) as u8;
            }
        }
        d[3] = 255;
    }
}

/// Tramo `[inicio, fin)` de páginas de 4 KiB que cubre el rectángulo `w×h` en
/// `(x, y)` de un búfer de 4 bytes por píxel con `pitch`. Las copias del CE entre
/// VRAM y memoria del sistema van por páginas enteras.
pub fn tramo_paginas(x: u32, y: u32, w: u32, h: u32, pitch: u32) -> Option<(u64, u64)> {
    if w == 0 || h == 0 || u64::from(w) * 4 > u64::from(pitch) {
        return None;
    }
    let ini = u64::from(y) * u64::from(pitch) + u64::from(x) * 4;
    let fin = (u64::from(y) + u64::from(h) - 1) * u64::from(pitch) + (u64::from(x) + u64::from(w)) * 4;
    Some((ini & !0xfff, fin.checked_add(0xfff)? & !0xfff))
}
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

    #[test]
    fn bits_de_mezcla_no_pisan_el_borrado() {
        assert_eq!(flag_blend(0), 1 << 8);
        assert_eq!(flag_blend(MAX_QUADS - 1) & FLAG_CLEAR_RT, 0);
        assert!(quad_blend(FLAG_CLEAR_RT | flag_blend(3), 3));
        assert!(!quad_blend(FLAG_CLEAR_RT | flag_blend(3), 2));
        assert!(!quad_blend(u32::MAX, MAX_QUADS));
    }

    #[test]
    fn mezcla_alfa() {
        let mut d = [10, 20, 30, 0, 100, 100, 100, 255, 200, 0, 50, 255];
        let s = [99, 99, 99, 0, 0, 255, 0, 255, 0, 0, 255, 128];
        mezclar_fila(&mut d, &s);
        assert_eq!(&d[..4], &[10, 20, 30, 0], "alfa 0 no toca");
        assert_eq!(&d[4..8], &[0, 255, 0, 255], "alfa 255 copia");
        // (0*128 + 200*127 + 127) / 255 = 100; (255*128 + 50*127 + 127) / 255 = 153
        assert_eq!(&d[8..12], &[100, 0, 153, 255]);
    }

    #[test]
    fn tramo_cubre_el_rectangulo_por_paginas() {
        // Puntero 12×16 en (100, 200) sobre 1280 px de ancho.
        let (a, b) = tramo_paginas(100, 200, 12, 16, 5120).unwrap();
        assert_eq!(a % 4096, 0);
        assert_eq!(b % 4096, 0);
        assert!(a <= 200 * 5120 + 400);
        assert!(b >= 215 * 5120 + 448);
        assert_eq!(tramo_paginas(0, 0, 12, 16, 48), Some((0, 4096)));
        assert_eq!(tramo_paginas(0, 0, 13, 1, 48), None);
    }
}
