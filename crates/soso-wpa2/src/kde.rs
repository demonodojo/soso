//! Key Data Elements del 4-way handshake (IEEE 802.11-2016 §12.7.2).
//!
//! Un KDE es `0xDD len OUI[3] data_type data…`, donde `len` cuenta desde el
//! OUI. El KDE de la GTK añade **dos** bytes propios antes de la clave:
//!
//! ```text
//! dd <len> 00 0f ac 01 <keyid|tx> <reservado> <GTK…>
//!  0   1   2  3  4  5      6           7        8
//! ```
//!
//! Empezar la clave en el offset 7 se lleva el byte reservado por delante y
//! pierde el último byte real de la GTK.

pub const KDE_TYPE: u8 = 0xdd;
pub const OUI_RSN: [u8; 3] = [0x00, 0x0f, 0xac];
pub const KDE_DATA_TYPE_GTK: u8 = 1;
pub const KDE_HDR_LEN: usize = 6;

/// GTK extraída de Key Data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gtk {
    pub key: [u8; 16],
    /// Índice 802.11 (1 o 2 en la práctica); va al firmware como Key ID.
    pub key_id: u8,
    /// Bit Tx del KDE.
    pub tx: bool,
}

/// Recorre Key Data emparejando `(data_type, payload)` de cada KDE RSN.
///
/// Los elementos que no son KDE RSN (por ejemplo el RSN IE, id 0x30) se saltan
/// respetando su longitud; un elemento que declare más bytes de los que quedan
/// corta el recorrido en vez de leer fuera.
pub fn for_each_rsn_kde<F: FnMut(u8, &[u8])>(key_data: &[u8], mut f: F) {
    let mut i = 0usize;
    while i + 2 <= key_data.len() {
        let id = key_data[i];
        let elen = key_data[i + 1] as usize;
        if elen == 0 || i + 2 + elen > key_data.len() {
            break;
        }
        let body = &key_data[i + 2..i + 2 + elen];
        if id == KDE_TYPE && elen >= 4 && body[..3] == OUI_RSN {
            f(body[3], &body[4..]);
        }
        i += 2 + elen;
    }
}

/// Busca el KDE de la GTK. `None` si no está o si la clave no mide 16 bytes.
pub fn find_gtk(key_data: &[u8]) -> Option<Gtk> {
    let mut found = None;
    for_each_rsn_kde(key_data, |data_type, payload| {
        if data_type != KDE_DATA_TYPE_GTK || found.is_some() {
            return;
        }
        // payload = keyid|tx (1) ‖ reservado (1) ‖ GTK
        if payload.len() < 2 + 16 {
            return;
        }
        let mut key = [0u8; 16];
        key.copy_from_slice(&payload[2..2 + 16]);
        found = Some(Gtk {
            key,
            key_id: payload[0] & 0x03,
            tx: payload[0] & 0x04 != 0,
        });
    });
    found
}
