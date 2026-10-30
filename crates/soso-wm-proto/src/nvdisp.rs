//! Contrato N4/N5 NVDisplay (modo fijo + scanout FB), alineado con `nvdisp_head.h`.

pub const HEAD_WIDTH: u32 = 640;
pub const HEAD_HEIGHT: u32 = 480;
pub const HEAD_PITCH: u32 = HEAD_WIDTH * 4;
pub const HEAD_FB_VRAM: u32 = 0x0180_0000;

/// Recorta `w`/`h` al rectángulo head (misma lógica que `nvdisp_scanout.c`).
/// Bytes del tile dañado (N8: un CE blit por rect GSCEN).
pub fn damaged_tile_bytes(w: u32, h: u32) -> u64 {
    (w as u64).saturating_mul(h as u64).saturating_mul(4)
}

pub fn clip_rect(x: u32, y: u32, w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    if x >= HEAD_WIDTH || y >= HEAD_HEIGHT {
        return None;
    }
    let mut cw = w;
    let mut ch = h;
    if x + cw > HEAD_WIDTH {
        cw = HEAD_WIDTH - x;
    }
    if y + ch > HEAD_HEIGHT {
        ch = HEAD_HEIGHT - y;
    }
    if cw == 0 || ch == 0 {
        return None;
    }
    Some((x, y, cw, ch))
}

/// Auto: NVDisplay si está listo; si no, GOP (`scanout::effective_backend` sin política forzada).
pub fn auto_backend(nvdisp_capable: bool, gop_capable: bool) -> Option<&'static str> {
    match crate::scanout::effective_backend(
        crate::scanout::ScanoutPolicy::Auto,
        nvdisp_capable,
        gop_capable,
    ) {
        Some(crate::scanout::ScanoutBackend::Nvdisp) => Some("nvdisp"),
        Some(crate::scanout::ScanoutBackend::Gop) => Some("gop"),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpres;

    #[test]
    fn head_mode_matches_gnvd_mvp() {
        assert_eq!(HEAD_PITCH, 2560);
        assert!(HEAD_FB_VRAM >= 0x0100_0000);
    }

    #[test]
    fn damaged_tile_bytes_hostcheck() {
        let (_, _, w, h) = clip_rect(600, 400, 100, 50).unwrap();
        assert_eq!(damaged_tile_bytes(w, h), 40 * 50 * 4);
    }

    #[test]
    fn scanout_clip_hostcheck_case() {
        let (_, _, w, h) = clip_rect(600, 400, 100, 50).unwrap();
        assert_eq!(w, 40);
        assert_eq!(h, 50);
    }

    #[test]
    fn gnvd_encode_default_head() {
        let cmd = gpres::encode_gnvd(3, HEAD_WIDTH, HEAD_HEIGHT, HEAD_PITCH);
        assert_eq!(gpres::LEN, 25);
        assert_eq!(&cmd[..4], b"GNVD");
        assert_eq!(cmd[4], 0);
        assert_eq!(
            u32::from_le_bytes(cmd[21..25].try_into().unwrap()),
            HEAD_PITCH
        );
    }

    #[test]
    fn auto_backend_matches_kernel_auto() {
        assert_eq!(auto_backend(true, true), Some("nvdisp"));
        assert_eq!(auto_backend(false, true), Some("gop"));
        assert_eq!(auto_backend(false, false), None);
    }

    #[test]
    fn gop_policy_string_for_gfx_demo() {
        use crate::scanout::{ScanoutBackend, ScanoutPolicy, effective_backend};
        assert_eq!(
            effective_backend(ScanoutPolicy::Gop, true, true),
            Some(ScanoutBackend::Gop)
        );
    }
}
