//! Layout de `GPRES` / `GNVD` (presentación VRAM → GOP o NVDisplay).
//!
//! Tag de 5 bytes (como el resto de comandos GPU). `GNVD` rellena el quinto con `0`.

pub const LEN: usize = 25;

pub const TAG_GPRES: [u8; 5] = *b"GPRES";
pub const TAG_GNVD: [u8; 5] = *b"GNVD\0";

pub fn encode(tag: [u8; 5], rt_h: u64, w: u32, h: u32, pitch: u32) -> [u8; LEN] {
    let mut cmd = [0u8; LEN];
    cmd[..5].copy_from_slice(&tag);
    cmd[5..13].copy_from_slice(&rt_h.to_le_bytes());
    cmd[13..17].copy_from_slice(&w.to_le_bytes());
    cmd[17..21].copy_from_slice(&h.to_le_bytes());
    cmd[21..25].copy_from_slice(&pitch.to_le_bytes());
    cmd
}

pub fn encode_gpres(rt_h: u64, w: u32, h: u32, pitch: u32) -> [u8; LEN] {
    encode(TAG_GPRES, rt_h, w, h, pitch)
}

pub fn encode_gnvd(rt_h: u64, w: u32, h: u32, pitch: u32) -> [u8; LEN] {
    encode(TAG_GNVD, rt_h, w, h, pitch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpres_and_gnvd_layout() {
        let p = encode_gpres(7, 640, 480, 2560);
        assert_eq!(&p[..5], b"GPRES");
        assert_eq!(u64::from_le_bytes(p[5..13].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(p[13..17].try_into().unwrap()), 640);
        let n = encode_gnvd(9, 1920, 1080, 7680);
        assert_eq!(&n[..4], b"GNVD");
        assert_eq!(n[4], 0);
        assert_eq!(u64::from_le_bytes(n[5..13].try_into().unwrap()), 9);
        assert_eq!(LEN, 25);
    }
}
