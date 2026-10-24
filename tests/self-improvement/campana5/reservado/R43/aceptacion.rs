//! Aceptación reservada de R43.

use soso_wpa2::eapol::{
    build_reply, set_mic, verify_mic, EapolKey, EAPOL_BODY_LEN, EAPOL_KEY_LEN, OFF_KD_LEN,
};
use soso_wpa2::kde::find_gtk;
use soso_wpa2::{key_info, ParseError, Supplicant};

const FLAGS: u16 = key_info::KEY_TYPE | key_info::ACK;

fn trama(kd: &[u8]) -> Vec<u8> {
    let mut buf = vec![0u8; 256];
    let n = build_reply(&mut buf, 2, 2, FLAGS, 16, &[0, 0, 0, 0, 0, 0, 0, 1], &[7u8; 32], kd);
    assert!(n > 0);
    buf.truncate(n);
    buf
}

fn kde_gtk(carga: &[u8]) -> Vec<u8> {
    let mut v = vec![0xddu8, (4 + carga.len()) as u8, 0x00, 0x0f, 0xac, 0x01];
    v.extend_from_slice(carga);
    v
}

fn carga(byte0: u8, clave: u8) -> Vec<u8> {
    let mut c = vec![byte0, 0x00];
    c.extend_from_slice(&[clave; 16]);
    c
}

#[test]
fn trama_valida_sigue_valiendo() {
    for v in [1u8, 2, 3] {
        let mut t = trama(&[]);
        t[0] = v;
        let k = EapolKey::parse(&t).expect("versión válida");
        assert_eq!(k.bytes().len(), EAPOL_KEY_LEN);
    }
    let kd = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let t = trama(&kd);
    assert_eq!(EapolKey::parse(&t).unwrap().key_data(), &kd);
}

#[test]
fn version_8021x_invalida() {
    for v in [0u8, 4, 9, 0xff] {
        let mut t = trama(&[]);
        t[0] = v;
        assert_eq!(EapolKey::parse(&t).err(), Some(ParseError::BadEapolVersion), "v={v}");
    }
    // Orden: la longitud mínima va antes que la versión; la versión antes que el tipo.
    assert_eq!(EapolKey::parse(&[0u8; 10]).err(), Some(ParseError::TooShort));
    let mut t = trama(&[]);
    t[0] = 0;
    t[1] = 1;
    assert_eq!(EapolKey::parse(&t).err(), Some(ParseError::BadEapolVersion));
    let mut t = trama(&[]);
    t[1] = 1;
    assert_eq!(EapolKey::parse(&t).err(), Some(ParseError::NotKeyFrame));
}

#[test]
fn el_cuerpo_debe_ser_exacto() {
    let kd = [9u8; 8];
    let mut t = trama(&kd);
    let body = u16::from_be_bytes([t[2], t[3]]);
    assert_eq!(body as usize, EAPOL_BODY_LEN + 8);
    // Cuerpo mayor que lo que declara Key Data Length (con bytes de sobra presentes).
    t.extend_from_slice(&[0u8; 4]);
    let mut mayor = t.clone();
    mayor[2..4].copy_from_slice(&(body + 4).to_be_bytes());
    assert_eq!(EapolKey::parse(&mayor).err(), Some(ParseError::BadLength));
    // Cuerpo menor que lo que cabe Key Data.
    let mut menor = t.clone();
    menor[2..4].copy_from_slice(&(body - 1).to_be_bytes());
    assert_eq!(EapolKey::parse(&menor).err(), Some(ParseError::BadLength));
    // Cuerpo menor que el mínimo.
    let mut min = t.clone();
    min[2..4].copy_from_slice(&((EAPOL_BODY_LEN as u16) - 1).to_be_bytes());
    assert_eq!(EapolKey::parse(&min).err(), Some(ParseError::BadLength));
    // Key Data Length mayor que el cuerpo.
    let mut kdl = t.clone();
    kdl[OFF_KD_LEN..OFF_KD_LEN + 2].copy_from_slice(&9u16.to_be_bytes());
    assert_eq!(EapolKey::parse(&kdl).err(), Some(ParseError::BadLength));
    // Relleno de Ethernet tras el cuerpo: se ignora y no entra en bytes().
    let k = EapolKey::parse(&t).expect("relleno ignorado");
    assert_eq!(k.bytes().len(), EAPOL_KEY_LEN + 8);
    assert_eq!(k.key_data(), &kd);
}

#[test]
fn verify_mic_exige_kck_de_16_bytes() {
    let kck = [0x42u8; 16];
    let mut t = trama(&[1, 2, 3, 4, 5, 6, 7, 8]);
    set_mic(&kck, &mut t);
    let k = EapolKey::parse(&t).unwrap();
    let mut scratch = [0u8; 512];
    assert!(verify_mic(&kck, &k, &mut scratch));
    for n in [0usize, 1, 15, 17, 20, 32] {
        let largo = [0x42u8; 32];
        assert!(!verify_mic(&largo[..n], &k, &mut scratch), "n={n}");
    }
    // Con una KCK de 16 bytes pero distinta, falla igual.
    assert!(!verify_mic(&[0x43u8; 16], &k, &mut scratch));
}

#[test]
fn gtk_bien_formada() {
    let g = find_gtk(&kde_gtk(&carga(0x05, 0xab))).expect("GTK");
    assert_eq!((g.key, g.key_id, g.tx), ([0xab; 16], 1, true));
    let g = find_gtk(&kde_gtk(&carga(0x03, 0xcd))).expect("GTK");
    assert_eq!((g.key_id, g.tx), (3, false));
    // Otro elemento delante (RSN IE) y KDE de otro tipo: se saltan.
    let mut kd = vec![0x30u8, 2, 0xaa, 0xbb];
    kd.extend_from_slice(&[0xdd, 6, 0x00, 0x0f, 0xac, 0x04, 1, 2]);
    kd.extend_from_slice(&kde_gtk(&carga(0x02, 0x11)));
    assert_eq!(find_gtk(&kd).unwrap().key_id, 2);
}

#[test]
fn gtk_con_carga_de_longitud_incorrecta() {
    let mut larga = carga(0x01, 0x22);
    larga.extend_from_slice(&[0x22; 16]); // 32 bytes de clave
    assert_eq!(find_gtk(&kde_gtk(&larga)), None);
    let mut mas_uno = carga(0x01, 0x22);
    mas_uno.push(0);
    assert_eq!(find_gtk(&kde_gtk(&mas_uno)), None);
    let mut corta = carga(0x01, 0x22);
    corta.pop();
    assert_eq!(find_gtk(&kde_gtk(&corta)), None);
}

#[test]
fn gtk_con_key_id_cero() {
    assert_eq!(find_gtk(&kde_gtk(&carga(0x00, 0x33))), None);
    // Sólo cuentan los 2 bits bajos: 0x04 (Tx) con Key ID 0 sigue siendo inválido.
    assert_eq!(find_gtk(&kde_gtk(&carga(0x04, 0x33))), None);
    assert_eq!(find_gtk(&kde_gtk(&carga(0xfc, 0x33))), None);
}

#[test]
fn solo_cuenta_el_primer_kde_de_gtk() {
    let buena = kde_gtk(&carga(0x02, 0x55));
    let mala = kde_gtk(&carga(0x00, 0x66));
    let mut corta = carga(0x01, 0x77);
    corta.pop();
    let corta = kde_gtk(&corta);

    let mut kd = mala.clone();
    kd.extend_from_slice(&buena);
    assert_eq!(find_gtk(&kd), None, "mala primero");

    let mut kd = corta.clone();
    kd.extend_from_slice(&buena);
    assert_eq!(find_gtk(&kd), None, "corta primero");

    let mut kd = buena.clone();
    kd.extend_from_slice(&mala);
    assert_eq!(find_gtk(&kd).unwrap().key, [0x55; 16], "buena primero");
}

#[test]
fn el_suplicante_descarta_version_8021x_invalida() {
    let mut s = Supplicant::new([1u8; 32], [2u8; 6], [3u8; 6], &[0x30, 0], [4u8; 32]);
    let mut t = trama(&[]);
    t[0] = 0;
    let r = s.on_eapol(&t);
    assert!(!r.send);
    assert_eq!(
        r.dropped,
        Some(soso_wpa2::Discard::NotEapol(ParseError::BadEapolVersion))
    );
    // Y una versión válida sí produce M2.
    let t = trama(&[]);
    let r = s.on_eapol(&t);
    assert!(r.send && r.dropped.is_none());
}
