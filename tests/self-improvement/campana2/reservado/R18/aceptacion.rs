//! Aceptación reservada de R18.

use soso_wpa2::kde::{find_gtk, find_pmkid, KDE_DATA_TYPE_PMKID};

fn kde(tipo: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0xdd, (4 + payload.len()) as u8, 0x00, 0x0f, 0xac, tipo];
    v.extend_from_slice(payload);
    v
}

#[test]
fn encuentra_el_pmkid() {
    assert_eq!(KDE_DATA_TYPE_PMKID, 4);
    let pmkid: Vec<u8> = (1..=16).collect();
    let datos = kde(4, &pmkid);
    assert_eq!(find_pmkid(&datos).map(|k| k.to_vec()), Some(pmkid.clone()));
    // Detrás de otros elementos.
    let mut mezcla = vec![0x30, 2, 0xaa, 0xbb];
    mezcla.extend(kde(1, &[0x01, 0x00, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9]));
    mezcla.extend(kde(4, &pmkid));
    assert_eq!(find_pmkid(&mezcla).map(|k| k.to_vec()), Some(pmkid));
    assert!(find_gtk(&mezcla).is_some(), "find_gtk sigue funcionando");
}

#[test]
fn payloads_cortos_se_ignoran_y_largos_se_cortan() {
    assert_eq!(find_pmkid(&kde(4, &[1; 15])), None);
    let largo: Vec<u8> = (0..20).collect();
    assert_eq!(find_pmkid(&kde(4, &largo)).map(|k| k.to_vec()), Some((0..16).collect::<Vec<u8>>()));
    // Un KDE corto no tapa a uno bueno posterior.
    let mut v = kde(4, &[7; 10]);
    v.extend(kde(4, &[8; 16]));
    assert_eq!(find_pmkid(&v), Some([8u8; 16]));
}

#[test]
fn sin_pmkid_o_truncado_no_hay_nada() {
    assert_eq!(find_pmkid(&[]), None);
    assert_eq!(find_pmkid(&kde(1, &[0; 18])), None);
    let mut truncado = kde(4, &[5; 16]);
    truncado.pop();
    truncado[1] = 20; // declara más de lo que hay
    assert_eq!(find_pmkid(&truncado), None);
    assert_eq!(find_pmkid(&[0x30, 0]), None);
}
