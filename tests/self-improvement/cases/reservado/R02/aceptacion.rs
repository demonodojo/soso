//! Aceptación reservada de R02: `Header::parse` valida el CRC32 de la cabecera.
//!
//! Se comprueba el comportamiento, no el nombre del error: una cabecera intacta
//! se parsea y una manipulada se rechaza.

use gptdisk::{Guid, Header, SECTOR};

fn cabecera_de_prueba() -> (Header, Vec<u8>) {
    let hdr = Header {
        revision: 0x0001_0000,
        header_size: 92,
        my_lba: 1,
        alternate_lba: 2047,
        first_usable: 34,
        last_usable: 2014,
        disk_guid: Guid([0x11; 16]),
        entries_lba: 2,
        num_entries: 128,
        entry_size: 128,
    };
    let entradas = vec![0u8; hdr.entries_bytes()];
    (hdr, entradas)
}

#[test]
fn una_cabecera_intacta_se_parsea() {
    let (hdr, entradas) = cabecera_de_prueba();
    let sector = hdr.render(&entradas);
    let leida = Header::parse(&sector).expect("una cabecera recién generada debe parsearse");
    assert_eq!(leida.my_lba, hdr.my_lba);
    assert_eq!(leida.num_entries, hdr.num_entries);
    assert_eq!(leida.entry_size, hdr.entry_size);
}

#[test]
fn un_campo_manipulado_invalida_la_cabecera() {
    let (hdr, entradas) = cabecera_de_prueba();
    let mut sector = hdr.render(&entradas);
    // Cambiar `first_usable` sin recalcular el CRC: los bytes siguen siendo
    // plausibles, solo que ya no son los que se firmaron.
    sector[40] ^= 0x04;
    assert!(
        Header::parse(&sector).is_err(),
        "una cabecera con el CRC roto no debe aceptarse"
    );
}

#[test]
fn un_crc_a_cero_invalida_la_cabecera() {
    let (hdr, entradas) = cabecera_de_prueba();
    let mut sector = hdr.render(&entradas);
    for b in &mut sector[16..20] {
        *b = 0;
    }
    assert!(
        Header::parse(&sector).is_err(),
        "un CRC a cero no es un CRC válido"
    );
}

#[test]
fn el_sector_sigue_midiendo_lo_de_siempre() {
    let (hdr, entradas) = cabecera_de_prueba();
    assert_eq!(hdr.render(&entradas).len(), SECTOR);
}
