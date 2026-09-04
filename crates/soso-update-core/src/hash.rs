//! SHA-256 en no_std.

use sha2::{Digest, Sha256};

pub type Hash256 = [u8; 32];

pub fn sha256(data: &[u8]) -> Hash256 {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().into()
}

/// Hex minúsculas de 64 caracteres.
pub fn hex_sha256(data: &[u8]) -> alloc::string::String {
    let h = sha256(data);
    hex::encode(h)
}

mod hex {
    use alloc::string::String;

    pub fn encode(bytes: [u8; 32]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(64);
        for b in bytes {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 0xf) as usize] as char);
        }
        s
    }

    pub fn decode(s: &str) -> Option<[u8; 32]> {
        if s.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        let bs = s.as_bytes();
        for i in 0..32 {
            out[i] = (nibble(bs[i * 2])? << 4) | nibble(bs[i * 2 + 1])?;
        }
        Some(out)
    }

    fn nibble(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
}

pub use hex::decode as decode_hex_sha256;
