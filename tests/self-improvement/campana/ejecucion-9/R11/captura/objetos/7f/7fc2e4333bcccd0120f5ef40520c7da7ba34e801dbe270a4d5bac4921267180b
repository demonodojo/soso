//! Primitivas: PBKDF2-PSK, PRF-SHA1, derivación PTK, MIC y AES key unwrap.
//!
//! Todas son funciones puras sobre `[u8]`: el banco de pruebas del host las
//! ejecuta contra vectores publicados sin tocar nada del kernel.

use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// Longitudes de la PTK de WPA2-PSK/CCMP: KCK(16) ‖ KEK(16) ‖ TK(16).
pub const PTK_LEN: usize = 48;
pub const KCK_LEN: usize = 16;
pub const KEK_LEN: usize = 16;
pub const TK_LEN: usize = 16;

/// PMK de 32 B desde passphrase ASCII y SSID (IEEE 802.11i anexo J).
pub fn pbkdf2_psk(passphrase: &str, ssid: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha1>(passphrase.as_bytes(), ssid.as_bytes(), 4096, &mut out);
    out
}

/// PRF-SHA1 de IEEE 802.11i (`sha1_prf` de hostap).
///
/// Cada bloque es `HMAC-SHA1(key, label ‖ 0x00 ‖ data ‖ i)` con `i` un byte que
/// **empieza en cero y también va en el primer bloque**: omitirlo desplaza toda
/// la PTK y deja los últimos bytes sin generar.
pub fn prf_sha1(key: &[u8], label: &[u8], data: &[u8], out: &mut [u8]) {
    let mut counter: u8 = 0;
    let mut pos = 0usize;
    while pos < out.len() {
        let mut mac = HmacSha1::new_from_slice(key).expect("hmac acepta cualquier longitud");
        mac.update(label);
        mac.update(&[0u8]);
        mac.update(data);
        mac.update(&[counter]);
        let block = mac.finalize().into_bytes();
        let n = core::cmp::min(20, out.len() - pos);
        out[pos..pos + n].copy_from_slice(&block[..n]);
        pos += n;
        counter = counter.wrapping_add(1);
    }
}

/// PTK derivada de la PMK: 48 B = KCK ‖ KEK ‖ TK.
#[derive(Clone, Copy)]
pub struct Ptk(pub [u8; PTK_LEN]);

impl Ptk {
    pub fn kck(&self) -> &[u8] {
        &self.0[..KCK_LEN]
    }
    pub fn kek(&self) -> &[u8] {
        &self.0[KCK_LEN..KCK_LEN + KEK_LEN]
    }
    /// Clave temporal de datos. CCMP cifra con **esta**, no con la KCK.
    pub fn tk(&self) -> &[u8; TK_LEN] {
        self.0[KCK_LEN + KEK_LEN..]
            .try_into()
            .expect("48 - 32 = 16")
    }
}

impl Default for Ptk {
    fn default() -> Self {
        Ptk([0u8; PTK_LEN])
    }
}

/// `PRF-384(PMK, "Pairwise key expansion", min(AA,SPA) ‖ max ‖ min(nonce) ‖ max)`.
pub fn derive_ptk(
    pmk: &[u8; 32],
    aa: &[u8; 6],
    spa: &[u8; 6],
    anonce: &[u8; 32],
    snonce: &[u8; 32],
) -> Ptk {
    let mut data = [0u8; 12 + 64];
    let (lo, hi) = if aa <= spa { (aa, spa) } else { (spa, aa) };
    data[..6].copy_from_slice(lo);
    data[6..12].copy_from_slice(hi);
    let (nlo, nhi) = if anonce <= snonce {
        (anonce, snonce)
    } else {
        (snonce, anonce)
    };
    data[12..44].copy_from_slice(nlo);
    data[44..76].copy_from_slice(nhi);

    let mut ptk = Ptk::default();
    prf_sha1(pmk, b"Pairwise key expansion", &data, &mut ptk.0);
    ptk
}

/// MIC de un EAPOL-Key con descriptor versión 2 (HMAC-SHA1-128 truncado).
pub fn eapol_mic_sha1(kck: &[u8], frame: &[u8]) -> [u8; 16] {
    let mut mac = HmacSha1::new_from_slice(kck).expect("hmac");
    mac.update(frame);
    let t = mac.finalize().into_bytes();
    let mut out = [0u8; 16];
    out.copy_from_slice(&t[..16]);
    out
}

/// Comparación en tiempo constante: el MIC se compara contra datos del aire.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Error de `aes_unwrap`.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum UnwrapError {
    /// El texto cifrado no mide `8·(n+1)` con `n ≥ 2`, o no cabe en `out`.
    BadLength,
    /// El IV inicial no es `A6A6A6A6A6A6A6A6`: clave o datos incorrectos.
    Integrity,
}

/// AES Key Unwrap (RFC 3394), el envoltorio de Key Data de WPA2.
///
/// `wrapped` mide 8 bytes más que el texto claro. Devuelve los bytes escritos.
pub fn aes_unwrap(kek: &[u8], wrapped: &[u8], out: &mut [u8]) -> Result<usize, UnwrapError> {
    use aes::cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray};

    if wrapped.len() < 24 || wrapped.len() % 8 != 0 {
        return Err(UnwrapError::BadLength);
    }
    let n = wrapped.len() / 8 - 1;
    if out.len() < n * 8 {
        return Err(UnwrapError::BadLength);
    }
    let cipher = match kek.len() {
        16 => aes::Aes128::new_from_slice(kek).map_err(|_| UnwrapError::BadLength)?,
        _ => return Err(UnwrapError::BadLength),
    };

    let mut a = [0u8; 8];
    a.copy_from_slice(&wrapped[..8]);
    let r = &mut out[..n * 8];
    r.copy_from_slice(&wrapped[8..]);

    for j in (0..6).rev() {
        for i in (1..=n).rev() {
            let t = (n * j + i) as u64;
            let tb = t.to_be_bytes();
            let mut block = [0u8; 16];
            for k in 0..8 {
                block[k] = a[k] ^ tb[k];
            }
            block[8..].copy_from_slice(&r[(i - 1) * 8..i * 8]);
            cipher.decrypt_block(GenericArray::from_mut_slice(&mut block));
            a.copy_from_slice(&block[..8]);
            r[(i - 1) * 8..i * 8].copy_from_slice(&block[8..]);
        }
    }
    if !ct_eq(&a, &[0xa6u8; 8]) {
        return Err(UnwrapError::Integrity);
    }
    Ok(n * 8)
}
