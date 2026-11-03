//! Layout de `GPRIM` (GSCEN v2): lista de primitivas 2D para el backend GPU.
//!
//! Mismo formato de primitiva que `escena.sal` (16 palabras LE de 32 bits); el
//! backend CPU de Sal es el oráculo con el que se compara el de GPU. Es un
//! comando aparte de `GSCEN` (quads con textura): no rompe el wire format v1.

pub const TAG: &[u8; 5] = b"GPRIM";
pub const HEADER_LEN: usize = 32;
pub const PRIM_BYTES: usize = 64;
pub const MAX_PRIMS: usize = 24;
pub const LEN: usize = HEADER_LEN + MAX_PRIMS * PRIM_BYTES;

pub const FLAG_CLEAR_RT: u32 = 1;

pub mod kind {
    pub const NADA: u32 = 0;
    pub const SOLIDO: u32 = 1;
    pub const DEGRADADO: u32 = 2;
    pub const RRECT: u32 = 3;
    pub const ANILLO: u32 = 4;
    /// Rejilla de celdas de texto (G2/G4): ix0 columnas, iy0 filas, ix1/iy1 tamaño de celda,
    /// radios = bytes por glifo, borde_c = tamaño de la cabecera de la fuente PSF2, flags = primera
    /// celda en el búfer de celdas (3 palabras por celda: glifo|0xFFFFFFFF, tinta, fondo).
    pub const REJILLA: u32 = 5;
}

/// Desplazamientos (bytes) de los campos dentro de una primitiva.
pub mod off {
    pub const KIND: usize = 0;
    pub const X0: usize = 4;
    pub const Y0: usize = 8;
    pub const X1: usize = 12;
    pub const Y1: usize = 16;
    pub const C0: usize = 20;
    pub const C1: usize = 24;
    pub const ALFA: usize = 28;
    pub const RADIOS: usize = 32;
    pub const BORDE_C: usize = 36;
    pub const BORDE_W: usize = 40;
    pub const IX0: usize = 44;
    pub const IY0: usize = 48;
    pub const IX1: usize = 52;
    pub const IY1: usize = 56;
    pub const FLAGS: usize = 60;
}

pub type Prim = [u32; 16];

pub fn encode_header(rt_h: u64, fb_w: u32, fb_h: u32, pitch: u32, flags: u32, count: u16) -> [u8; HEADER_LEN] {
    assert!((count as usize) <= MAX_PRIMS);
    let mut h = [0u8; HEADER_LEN];
    h[..5].copy_from_slice(TAG);
    h[5..13].copy_from_slice(&rt_h.to_le_bytes());
    h[13..17].copy_from_slice(&fb_w.to_le_bytes());
    h[17..21].copy_from_slice(&fb_h.to_le_bytes());
    h[21..25].copy_from_slice(&pitch.to_le_bytes());
    h[25..29].copy_from_slice(&flags.to_le_bytes());
    h[29..31].copy_from_slice(&count.to_le_bytes());
    h
}

pub fn encode(rt_h: u64, fb_w: u32, fb_h: u32, pitch: u32, flags: u32, prims: &[Prim]) -> [u8; LEN] {
    assert!(prims.len() <= MAX_PRIMS);
    let mut cmd = [0u8; LEN];
    cmd[..HEADER_LEN].copy_from_slice(&encode_header(rt_h, fb_w, fb_h, pitch, flags, prims.len() as u16));
    for (i, p) in prims.iter().enumerate() {
        for (j, w) in p.iter().enumerate() {
            let o = HEADER_LEN + i * PRIM_BYTES + j * 4;
            cmd[o..o + 4].copy_from_slice(&w.to_le_bytes());
        }
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        assert_eq!(PRIM_BYTES, 64);
        assert_eq!(off::FLAGS + 4, PRIM_BYTES);
        assert_eq!(LEN, 32 + 24 * 64);
        assert_eq!(kind::REJILLA, 5);
    }

    #[test]
    fn header_y_dos_primitivas() {
        let mut a: Prim = [0; 16];
        a[off::KIND / 4] = kind::RRECT;
        a[off::X0 / 4] = (-7i32) as u32;
        a[off::RADIOS / 4] = 6 | 6 << 8;
        let mut b: Prim = [0; 16];
        b[off::KIND / 4] = kind::SOLIDO;
        b[off::ALFA / 4] = 90;
        let cmd = encode(7, 640, 480, 2560, FLAG_CLEAR_RT, &[a, b]);
        assert_eq!(&cmd[..5], b"GPRIM");
        assert_eq!(u64::from_le_bytes(cmd[5..13].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(cmd[25..29].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(cmd[29..31].try_into().unwrap()), 2);
        let p0 = HEADER_LEN;
        assert_eq!(u32::from_le_bytes(cmd[p0..p0 + 4].try_into().unwrap()), kind::RRECT);
        assert_eq!(i32::from_le_bytes(cmd[p0 + 4..p0 + 8].try_into().unwrap()), -7);
        let p1 = HEADER_LEN + PRIM_BYTES;
        assert_eq!(u32::from_le_bytes(cmd[p1 + off::ALFA..p1 + off::ALFA + 4].try_into().unwrap()), 90);
    }
}
