//! Supplicant WPA2-PSK/CCMP en `no_std`, sin E/S.
//!
//! El kernel le da tramas Ethernet y le pide la respuesta; todo lo que decide
//! —derivar la PTK, validar MIC y contador de reenvío, descifrar Key Data,
//! elegir qué clave instalar— ocurre aquí, donde el banco del host lo ejecuta
//! contra transcripciones y vectores publicados.
//!
//! Estados: `Idle` → `Msg2Sent` → `Authorized`. Asociada **no** es autorizada:
//! hasta `Authorized` no hay tráfico IP que valga.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
extern crate std;

pub mod crypto;
pub mod eapol;
pub mod kde;

#[cfg(test)]
mod tests;

pub use crypto::{Ptk, pbkdf2_psk};
pub use eapol::{EapolKey, ParseError, key_info};
pub use kde::Gtk;

/// Bytes máximos de un EAPOL-Key aceptado (cabecera + Key Data).
pub const MAX_EAPOL: usize = 512;
/// Bytes máximos del RSN IE que se repite en M2.
pub const MAX_RSN_IE: usize = 64;
const MAX_TX: usize = 14 + eapol::EAPOL_KEY_LEN + MAX_RSN_IE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Asociada, sin PTK.
    Idle,
    /// M2 enviado; esperando M3.
    Msg2Sent,
    /// 4-way terminado y claves entregadas al llamante.
    Authorized,
}

/// Por qué se descartó una trama. Sirve para el log: un descarte silencioso
/// es lo que dejaba el handshake colgado sin decir dónde.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discard {
    /// No es EAPOL o no es EAPOL-Key válido.
    NotEapol(ParseError),
    /// EAPOL-Key mayor que `MAX_EAPOL`.
    TooBig,
    /// Sin el bit ACK: la escribió otro suplicante, no el AP.
    NotFromAp,
    /// Descriptor distinto de 2 (HMAC-SHA1 + AES): no es WPA2/CCMP.
    BadVersion,
    /// Contador de reenvío repetido o retrocedido.
    Replay,
    /// MIC incorrecto.
    BadMic,
    /// M3 con una ANonce distinta de la de M1.
    NonceMismatch,
    /// Key Data cifrada que no se pudo desenvolver con la KEK.
    KeyDataUnwrap,
    /// M3 sin KDE de GTK utilizable.
    NoGtk,
    /// Mensaje fuera de la secuencia esperada para el estado actual.
    Unexpected,
    /// No cabía la respuesta.
    NoSpace,
}

/// Resultado de procesar una trama.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Outcome {
    /// Hay una respuesta en `Supplicant::tx()` para enviar ya.
    pub send: bool,
    /// Clave temporal por pares que debe instalarse (CCMP, key id 0).
    pub install_tk: Option<[u8; 16]>,
    /// Clave de grupo con su índice y su contador de recepción.
    pub install_gtk: Option<GroupKey>,
    /// El 4-way ha terminado: a partir de aquí sí hay enlace autorizado.
    pub authorized: bool,
    /// Trama descartada, con el motivo.
    pub dropped: Option<Discard>,
}

/// GTK lista para el firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupKey {
    pub key: [u8; 16],
    pub key_id: u8,
    /// Key RSC del mensaje que la trajo: el PN desde el que valida el receptor.
    pub rsc: [u8; 8],
}

pub struct Supplicant {
    pmk: [u8; 32],
    sta: [u8; 6],
    bssid: [u8; 6],
    rsn_ie: [u8; MAX_RSN_IE],
    rsn_ie_len: usize,
    snonce: [u8; 32],
    anonce: [u8; 32],
    ptk: Ptk,
    have_ptk: bool,
    replay: u64,
    have_replay: bool,
    state: State,
    tx: [u8; MAX_TX],
    tx_len: usize,
    scratch: [u8; MAX_EAPOL],
}

impl Supplicant {
    /// `snonce` lo aporta el llamante: aquí no hay fuente de aleatoriedad.
    pub fn new(
        pmk: [u8; 32],
        sta: [u8; 6],
        bssid: [u8; 6],
        rsn_ie: &[u8],
        snonce: [u8; 32],
    ) -> Self {
        let mut ie = [0u8; MAX_RSN_IE];
        let n = rsn_ie.len().min(MAX_RSN_IE);
        ie[..n].copy_from_slice(&rsn_ie[..n]);
        Self {
            pmk,
            sta,
            bssid,
            rsn_ie: ie,
            rsn_ie_len: n,
            snonce,
            anonce: [0u8; 32],
            ptk: Ptk::default(),
            have_ptk: false,
            replay: 0,
            have_replay: false,
            state: State::Idle,
            tx: [0u8; MAX_TX],
            tx_len: 0,
            scratch: [0u8; MAX_EAPOL],
        }
    }

    pub fn state(&self) -> State {
        self.state
    }
    pub fn authorized(&self) -> bool {
        self.state == State::Authorized
    }
    /// Trama Ethernet lista para transmitir tras un `Outcome::send`.
    pub fn tx(&self) -> &[u8] {
        &self.tx[..self.tx_len]
    }
    /// PTK vigente; sólo para pruebas y diagnóstico.
    pub fn ptk(&self) -> Option<&Ptk> {
        self.have_ptk.then_some(&self.ptk)
    }

    /// Procesa una trama Ethernet completa (14 B de cabecera + carga).
    ///
    /// Devuelve `Outcome::default()` con `dropped = None` para lo que no es
    /// EAPOL: el llamante debe entregar esas tramas a la pila IP.
    pub fn on_ethernet(&mut self, frame: &[u8]) -> Outcome {
        if frame.len() < 14 {
            return Outcome::default();
        }
        if u16::from_be_bytes([frame[12], frame[13]]) != eapol::ETH_P_EAPOL {
            return Outcome::default();
        }
        self.on_eapol(&frame[14..])
    }

    /// Igual que `on_ethernet` pero sobre la carga EAPOL ya desencapsulada.
    pub fn on_eapol(&mut self, payload: &[u8]) -> Outcome {
        self.tx_len = 0;
        if payload.len() > MAX_EAPOL {
            return drop_with(Discard::TooBig);
        }
        let key = match EapolKey::parse(payload) {
            Ok(k) => k,
            Err(e) => return drop_with(Discard::NotEapol(e)),
        };
        // Los mensajes del autenticador llevan ACK. Descartarlos por eso era
        // tirar todo M1 y quedarse esperando para siempre.
        if !key.has(key_info::ACK) {
            return drop_with(Discard::NotFromAp);
        }
        if key.descriptor_version() != key_info::VERSION_HMAC_SHA1_AES {
            return drop_with(Discard::BadVersion);
        }
        let replay = u64::from_be_bytes(key.replay());
        if key.pairwise() {
            if key.has(key_info::MIC) {
                self.handle_m3(&key, replay)
            } else {
                self.handle_m1(&key, replay)
            }
        } else {
            self.handle_group(&key, replay)
        }
    }

    /// M1: ANonce del AP. Deriva la PTK y contesta M2 con el RSN IE negociado.
    fn handle_m1(&mut self, key: &EapolKey<'_>, replay: u64) -> Outcome {
        // Una retransmisión de M1 repite el contador; sólo se rechaza si
        // retrocede respecto a lo ya procesado.
        if self.have_replay && replay < self.replay {
            return drop_with(Discard::Replay);
        }
        self.anonce = key.nonce();
        self.ptk = crypto::derive_ptk(&self.pmk, &self.bssid, &self.sta, &self.anonce, &self.snonce);
        self.have_ptk = true;
        self.replay = replay;
        self.have_replay = true;

        let flags = key_info::KEY_TYPE | key_info::MIC;
        let rsn = self.rsn_ie;
        let rsn_len = self.rsn_ie_len;
        let snonce = self.snonce;
        let n = self.build_reply(key, flags, &snonce, &rsn[..rsn_len]);
        if n == 0 {
            return drop_with(Discard::NoSpace);
        }
        self.state = State::Msg2Sent;
        Outcome {
            send: true,
            ..Outcome::default()
        }
    }

    /// M3: valida MIC, ANonce y contador; extrae la GTK y contesta M4.
    fn handle_m3(&mut self, key: &EapolKey<'_>, replay: u64) -> Outcome {
        if !self.have_ptk {
            return drop_with(Discard::Unexpected);
        }
        // El contador no puede retroceder. Se admite la igualdad porque un M3
        // retransmitido (M4 perdido) repite el suyo y reprocesarlo es idempotente.
        if self.have_replay && replay < self.replay {
            return drop_with(Discard::Replay);
        }
        if !eapol::verify_mic(self.ptk.kck(), key, &mut self.scratch) {
            return drop_with(Discard::BadMic);
        }
        if key.nonce() != self.anonce {
            return drop_with(Discard::NonceMismatch);
        }
        let mut kd = [0u8; MAX_EAPOL];
        let kd_len = match self.key_data(key, &mut kd) {
            Ok(n) => n,
            Err(d) => return drop_with(d),
        };
        // En RSN el M3 siempre trae la GTK. Sin ella no se autoriza el enlace:
        // ignorarla dejaba la estación sin poder recibir difusión ni ARP.
        let Some(g) = kde::find_gtk(&kd[..kd_len]) else {
            return drop_with(Discard::NoGtk);
        };

        let flags = key_info::KEY_TYPE | key_info::MIC | key_info::SECURE;
        let n = self.build_reply(key, flags, &[0u8; 32], &[]);
        if n == 0 {
            return drop_with(Discard::NoSpace);
        }
        self.replay = replay;
        self.state = State::Authorized;
        Outcome {
            send: true,
            install_tk: Some(*self.ptk.tk()),
            install_gtk: Some(GroupKey {
                key: g.key,
                key_id: g.key_id,
                rsc: key.rsc(),
            }),
            authorized: true,
            dropped: None,
        }
    }

    /// Renovación de clave de grupo (mensaje 1 de 2 del group key handshake).
    fn handle_group(&mut self, key: &EapolKey<'_>, replay: u64) -> Outcome {
        if !self.have_ptk {
            return drop_with(Discard::Unexpected);
        }
        if self.have_replay && replay < self.replay {
            return drop_with(Discard::Replay);
        }
        if !eapol::verify_mic(self.ptk.kck(), key, &mut self.scratch) {
            return drop_with(Discard::BadMic);
        }
        let mut kd = [0u8; MAX_EAPOL];
        let kd_len = match self.key_data(key, &mut kd) {
            Ok(n) => n,
            Err(d) => return drop_with(d),
        };
        let Some(g) = kde::find_gtk(&kd[..kd_len]) else {
            return drop_with(Discard::NoGtk);
        };
        let flags = key_info::MIC | key_info::SECURE;
        let n = self.build_reply(key, flags, &[0u8; 32], &[]);
        if n == 0 {
            return drop_with(Discard::NoSpace);
        }
        self.replay = replay;
        Outcome {
            send: true,
            install_gtk: Some(GroupKey {
                key: g.key,
                key_id: g.key_id,
                rsc: key.rsc(),
            }),
            ..Outcome::default()
        }
    }

    /// Key Data en claro: se desenvuelve con AES-KEK si viene cifrada.
    fn key_data(&self, key: &EapolKey<'_>, out: &mut [u8]) -> Result<usize, Discard> {
        let raw = key.key_data();
        if !key.has(key_info::ENCR_KEY_DATA) {
            let n = raw.len().min(out.len());
            out[..n].copy_from_slice(&raw[..n]);
            return Ok(n);
        }
        crypto::aes_unwrap(self.ptk.kek(), raw, out).map_err(|_| Discard::KeyDataUnwrap)
    }

    /// Construye la respuesta en `self.tx` (Ethernet + EAPOL-Key + MIC).
    fn build_reply(
        &mut self,
        src: &EapolKey<'_>,
        flags: u16,
        nonce: &[u8; 32],
        key_data: &[u8],
    ) -> usize {
        let version = src.descriptor_version();
        let replay = src.replay();
        let eapol_version = src.bytes()[0];
        let total = 14 + eapol::EAPOL_KEY_LEN + key_data.len();
        if total > MAX_TX {
            return 0;
        }
        self.tx[..6].copy_from_slice(&self.bssid);
        self.tx[6..12].copy_from_slice(&self.sta);
        self.tx[12..14].copy_from_slice(&eapol::ETH_P_EAPOL.to_be_bytes());
        // key_length = 0 en RSN, tanto en M2 como en M4 y en el grupo.
        let n = eapol::build_reply(
            &mut self.tx[14..],
            eapol_version,
            version,
            flags,
            0,
            &replay,
            nonce,
            key_data,
        );
        if n == 0 {
            return 0;
        }
        eapol::set_mic(self.ptk.kck(), &mut self.tx[14..14 + n]);
        self.tx_len = 14 + n;
        self.tx_len
    }
}

fn drop_with(d: Discard) -> Outcome {
    Outcome {
        dropped: Some(d),
        ..Outcome::default()
    }
}
