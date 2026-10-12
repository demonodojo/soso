//! Tramas EAPOL-Key (IEEE 802.1X-2004 + IEEE 802.11i).
//!
//! Disposición, desde el byte 0 de la trama EAPOL (tras la cabecera Ethernet):
//!
//! | Offset | Campo |
//! |---|---|
//! | 0 | versión 802.1X |
//! | 1 | tipo de paquete (3 = EAPOL-Key) |
//! | 2..4 | longitud del cuerpo (big-endian) |
//! | 4 | tipo de descriptor (2 = RSN) |
//! | 5..7 | Key Information (big-endian) |
//! | 7..9 | Key Length |
//! | 9..17 | Replay Counter |
//! | 17..49 | Key Nonce |
//! | 49..65 | EAPOL Key IV |
//! | 65..73 | Key RSC |
//! | 73..81 | reservado |
//! | 81..97 | Key MIC |
//! | 97..99 | Key Data Length |
//! | 99.. | Key Data |
//!
//! `parse()` valida **antes** de leer: una trama de 4 bytes no puede hacer
//! pánico leyendo Key Information.

pub const ETH_P_EAPOL: u16 = 0x888e;
pub const EAPOL_TYPE_KEY: u8 = 3;
pub const DESC_TYPE_RSN: u8 = 2;

/// Bytes de un EAPOL-Key sin Key Data.
pub const EAPOL_KEY_LEN: usize = 99;
/// Longitud del cuerpo (offset 2..4) de un EAPOL-Key sin Key Data.
pub const EAPOL_BODY_LEN: usize = EAPOL_KEY_LEN - 4;

pub const OFF_DESC_TYPE: usize = 4;
pub const OFF_KEY_INFO: usize = 5;
pub const OFF_KEY_LEN: usize = 7;
pub const OFF_REPLAY: usize = 9;
pub const OFF_NONCE: usize = 17;
pub const OFF_IV: usize = 49;
pub const OFF_RSC: usize = 65;
pub const OFF_MIC: usize = 81;
pub const OFF_KD_LEN: usize = 97;
pub const OFF_KEY_DATA: usize = 99;

/// Bits de Key Information (`wpa_common.h`).
pub mod key_info {
    pub const VERSION_MASK: u16 = 0x0007;
    /// HMAC-SHA1-128 para el MIC y AES key wrap para Key Data: WPA2/CCMP.
    pub const VERSION_HMAC_SHA1_AES: u16 = 2;
    /// 1 = por pares, 0 = de grupo.
    pub const KEY_TYPE: u16 = 0x0008;
    pub const KEY_INDEX_MASK: u16 = 0x0030;
    pub const KEY_INDEX_SHIFT: u16 = 4;
    /// En mensajes por pares: instalar. En los de grupo: Tx.
    pub const INSTALL: u16 = 0x0040;
    /// Lo pone el autenticador: los mensajes del AP **llevan ACK**.
    pub const ACK: u16 = 0x0080;
    pub const MIC: u16 = 0x0100;
    pub const SECURE: u16 = 0x0200;
    pub const ERROR: u16 = 0x0400;
    pub const REQUEST: u16 = 0x0800;
    pub const ENCR_KEY_DATA: u16 = 0x1000;
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ParseError {
    /// Menos de 99 bytes: no hay EAPOL-Key completo.
    TooShort,
    /// `packet type` distinto de 3.
    NotKeyFrame,
    /// Tipo de descriptor distinto de RSN.
    NotRsn,
    /// La longitud declarada no concuerda con lo recibido.
    BadLength,
}

/// Vista de solo lectura sobre un EAPOL-Key ya validado.
#[derive(Clone, Copy)]
pub struct EapolKey<'a> {
    raw: &'a [u8],
    kd_len: usize,
}

impl<'a> EapolKey<'a> {
    /// Valida longitudes y tipos. No mira ni MIC ni contador de reenvío.
    pub fn parse(frame: &'a [u8]) -> Result<Self, ParseError> {
        if frame.len() < EAPOL_KEY_LEN {
            return Err(ParseError::TooShort);
        }
        if frame[1] != EAPOL_TYPE_KEY {
            return Err(ParseError::NotKeyFrame);
        }
        if frame[OFF_DESC_TYPE] != DESC_TYPE_RSN {
            return Err(ParseError::NotRsn);
        }
        let body = u16::from_be_bytes([frame[2], frame[3]]) as usize;
        if body < EAPOL_BODY_LEN || 4 + body > frame.len() {
            return Err(ParseError::BadLength);
        }
        let kd_len = u16::from_be_bytes([frame[OFF_KD_LEN], frame[OFF_KD_LEN + 1]]) as usize;
        if EAPOL_BODY_LEN + kd_len > body {
            return Err(ParseError::BadLength);
        }
        Ok(Self {
            raw: &frame[..4 + body],
            kd_len,
        })
    }

    /// La trama completa tal y como entra en el cálculo del MIC.
    pub fn bytes(&self) -> &'a [u8] {
        self.raw
    }
    pub fn key_info(&self) -> u16 {
        u16::from_be_bytes([self.raw[OFF_KEY_INFO], self.raw[OFF_KEY_INFO + 1]])
    }
    pub fn key_length(&self) -> u16 {
        u16::from_be_bytes([self.raw[OFF_KEY_LEN], self.raw[OFF_KEY_LEN + 1]])
    }
    pub fn descriptor_version(&self) -> u16 {
        self.key_info() & key_info::VERSION_MASK
    }
    pub fn replay(&self) -> [u8; 8] {
        self.raw[OFF_REPLAY..OFF_REPLAY + 8].try_into().expect("8")
    }
    pub fn nonce(&self) -> [u8; 32] {
        self.raw[OFF_NONCE..OFF_NONCE + 32].try_into().expect("32")
    }
    pub fn rsc(&self) -> [u8; 8] {
        self.raw[OFF_RSC..OFF_RSC + 8].try_into().expect("8")
    }
    pub fn mic(&self) -> [u8; 16] {
        self.raw[OFF_MIC..OFF_MIC + 16].try_into().expect("16")
    }
    pub fn key_data(&self) -> &'a [u8] {
        &self.raw[OFF_KEY_DATA..OFF_KEY_DATA + self.kd_len]
    }
    pub fn has(&self, bit: u16) -> bool {
        self.key_info() & bit != 0
    }
    /// Mensaje por pares (M1–M4) frente a mensaje del handshake de grupo.
    pub fn pairwise(&self) -> bool {
        self.has(key_info::KEY_TYPE)
    }
    /// Índice de clave de los mensajes de grupo antiguos (WPA1); en WPA2 el
    /// índice real va en el KDE de la GTK.
    pub fn key_index(&self) -> u8 {
        ((self.key_info() & key_info::KEY_INDEX_MASK) >> key_info::KEY_INDEX_SHIFT) as u8
    }
}

/// Construye una respuesta del suplicante (M2, M4 o mensaje 2 de grupo).
///
/// `version` se copia del mensaje del AP: decide el algoritmo de MIC y el
/// envoltorio de Key Data, y no se puede elegir por cuenta propia.
/// Devuelve los bytes escritos en `out`, o 0 si no caben.
pub fn build_reply(
    out: &mut [u8],
    eapol_version: u8,
    version: u16,
    flags: u16,
    key_length: u16,
    replay: &[u8; 8],
    nonce: &[u8; 32],
    key_data: &[u8],
) -> usize {
    let total = EAPOL_KEY_LEN + key_data.len();
    if out.len() < total {
        return 0;
    }
    out[..total].fill(0);
    out[0] = eapol_version;
    out[1] = EAPOL_TYPE_KEY;
    let body = (EAPOL_BODY_LEN + key_data.len()) as u16;
    out[2..4].copy_from_slice(&body.to_be_bytes());
    out[OFF_DESC_TYPE] = DESC_TYPE_RSN;
    let ki = (version & key_info::VERSION_MASK) | flags;
    out[OFF_KEY_INFO..OFF_KEY_INFO + 2].copy_from_slice(&ki.to_be_bytes());
    out[OFF_KEY_LEN..OFF_KEY_LEN + 2].copy_from_slice(&key_length.to_be_bytes());
    out[OFF_REPLAY..OFF_REPLAY + 8].copy_from_slice(replay);
    out[OFF_NONCE..OFF_NONCE + 32].copy_from_slice(nonce);
    out[OFF_KD_LEN..OFF_KD_LEN + 2].copy_from_slice(&(key_data.len() as u16).to_be_bytes());
    out[OFF_KEY_DATA..total].copy_from_slice(key_data);
    total
}

/// Escribe el MIC sobre la trama ya completa (el campo va a cero antes).
pub fn set_mic(kck: &[u8], frame: &mut [u8]) {
    if frame.len() < EAPOL_KEY_LEN {
        return;
    }
    frame[OFF_MIC..OFF_MIC + 16].fill(0);
    let mic = crate::crypto::eapol_mic_sha1(kck, frame);
    frame[OFF_MIC..OFF_MIC + 16].copy_from_slice(&mic);
}

/// Verifica el MIC de una trama recibida sin modificar el original.
///
/// `scratch` debe medir al menos `key.bytes().len()`.
pub fn verify_mic(kck: &[u8], key: &EapolKey<'_>, scratch: &mut [u8]) -> bool {
    let raw = key.bytes();
    if scratch.len() < raw.len() {
        return false;
    }
    let buf = &mut scratch[..raw.len()];
    buf.copy_from_slice(raw);
    buf[OFF_MIC..OFF_MIC + 16].fill(0);
    let calc = crate::crypto::eapol_mic_sha1(kck, buf);
    crate::crypto::ct_eq(&calc, &key.mic())
}
