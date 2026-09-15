//! Banco del supplicant en host.
//!
//! Dos clases de prueba, y conviene no confundirlas:
//!
//! * **Vectores publicados**: PBKDF2 del anexo de IEEE 802.11i y AES Key Wrap
//!   del RFC 3394. Acreditan la primitiva por sí sola.
//! * **Contraste estructural y transcripciones**: la PRF se compara con una
//!   segunda implementación escrita aquí desde el texto de la norma, y el
//!   handshake completo se ejecuta contra un AP simulado que deriva su PTK por
//!   ese otro camino. Acredita que emisor y receptor coinciden y fija los
//!   defectos concretos que la investigación del 15/09/2026 reprodujo; no
//!   sustituye a una captura contra un AP real.

use super::*;
use crate::crypto::{aes_unwrap, derive_ptk, pbkdf2_psk, prf_sha1};
use crate::eapol::{EapolKey, build_reply, key_info, set_mic};
use aes::cipher::{BlockEncrypt, KeyInit as AesKeyInit, generic_array::GenericArray};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::vec::Vec;

const STA: [u8; 6] = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
const AP: [u8; 6] = [0x06, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];
/// El mismo RSN IE que `build_assoc_req()` mete en la Association Request.
const RSN_IE: [u8; 22] = [
    0x30, 0x14, 0x01, 0x00, 0x00, 0x0f, 0xac, 0x04, 0x01, 0x00, 0x00, 0x0f, 0xac, 0x04, 0x01, 0x00,
    0x00, 0x0f, 0xac, 0x02, 0x00, 0x00,
];

fn hex(s: &str) -> Vec<u8> {
    let b: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    b.chunks(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}

// --- vectores publicados ------------------------------------------------

#[test]
fn pbkdf2_vector_ieee() {
    // IEEE 802.11i, anexo de vectores de PSK.
    assert_eq!(
        pbkdf2_psk("password", "IEEE").to_vec(),
        hex("f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e")
    );
}

#[test]
fn aes_unwrap_rfc3394_128() {
    // RFC 3394 §4.1: 128 bits de clave con una KEK de 128 bits.
    let kek = hex("000102030405060708090A0B0C0D0E0F");
    let wrapped = hex("1FA68B0A8112B447AEF34BD8FB5A7B829D3E862371D2CFE5");
    let mut out = [0u8; 16];
    let n = aes_unwrap(&kek, &wrapped, &mut out).unwrap();
    assert_eq!(n, 16);
    assert_eq!(out.to_vec(), hex("00112233445566778899AABBCCDDEEFF"));
}

#[test]
fn aes_unwrap_detecta_kek_incorrecta() {
    let mut kek = hex("000102030405060708090A0B0C0D0E0F");
    kek[0] ^= 1;
    let wrapped = hex("1FA68B0A8112B447AEF34BD8FB5A7B829D3E862371D2CFE5");
    let mut out = [0u8; 16];
    assert_eq!(
        aes_unwrap(&kek, &wrapped, &mut out),
        Err(crypto::UnwrapError::Integrity)
    );
}

#[test]
fn aes_unwrap_rechaza_longitudes_imposibles() {
    let kek = hex("000102030405060708090A0B0C0D0E0F");
    let mut out = [0u8; 64];
    for len in [0usize, 7, 8, 16, 17, 23, 25] {
        let w = std::vec![0u8; len];
        assert_eq!(
            aes_unwrap(&kek, &w, &mut out),
            Err(crypto::UnwrapError::BadLength),
            "len={len}"
        );
    }
}

// --- PRF: contraste con una segunda implementación ----------------------

/// PRF-SHA1 escrita desde el texto de la norma: se construye el bloque
/// completo `label ‖ 0x00 ‖ data ‖ i` y se corta al final, en vez de ir
/// actualizando el HMAC por partes.
fn prf_referencia(key: &[u8], label: &[u8], data: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0u8;
    while out.len() < len {
        let mut msg = Vec::new();
        msg.extend_from_slice(label);
        msg.push(0);
        msg.extend_from_slice(data);
        msg.push(i);
        let mut m = <Hmac<Sha1> as Mac>::new_from_slice(key).unwrap();
        m.update(&msg);
        out.extend_from_slice(&m.finalize().into_bytes());
        i += 1;
    }
    out.truncate(len);
    out
}

#[test]
fn prf_coincide_con_la_referencia() {
    let key = [0x0bu8; 20];
    for len in [1usize, 16, 20, 21, 40, 48, 64] {
        let mut mine = std::vec![0u8; len];
        prf_sha1(&key, b"prefix", b"Hello World", &mut mine);
        assert_eq!(mine, prf_referencia(&key, b"prefix", b"Hello World", len), "len={len}");
    }
}

#[test]
fn prf_genera_la_longitud_pedida() {
    // El defecto anterior sólo escribía 40 de 48 bytes: los 8 últimos, que son
    // la mitad alta de la TK, se quedaban a cero.
    let pmk = [7u8; 32];
    let ptk = derive_ptk(&pmk, &AP, &STA, &[1u8; 32], &[2u8; 32]);
    assert_ne!(&ptk.0[40..48], &[0u8; 8]);
}

#[test]
fn prf_lleva_contador_en_el_primer_bloque() {
    // Sin el contador, el primer bloque es HMAC(key, label‖0‖data) y toda la
    // PTK queda desplazada 20 bytes.
    let key = [3u8; 32];
    let mut con = [0u8; 20];
    prf_sha1(&key, b"L", b"D", &mut con);
    let mut m = <Hmac<Sha1> as Mac>::new_from_slice(&key).unwrap();
    m.update(b"L\0D");
    let sin: Vec<u8> = m.finalize().into_bytes().to_vec();
    assert_ne!(con.to_vec(), sin);
    let mut esperado = <Hmac<Sha1> as Mac>::new_from_slice(&key).unwrap();
    esperado.update(b"L\0D\0");
    assert_eq!(con.to_vec(), esperado.finalize().into_bytes().to_vec());
}

#[test]
fn ptk_ordena_direcciones_y_nonces() {
    let pmk = [9u8; 32];
    let an = [0xaau8; 32];
    let sn = [0x11u8; 32];
    // min/max: intercambiar los papeles no puede cambiar la PTK.
    assert_eq!(
        derive_ptk(&pmk, &AP, &STA, &an, &sn).0,
        derive_ptk(&pmk, &STA, &AP, &sn, &an).0
    );
}

// --- AP simulado --------------------------------------------------------

fn aes_wrap(kek: &[u8], plain: &[u8]) -> Vec<u8> {
    let cipher = <aes::Aes128 as AesKeyInit>::new_from_slice(kek).unwrap();
    let n = plain.len() / 8;
    let mut a = [0xa6u8; 8];
    let mut r = plain.to_vec();
    for j in 0..6u64 {
        for i in 1..=n {
            let mut block = [0u8; 16];
            block[..8].copy_from_slice(&a);
            block[8..].copy_from_slice(&r[(i - 1) * 8..i * 8]);
            cipher.encrypt_block(GenericArray::from_mut_slice(&mut block));
            let t = (n as u64 * j + i as u64).to_be_bytes();
            for k in 0..8 {
                a[k] = block[k] ^ t[k];
            }
            r[(i - 1) * 8..i * 8].copy_from_slice(&block[8..]);
        }
    }
    let mut out = a.to_vec();
    out.extend_from_slice(&r);
    out
}

fn gtk_kde(gtk: &[u8; 16], key_id: u8) -> Vec<u8> {
    let mut kde = std::vec![0xddu8, (4 + 2 + 16) as u8, 0x00, 0x0f, 0xac, 0x01, key_id & 3, 0x00];
    kde.extend_from_slice(gtk);
    kde
}

struct Ap {
    pmk: [u8; 32],
    anonce: [u8; 32],
    gtk: [u8; 16],
    replay: u64,
}

impl Ap {
    fn new() -> Self {
        Self {
            pmk: pbkdf2_psk("clave-de-prueba", "soso-test"),
            anonce: [0x5au8; 32],
            gtk: [0x77u8; 16],
            replay: 1,
        }
    }

    fn eth(payload: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&STA);
        f.extend_from_slice(&AP);
        f.extend_from_slice(&eapol::ETH_P_EAPOL.to_be_bytes());
        f.extend_from_slice(payload);
        f
    }

    fn m1(&self) -> Vec<u8> {
        let mut buf = [0u8; 256];
        let n = build_reply(
            &mut buf,
            2,
            key_info::VERSION_HMAC_SHA1_AES,
            key_info::KEY_TYPE | key_info::ACK,
            16,
            &self.replay.to_be_bytes(),
            &self.anonce,
            &[],
        );
        Self::eth(&buf[..n])
    }

    /// PTK del lado del AP, derivada con la PRF de referencia del banco.
    fn ptk(&self, snonce: &[u8; 32]) -> Vec<u8> {
        let mut data = Vec::new();
        let (lo, hi) = if AP <= STA { (AP, STA) } else { (STA, AP) };
        data.extend_from_slice(&lo);
        data.extend_from_slice(&hi);
        let (nlo, nhi) = if self.anonce <= *snonce {
            (self.anonce, *snonce)
        } else {
            (*snonce, self.anonce)
        };
        data.extend_from_slice(&nlo);
        data.extend_from_slice(&nhi);
        prf_referencia(&self.pmk, b"Pairwise key expansion", &data, 48)
    }

    fn m3(&mut self, ptk: &[u8], cifrar: bool) -> Vec<u8> {
        self.replay += 1;
        let kde = gtk_kde(&self.gtk, 1);
        let kd = if cifrar {
            let mut p = kde.clone();
            while p.len() % 8 != 0 {
                p.push(0xdd); // relleno de KDE nulo permitido por la norma
            }
            aes_wrap(&ptk[16..32], &p)
        } else {
            kde
        };
        let mut flags = key_info::KEY_TYPE | key_info::ACK | key_info::MIC
            | key_info::SECURE
            | key_info::INSTALL;
        if cifrar {
            flags |= key_info::ENCR_KEY_DATA;
        }
        let mut buf = [0u8; 512];
        let n = build_reply(
            &mut buf,
            2,
            key_info::VERSION_HMAC_SHA1_AES,
            flags,
            16,
            &self.replay.to_be_bytes(),
            &self.anonce,
            &kd,
        );
        set_mic(&ptk[..16], &mut buf[..n]);
        Self::eth(&buf[..n])
    }

    fn group1(&mut self, ptk: &[u8], gtk: &[u8; 16], key_id: u8) -> Vec<u8> {
        self.replay += 1;
        let mut p = gtk_kde(gtk, key_id);
        while p.len() % 8 != 0 {
            p.push(0xdd);
        }
        let kd = aes_wrap(&ptk[16..32], &p);
        let flags =
            key_info::ACK | key_info::MIC | key_info::SECURE | key_info::ENCR_KEY_DATA;
        let mut buf = [0u8; 512];
        let n = build_reply(
            &mut buf,
            2,
            key_info::VERSION_HMAC_SHA1_AES,
            flags,
            16,
            &self.replay.to_be_bytes(),
            &[0u8; 32],
            &kd,
        );
        set_mic(&ptk[..16], &mut buf[..n]);
        Self::eth(&buf[..n])
    }
}

fn supplicant(ap: &Ap, snonce: [u8; 32]) -> Supplicant {
    Supplicant::new(ap.pmk, STA, AP, &RSN_IE, snonce)
}

// --- defectos reproducidos por la investigación -------------------------

#[test]
fn m1_valido_produce_m2() {
    let ap = Ap::new();
    let mut s = supplicant(&ap, [0x33u8; 32]);
    let r = s.on_ethernet(&ap.m1());
    assert_eq!(r.dropped, None, "M1 con ACK descartado");
    assert!(r.send);
    assert_eq!(s.state(), State::Msg2Sent);
}

#[test]
fn m2_cabecera_en_el_cable() {
    let ap = Ap::new();
    let mut s = supplicant(&ap, [0x33u8; 32]);
    s.on_ethernet(&ap.m1());
    let m2 = EapolKey::parse(&s.tx()[14..]).unwrap();
    // versión 2, por pares, con MIC y sin ACK.
    assert_eq!(m2.key_info(), 0x010a);
    assert_eq!(m2.key_length(), 0);
}

#[test]
fn m2_repite_el_rsn_ie_negociado() {
    let ap = Ap::new();
    let mut s = supplicant(&ap, [0x33u8; 32]);
    s.on_ethernet(&ap.m1());
    let m2 = EapolKey::parse(&s.tx()[14..]).unwrap();
    assert_eq!(m2.key_data(), &RSN_IE[..]);
    // La longitud del cuerpo tiene que contar el Key Data.
    let raw = m2.bytes();
    let body = u16::from_be_bytes([raw[2], raw[3]]) as usize;
    assert_eq!(body, eapol::EAPOL_BODY_LEN + RSN_IE.len());
    assert_eq!(raw.len(), 4 + body);
}

#[test]
fn m2_lleva_mic_valido_para_el_ap() {
    let ap = Ap::new();
    let snonce = [0x33u8; 32];
    let mut s = supplicant(&ap, snonce);
    s.on_ethernet(&ap.m1());
    let ptk = ap.ptk(&snonce);
    let m2 = EapolKey::parse(&s.tx()[14..]).unwrap();
    let mut scratch = [0u8; MAX_EAPOL];
    assert!(eapol::verify_mic(&ptk[..16], &m2, &mut scratch));
    // Y la PTK del suplicante coincide con la del AP, derivada por otro camino.
    assert_eq!(&s.ptk().unwrap().0[..], &ptk[..]);
}

#[test]
fn m4_cabecera_en_el_cable() {
    let ap = &mut Ap::new();
    let snonce = [0x33u8; 32];
    let mut s = supplicant(ap, snonce);
    s.on_ethernet(&ap.m1());
    let ptk = ap.ptk(&snonce);
    let m3 = ap.m3(&ptk, true);
    let r = s.on_ethernet(&m3);
    assert_eq!(r.dropped, None);
    let m4 = EapolKey::parse(&s.tx()[14..]).unwrap();
    // versión 2, por pares, MIC y secure; sin ACK ni Key Data.
    assert_eq!(m4.key_info(), 0x030a);
    assert!(m4.key_data().is_empty());
    let mut scratch = [0u8; MAX_EAPOL];
    assert!(eapol::verify_mic(&ptk[..16], &m4, &mut scratch));
}

#[test]
fn gtk_se_extrae_completa_y_con_su_indice() {
    let gtk = [16u8, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31];
    let kd = gtk_kde(&gtk, 2);
    let g = kde::find_gtk(&kd).expect("GTK presente");
    assert_eq!(g.key, gtk);
    assert_eq!(g.key_id, 2);
}

#[test]
fn eapol_corto_no_hace_panico() {
    let ap = Ap::new();
    let m1 = ap.m1();
    for n in 0..m1.len() {
        let mut s = supplicant(&ap, [0x33u8; 32]);
        let r = s.on_ethernet(&m1[..n]);
        if n >= 14 + eapol::EAPOL_KEY_LEN {
            continue;
        }
        assert!(r.install_tk.is_none() && !r.authorized, "n={n}");
    }
    // Y una carga EAPOL de 4 bytes, que era el caso del pánico por índice 5.
    let mut s = supplicant(&ap, [0x33u8; 32]);
    assert_eq!(
        s.on_eapol(&[2, 3, 0, 0]).dropped,
        Some(Discard::NotEapol(ParseError::TooShort))
    );
}

// --- handshake completo -------------------------------------------------

#[test]
fn handshake_completo_instala_la_tk_no_la_kck() {
    let ap = &mut Ap::new();
    let snonce = [0x33u8; 32];
    let mut s = supplicant(ap, snonce);
    assert!(!s.authorized());
    s.on_ethernet(&ap.m1());
    assert!(!s.authorized(), "asociada no es autorizada");

    let ptk = ap.ptk(&snonce);
    let r = s.on_ethernet(&ap.m3(&ptk, true));
    assert_eq!(r.dropped, None);
    assert!(r.send && r.authorized);
    assert!(s.authorized());

    let tk = r.install_tk.expect("TK");
    assert_eq!(&tk[..], &ptk[32..48], "CCMP usa la TK");
    assert_ne!(&tk[..], &ptk[..16], "instalar la KCK como clave de datos");
    let g = r.install_gtk.expect("GTK");
    assert_eq!(g.key, ap.gtk);
    assert_eq!(g.key_id, 1);
}

#[test]
fn key_data_sin_cifrar_tambien_vale() {
    let ap = &mut Ap::new();
    let snonce = [0x44u8; 32];
    let mut s = supplicant(ap, snonce);
    s.on_ethernet(&ap.m1());
    let ptk = ap.ptk(&snonce);
    let r = s.on_ethernet(&ap.m3(&ptk, false));
    assert_eq!(r.dropped, None);
    assert_eq!(r.install_gtk.unwrap().key, ap.gtk);
}

#[test]
fn m3_con_mic_falso_se_descarta() {
    let ap = &mut Ap::new();
    let snonce = [0x33u8; 32];
    let mut s = supplicant(ap, snonce);
    s.on_ethernet(&ap.m1());
    let ptk = ap.ptk(&snonce);
    let mut m3 = ap.m3(&ptk, true);
    let mic = 14 + eapol::OFF_MIC;
    m3[mic] ^= 0xff;
    let r = s.on_ethernet(&m3);
    assert_eq!(r.dropped, Some(Discard::BadMic));
    assert!(!s.authorized());
}

#[test]
fn m3_con_otra_anonce_se_descarta() {
    let ap = &mut Ap::new();
    let snonce = [0x33u8; 32];
    let mut s = supplicant(ap, snonce);
    s.on_ethernet(&ap.m1());
    let ptk = ap.ptk(&snonce);
    let mut m3 = ap.m3(&ptk, true);
    m3[14 + eapol::OFF_NONCE] ^= 0x01;
    // Cambiar la ANonce invalida también el MIC: se rehace para aislar el caso.
    let n = m3.len() - 14;
    set_mic(&ptk[..16], &mut m3[14..14 + n]);
    assert_eq!(s.on_ethernet(&m3).dropped, Some(Discard::NonceMismatch));
}

#[test]
fn m3_con_key_data_ilegible_se_descarta() {
    let ap = &mut Ap::new();
    let snonce = [0x33u8; 32];
    let ptk = ap.ptk(&snonce);
    let mut s = supplicant(ap, snonce);
    s.on_ethernet(&ap.m1());

    let mut m3 = ap.m3(&ptk, true);
    // Envoltura AES corrompida: el control de integridad del RFC 3394 la caza.
    m3[14 + eapol::OFF_KEY_DATA] ^= 0xff;
    let n = m3.len() - 14;
    set_mic(&ptk[..16], &mut m3[14..14 + n]);
    let r = s.on_ethernet(&m3);
    assert_eq!(r.dropped, Some(Discard::KeyDataUnwrap));
    assert!(r.install_gtk.is_none() && !s.authorized());
}

#[test]
fn contador_de_reenvio_que_retrocede_se_descarta() {
    let ap = &mut Ap::new();
    let snonce = [0x33u8; 32];
    let mut s = supplicant(ap, snonce);
    s.on_ethernet(&ap.m1());
    let ptk = ap.ptk(&snonce);
    let m3 = ap.m3(&ptk, true);
    s.on_ethernet(&m3);
    // Un M3 antiguo, con MIC válido pero contador anterior.
    ap.replay = 0;
    let viejo = ap.m3(&ptk, true);
    assert_eq!(s.on_ethernet(&viejo).dropped, Some(Discard::Replay));
}

#[test]
fn renovacion_de_gtk_instala_la_nueva() {
    let ap = &mut Ap::new();
    let snonce = [0x33u8; 32];
    let mut s = supplicant(ap, snonce);
    s.on_ethernet(&ap.m1());
    let ptk = ap.ptk(&snonce);
    s.on_ethernet(&ap.m3(&ptk, true));
    let nueva = [0xa1u8; 16];
    let r = s.on_ethernet(&ap.group1(&ptk, &nueva, 2));
    assert_eq!(r.dropped, None);
    assert!(r.send);
    assert!(r.install_tk.is_none(), "la renovación de grupo no toca la PTK");
    let g = r.install_gtk.expect("GTK nueva");
    assert_eq!(g.key, nueva);
    assert_eq!(g.key_id, 2);
    let g2 = EapolKey::parse(&s.tx()[14..]).unwrap();
    assert_eq!(g2.key_info(), 0x0302);
}

#[test]
fn trafico_normal_pasa_de_largo() {
    let ap = Ap::new();
    let mut s = supplicant(&ap, [0x33u8; 32]);
    let mut arp = std::vec![0u8; 60];
    arp[12] = 0x08;
    arp[13] = 0x06;
    let r = s.on_ethernet(&arp);
    assert_eq!(r, Outcome::default(), "una trama ARP no es asunto del supplicant");
}

#[test]
fn mensaje_fuera_de_secuencia_se_descarta() {
    let ap = &mut Ap::new();
    let snonce = [0x33u8; 32];
    let mut s = supplicant(ap, snonce);
    let ptk = ap.ptk(&snonce);
    // M3 sin M1 previo: no hay PTK con la que validar nada.
    assert_eq!(s.on_ethernet(&ap.m3(&ptk, true)).dropped, Some(Discard::Unexpected));
    assert!(!s.authorized());
}

#[test]
fn mensajes_sin_ack_no_son_del_ap() {
    let ap = Ap::new();
    let mut s = supplicant(&ap, [0x33u8; 32]);
    let mut buf = [0u8; 256];
    let n = build_reply(
        &mut buf,
        2,
        key_info::VERSION_HMAC_SHA1_AES,
        key_info::KEY_TYPE | key_info::MIC,
        0,
        &[0u8; 8],
        &[0u8; 32],
        &[],
    );
    assert_eq!(s.on_eapol(&buf[..n]).dropped, Some(Discard::NotFromAp));
}

#[test]
fn descriptor_de_otra_version_se_rechaza() {
    let ap = Ap::new();
    let mut s = supplicant(&ap, [0x33u8; 32]);
    let mut buf = [0u8; 256];
    let n = build_reply(
        &mut buf,
        2,
        1, // HMAC-MD5/RC4: WPA1, no este supplicant
        key_info::KEY_TYPE | key_info::ACK,
        16,
        &[0u8; 8],
        &[1u8; 32],
        &[],
    );
    assert_eq!(s.on_eapol(&buf[..n]).dropped, Some(Discard::BadVersion));
}

#[test]
fn longitud_declarada_incoherente_se_rechaza() {
    let ap = Ap::new();
    let m1 = ap.m1();
    let mut mal = m1[14..].to_vec();
    // Key Data Length mayor que el cuerpo declarado.
    mal[eapol::OFF_KD_LEN] = 0xff;
    let mut s = supplicant(&ap, [0x33u8; 32]);
    assert_eq!(
        s.on_eapol(&mal).dropped,
        Some(Discard::NotEapol(ParseError::BadLength))
    );
}
