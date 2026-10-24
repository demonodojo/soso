//! Aceptación reservada de R42.

use gptdisk::*;

fn hdr() -> Header {
    Header {
        revision: 0x0001_0000,
        header_size: 92,
        my_lba: 1,
        alternate_lba: 999,
        first_usable: 34,
        last_usable: 966,
        disk_guid: Guid([7; 16]),
        entries_lba: 2,
        num_entries: 128,
        entry_size: 128,
    }
}

fn entradas(h: &Header) -> Vec<u8> {
    vec![0u8; h.entries_bytes()]
}

fn sector(h: &Header) -> [u8; SECTOR] {
    h.render(&entradas(h))
}

#[test]
fn cabecera_valida_sigue_valida() {
    let h = hdr();
    assert_eq!(Header::parse(&sector(&h)), Ok(h));
    let h2 = Header { header_size: 128, entry_size: 4096, num_entries: 512, ..h };
    assert_eq!(Header::parse(&sector(&h2)), Ok(h2));
    let h3 = Header { entry_size: 256, num_entries: 1, first_usable: 50, last_usable: 50, ..h };
    assert_eq!(Header::parse(&sector(&h3)), Ok(h3));
}

#[test]
fn crc_de_cabecera_erroneo() {
    let h = hdr();
    let s = sector(&h);
    for off in [16usize, 17, 19, 24, 40, 56, 71, 80, 88, 91] {
        let mut m = s;
        m[off] ^= 0x01;
        // Los cambios que además invalidan otro campo también son BadHeader.
        assert_eq!(Header::parse(&m), Err(Error::BadHeader), "byte {off}");
    }
    // Con header_size 128 también cuentan los bytes 92..128.
    let h2 = Header { header_size: 128, ..h };
    let mut m = sector(&h2);
    assert!(Header::parse(&m).is_ok());
    m[100] = 0xFF;
    assert_eq!(Header::parse(&m), Err(Error::BadHeader));
    // Bytes más allá de header_size no entran en el CRC.
    let mut m = s;
    m[300] = 0xFF;
    assert_eq!(Header::parse(&m), Ok(h));
    // La firma manda sobre el CRC.
    let mut m = s;
    m[0] = b'X';
    assert_eq!(Header::parse(&m), Err(Error::BadSignature));
}

#[test]
fn tamano_de_entrada_potencia_de_dos() {
    let h = hdr();
    for malo in [0u32, 64, 127, 136, 192, 384, 4097, 8192, 1 << 20] {
        let h2 = Header { entry_size: malo, ..h };
        assert_eq!(Header::parse(&sector(&h2)), Err(Error::BadHeader), "entry_size {malo}");
    }
    for bueno in [128u32, 256, 512, 1024, 2048, 4096] {
        let h2 = Header { entry_size: bueno, ..h };
        assert!(Header::parse(&sector(&h2)).is_ok(), "entry_size {bueno}");
    }
    for n in [0u32, 513] {
        let h2 = Header { num_entries: n, ..h };
        assert_eq!(Header::parse(&sector(&h2)), Err(Error::BadHeader));
    }
}

#[test]
fn lbas_incoherentes() {
    let h = hdr();
    for h2 in [
        Header { my_lba: 0, ..h },
        Header { entries_lba: 0, ..h },
        Header { first_usable: 967, ..h },
        Header { first_usable: u64::MAX, last_usable: 0, ..h },
    ] {
        assert_eq!(Header::parse(&sector(&h2)), Err(Error::BadHeader), "{h2:?}");
    }
    let h2 = Header { first_usable: 966, ..h };
    assert!(Header::parse(&sector(&h2)).is_ok());
}

#[test]
fn indices_extremos_no_desbordan() {
    let h = Header { num_entries: 4, ..hdr() };
    let mut v = entradas(&h);
    assert!(entry(&v, &h, 3).is_some());
    assert!(entry(&v, &h, 4).is_none());
    assert!(entry_mut(&mut v, &h, 3).is_some());
    assert!(entry_mut(&mut v, &h, 4).is_none());
    for i in [usize::MAX, usize::MAX / 128, usize::MAX / 128 - 1, usize::MAX / 2] {
        assert!(entry(&v, &h, i).is_none(), "entry {i}");
        assert!(entry_mut(&mut v, &h, i).is_none(), "entry_mut {i}");
    }
    let h4096 = Header { entry_size: 4096, ..h };
    assert!(entry(&v, &h4096, usize::MAX / 4096).is_none());
    assert!(entry_mut(&mut v, &h4096, usize::MAX / 4096).is_none());
    // Búfer más corto que la tabla.
    assert!(entry(&v[..200], &h, 1).is_none());
    assert!(entry(&v[..256], &h, 1).is_some());
}

#[test]
fn entradas_truncadas_no_entran_en_panico() {
    let mut e = [0u8; 128];
    e[0] = 1;
    e[16] = 2;
    e[32..40].copy_from_slice(&100u64.to_le_bytes());
    e[40..48].copy_from_slice(&200u64.to_le_bytes());
    assert!(entry_used(&e));
    assert_eq!(entry_first_lba(&e), 100);
    assert_eq!(entry_last_lba(&e), 200);
    assert_eq!(entry_type(&e).0[0], 1);
    assert_eq!(entry_unique(&e).0[0], 2);

    assert!(!entry_used(&e[..15]));
    assert!(entry_used(&e[..16]));
    assert!(!entry_used(&[]));
    assert_eq!(entry_type(&e[..15]), Guid::ZERO);
    assert_eq!(entry_type(&e[..16]).0[0], 1);
    assert_eq!(entry_unique(&e[..31]), Guid::ZERO);
    assert_eq!(entry_unique(&e[..32]).0[0], 2);
    assert_eq!(entry_first_lba(&e[..39]), 0);
    assert_eq!(entry_first_lba(&e[..40]), 100);
    assert_eq!(entry_last_lba(&e[..47]), 0);
    assert_eq!(entry_last_lba(&e[..48]), 200);
    assert_eq!(entry_last_lba(&[]), 0);

    let mut c = e;
    set_entry_first_lba(&mut c[..39], 7);
    set_entry_last_lba(&mut c[..47], 9);
    assert_eq!(c, e);
    set_entry_first_lba(&mut c[..40], 7);
    set_entry_last_lba(&mut c[..48], 9);
    assert_eq!(entry_first_lba(&c), 7);
    assert_eq!(entry_last_lba(&c), 9);
    set_entry_first_lba(&mut [], 1);
    set_entry_last_lba(&mut [], 1);
}

fn poner(v: &mut [u8], i: usize, first: u64, last: u64) {
    let e = &mut v[i * 128..(i + 1) * 128];
    e[0] = 0xAA;
    e[32..40].copy_from_slice(&first.to_le_bytes());
    e[40..48].copy_from_slice(&last.to_le_bytes());
}

#[test]
fn relayout_rechaza_particiones_invertidas_sin_tocar_nada() {
    let disco = 1_000_000u64;
    let mut h = hdr();
    let mut v = entradas(&h);
    poner(&mut v, 0, 2048, 4095);
    poner(&mut v, 1, 9000, 5000);
    let (h0, v0) = (h, v.clone());
    assert_eq!(relayout(&mut h, &mut v, disco), Err(Error::BadHeader));
    assert_eq!(h, h0);
    assert_eq!(v, v0);

    // Gana sobre TooSmall: la tabla es incoherente y además el disco no da.
    let mut v = entradas(&h);
    poner(&mut v, 0, 2048, 900_000);
    poner(&mut v, 5, 30, 10);
    assert_eq!(relayout(&mut h, &mut v, 100), Err(Error::BadHeader));
    // Con las entradas cortas, ShortEntries va antes.
    assert_eq!(relayout(&mut h, &mut v[..100], disco), Err(Error::ShortEntries));

    // first == last es válida, y una entrada vacía con basura no cuenta.
    let mut v = entradas(&h);
    poner(&mut v, 0, 2048, 2048);
    v[3 * 128 + 32..3 * 128 + 40].copy_from_slice(&50u64.to_le_bytes());
    assert!(relayout(&mut h, &mut v, disco).is_ok());
    assert_eq!(entry_last_lba(entry(&v, &h, 0).unwrap()), h.last_usable);
}
