//! Layout de `GUPRC` en `SYS_GPU_SUBMIT` (subida parcial a textura VRAM).

pub const LEN: usize = 41;

pub fn encode(
    tex_h: u64,
    src_ptr: u64,
    pitch: u32,
    dx: u32,
    dy: u32,
    rw: u32,
    rh: u32,
) -> [u8; LEN] {
    let mut cmd = [0u8; LEN];
    cmd[..5].copy_from_slice(b"GUPRC");
    cmd[5..13].copy_from_slice(&tex_h.to_le_bytes());
    cmd[13..21].copy_from_slice(&src_ptr.to_le_bytes());
    cmd[21..25].copy_from_slice(&pitch.to_le_bytes());
    cmd[25..29].copy_from_slice(&dx.to_le_bytes());
    cmd[29..33].copy_from_slice(&dy.to_le_bytes());
    cmd[33..37].copy_from_slice(&rw.to_le_bytes());
    cmd[37..41].copy_from_slice(&rh.to_le_bytes());
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_offsets() {
        let cmd = encode(7, 0x1000, 2560, 4, 8, 100, 16);
        assert_eq!(&cmd[..5], b"GUPRC");
        assert_eq!(u64::from_le_bytes(cmd[5..13].try_into().unwrap()), 7);
        assert_eq!(u64::from_le_bytes(cmd[13..21].try_into().unwrap()), 0x1000);
        assert_eq!(u32::from_le_bytes(cmd[33..37].try_into().unwrap()), 100);
    }
}
