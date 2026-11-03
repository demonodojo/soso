//! Layout de `GPRIM` (GSCEN v2): lista de primitivas 2D para el backend GPU.
//!
//! Mismo formato de primitiva que `escena.sal` (16 palabras LE de 32 bits); el
//! backend CPU de Sal es el oráculo con el que se compara el de GPU. Es un
//! comando aparte de `GSCEN` (quads con textura): no rompe el wire format v1.

pub const TAG: &[u8; 5] = b"GPRIM";
/// Cabecera: tag(5) rt(u64 @5) fb_w @13 fb_h @17 pitch(bytes) @21 flags @25 count(u16) @29
/// fmt(u8: 0 RGB, 1 BGR) @31, rect de present x @32 y @36 w @40 h @44. Detrás van las
/// primitivas, `u32 psf_len` + fuente PSF2 (relleno a 4) y `u32 ncells` + celdas de 12 B
/// (glifo|0xFFFFFFFF, tinta 0xRRGGBB, fondo 0xRRGGBB).
pub const HEADER_LEN: usize = 48;
pub const PRIM_BYTES: usize = 64;
pub const MAX_PRIMS: usize = 32;
pub const CELL_BYTES: usize = 12;
/// Tamaño de la parte fija (cabecera + primitivas) y de las dos cabeceras de blobs.
pub const LEN: usize = HEADER_LEN + MAX_PRIMS * PRIM_BYTES + 8;

/// No presentar: sólo renderiza en el RT (pruebas y lotes).
pub const FLAG_NO_PRESENT: u32 = 2;

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

pub struct Escena<'a> {
    pub rt_h: u64,
    pub fb_w: u32,
    pub fb_h: u32,
    pub pitch: u32,
    pub flags: u32,
    pub fmt: u8,
    pub present: (u32, u32, u32, u32),
    pub prims: &'a [Prim],
    pub psf: &'a [u8],
    pub cells: &'a [[u32; 3]],
}

pub fn encode(e: &Escena) -> Vec<u8> {
    assert!(!e.prims.is_empty() && e.prims.len() <= MAX_PRIMS);
    let mut c = Vec::with_capacity(HEADER_LEN + e.prims.len() * PRIM_BYTES + 8 + e.psf.len() + e.cells.len() * CELL_BYTES + 3);
    c.extend_from_slice(TAG);
    c.extend_from_slice(&e.rt_h.to_le_bytes());
    for v in [e.fb_w, e.fb_h, e.pitch, e.flags] {
        c.extend_from_slice(&v.to_le_bytes());
    }
    c.extend_from_slice(&(e.prims.len() as u16).to_le_bytes());
    c.push(e.fmt);
    for v in [e.present.0, e.present.1, e.present.2, e.present.3] {
        c.extend_from_slice(&v.to_le_bytes());
    }
    debug_assert_eq!(c.len(), HEADER_LEN);
    for p in e.prims {
        for w in p {
            c.extend_from_slice(&w.to_le_bytes());
        }
    }
    c.extend_from_slice(&(e.psf.len() as u32).to_le_bytes());
    c.extend_from_slice(e.psf);
    while c.len() % 4 != 0 {
        c.push(0);
    }
    c.extend_from_slice(&(e.cells.len() as u32).to_le_bytes());
    for cell in e.cells {
        for w in cell {
            c.extend_from_slice(&w.to_le_bytes());
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        assert_eq!(PRIM_BYTES, 64);
        assert_eq!(off::FLAGS + 4, PRIM_BYTES);
        assert_eq!(HEADER_LEN, 48);
        assert_eq!(kind::REJILLA, 5);
    }

    #[test]
    fn escena_completa() {
        let mut a: Prim = [0; 16];
        a[off::KIND / 4] = kind::RRECT;
        a[off::X0 / 4] = (-7i32) as u32;
        a[off::RADIOS / 4] = 6 | 6 << 8;
        let mut b: Prim = [0; 16];
        b[off::KIND / 4] = kind::REJILLA;
        b[off::ALFA / 4] = 90;
        let psf = [1u8, 2, 3, 4, 5];
        let cells = [[7u32, 0xffffff, 0x101010], [0xFFFF_FFFF, 1, 2]];
        let cmd = encode(&Escena {
            rt_h: 7,
            fb_w: 640,
            fb_h: 480,
            pitch: 2560,
            flags: FLAG_NO_PRESENT,
            fmt: 1,
            present: (1, 2, 3, 4),
            prims: &[a, b],
            psf: &psf,
            cells: &cells,
        });
        assert_eq!(&cmd[..5], b"GPRIM");
        let u32_at = |o: usize| u32::from_le_bytes(cmd[o..o + 4].try_into().unwrap());
        assert_eq!(u64::from_le_bytes(cmd[5..13].try_into().unwrap()), 7);
        assert_eq!((u32_at(13), u32_at(17), u32_at(21), u32_at(25)), (640, 480, 2560, 2));
        assert_eq!(u16::from_le_bytes(cmd[29..31].try_into().unwrap()), 2);
        assert_eq!(cmd[31], 1);
        assert_eq!((u32_at(32), u32_at(36), u32_at(40), u32_at(44)), (1, 2, 3, 4));
        assert_eq!(u32_at(HEADER_LEN), kind::RRECT);
        assert_eq!(u32_at(HEADER_LEN + 4) as i32, -7);
        let fin = HEADER_LEN + 2 * PRIM_BYTES;
        assert_eq!(u32_at(fin), 5); // psf_len
        assert_eq!(&cmd[fin + 4..fin + 9], &psf);
        let ch = fin + 4 + 8; // 5 bytes de fuente → relleno a 8
        assert_eq!(u32_at(ch), 2);
        assert_eq!(u32_at(ch + 4), 7);
        assert_eq!(u32_at(ch + 4 + 12), 0xFFFF_FFFF);
        assert_eq!(cmd.len(), ch + 4 + 24);
    }
}
