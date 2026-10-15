//! Aceptación reservada de R24.

use gptdisk::{Error, Guid, Header, SECTOR, SIGNATURE};

fn cabecera(entry_size: u32, first: u64, last: u64) -> Header {
    Header {
        revision: 0x0001_0000,
        header_size: 92,
        my_lba: 1,
        alternate_lba: 131_071,
        first_usable: first,
        last_usable: last,
        disk_guid: Guid::parse("12345678-1234-5678-1234-567812345678").unwrap(),
        entries_lba: 2,
        num_entries: 128,
        entry_size,
    }
}

fn sector(h: &Header) -> [u8; SECTOR] {
    h.render(&vec![0u8; h.entries_bytes()])
}

#[test]
fn cabecera_correcta_sigue_leyendose() {
    for esz in [128u32, 256, 512] {
        let h = cabecera(esz, 34, 131_000);
        assert_eq!(Header::parse(&sector(&h)), Ok(h), "entry_size {esz}");
    }
}

#[test]
fn entry_size_debe_ser_128_por_potencia_de_dos() {
    for esz in [136u32, 192, 384, 640, 0, 64, 129] {
        let h = cabecera(esz, 34, 131_000);
        assert_eq!(Header::parse(&sector(&h)), Err(Error::BadHeader), "entry_size {esz}");
    }
}

#[test]
fn rango_utilizable_incoherente() {
    let h = cabecera(128, 5000, 4999);
    assert_eq!(Header::parse(&sector(&h)), Err(Error::BadHeader));
    let h = cabecera(128, 200_000, 34);
    assert_eq!(Header::parse(&sector(&h)), Err(Error::BadHeader));
}

#[test]
fn rango_de_un_solo_sector_es_valido() {
    let h = cabecera(128, 100, 100);
    assert_eq!(Header::parse(&sector(&h)), Ok(h));
}

#[test]
fn la_firma_se_comprueba_primero() {
    let mut s = sector(&cabecera(136, 9, 1));
    s[0] = b'X';
    assert_eq!(Header::parse(&s), Err(Error::BadSignature));
    let mut s = [0u8; SECTOR];
    s[0..8].copy_from_slice(SIGNATURE);
    assert_eq!(Header::parse(&s), Err(Error::BadHeader));
}
