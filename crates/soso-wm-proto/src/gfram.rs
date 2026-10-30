//! Layout de `GFRAM` en `SYS_GPU_SUBMIT` (kernel `gpu.rs`).

pub const LEN: usize = 61;

pub fn encode(
    rt_h: u64,
    tex_h: u64,
    src_ptr: u64,
    fb_w: u32,
    fb_h: u32,
    pitch: u32,
    dx: u32,
    dy: u32,
    rw: u32,
    rh: u32,
    rgba: u32,
) -> [u8; LEN] {
    let mut cmd = [0u8; LEN];
    cmd[..5].copy_from_slice(b"GFRAM");
    cmd[5..13].copy_from_slice(&rt_h.to_le_bytes());
    cmd[13..21].copy_from_slice(&tex_h.to_le_bytes());
    cmd[21..29].copy_from_slice(&src_ptr.to_le_bytes());
    cmd[29..33].copy_from_slice(&fb_w.to_le_bytes());
    cmd[33..37].copy_from_slice(&fb_h.to_le_bytes());
    cmd[37..41].copy_from_slice(&pitch.to_le_bytes());
    cmd[41..45].copy_from_slice(&dx.to_le_bytes());
    cmd[45..49].copy_from_slice(&dy.to_le_bytes());
    cmd[49..53].copy_from_slice(&rw.to_le_bytes());
    cmd[53..57].copy_from_slice(&rh.to_le_bytes());
    cmd[57..61].copy_from_slice(&rgba.to_le_bytes());
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_and_field_offsets() {
        let cmd = encode(0x11, 0x22, 0x33, 640, 480, 2560, 10, 20, 100, 50, 0xff302030);
        assert_eq!(&cmd[..5], b"GFRAM");
        assert_eq!(u64::from_le_bytes(cmd[5..13].try_into().unwrap()), 0x11);
        assert_eq!(u64::from_le_bytes(cmd[13..21].try_into().unwrap()), 0x22);
        assert_eq!(u64::from_le_bytes(cmd[21..29].try_into().unwrap()), 0x33);
        assert_eq!(u32::from_le_bytes(cmd[29..33].try_into().unwrap()), 640);
        assert_eq!(u32::from_le_bytes(cmd[33..37].try_into().unwrap()), 480);
        assert_eq!(u32::from_le_bytes(cmd[37..41].try_into().unwrap()), 2560);
        assert_eq!(u32::from_le_bytes(cmd[41..45].try_into().unwrap()), 10);
        assert_eq!(u32::from_le_bytes(cmd[45..49].try_into().unwrap()), 20);
        assert_eq!(u32::from_le_bytes(cmd[49..53].try_into().unwrap()), 100);
        assert_eq!(u32::from_le_bytes(cmd[53..57].try_into().unwrap()), 50);
        assert_eq!(
            u32::from_le_bytes(cmd[57..61].try_into().unwrap()),
            0xff302030
        );
        assert_eq!(cmd.len(), LEN);
    }
}
