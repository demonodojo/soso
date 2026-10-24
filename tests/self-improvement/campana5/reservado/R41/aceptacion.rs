//! Aceptación reservada de R41.

use gptdisk::{Guid, protective_mbr_fix, SECTOR};

const BUENO: &str = "12345678-1234-5678-1234-567812345678";

#[test]
fn parse_acepta_la_forma_canonica() {
    let g = Guid::parse(BUENO).unwrap();
    assert_eq!(g.0[0], 0x78);
    assert_eq!(g.0[3], 0x12);
    assert_eq!(g.0[4], 0x34);
    assert_eq!(g.0[6], 0x78);
    assert_eq!(g.0[8], 0x12);
    assert_eq!(g.0[15], 0x78);
    assert_eq!(format!("{g}"), BUENO);
    let mixto = "c12a7328-F81F-11d2-bA4b-00a0C93ec93B";
    let g = Guid::parse(mixto).unwrap();
    assert_eq!(format!("{g}"), mixto.to_uppercase());
}

#[test]
fn parse_rechaza_un_guion_en_lugar_de_un_digito() {
    for i in 0..36 {
        let mut s = BUENO.as_bytes().to_vec();
        if matches!(i, 8 | 13 | 18 | 23) {
            continue;
        }
        s[i] = b'-';
        let s = String::from_utf8(s).unwrap();
        assert!(Guid::parse(&s).is_none(), "posición {i}: {s}");
    }
}

#[test]
fn parse_rechaza_guiones_mal_colocados_y_basura() {
    assert!(Guid::parse("").is_none());
    assert!(Guid::parse(&BUENO[..35]).is_none());
    assert!(Guid::parse(&format!("{BUENO}0")).is_none());
    assert!(Guid::parse("123456781234567812345678123456781234").is_none());
    assert!(Guid::parse("1234567-81234-5678-1234-567812345678").is_none());
    assert!(Guid::parse("1234567g-1234-5678-1234-567812345678").is_none());
    assert!(Guid::parse(" 2345678-1234-5678-1234-567812345678").is_none());
    // Un guion de más en una posición de separador no se compensa con uno de menos.
    assert!(Guid::parse("12345678-1234-5678-12345-67812345678").is_none());
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[test]
fn mbr_protector_completo_sobre_un_mbr_sucio() {
    let mut mbr = [0xAAu8; SECTOR];
    for (i, b) in mbr[..446].iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    let codigo: Vec<u8> = mbr[..446].to_vec();
    protective_mbr_fix(&mut mbr, 1000);
    assert_eq!(&mbr[..446], &codigo[..]);
    assert_eq!(mbr[446], 0x00);
    assert_eq!(&mbr[447..450], &[0x00, 0x02, 0x00]);
    assert_eq!(mbr[450], 0xEE);
    assert_eq!(&mbr[451..454], &[0xFF, 0xFF, 0xFF]);
    assert_eq!(le32(&mbr[454..458]), 1);
    assert_eq!(le32(&mbr[458..462]), 999);
    assert!(mbr[462..510].iter().all(|&b| b == 0), "otras entradas sin vaciar");
    assert_eq!(&mbr[510..512], &[0x55, 0xAA]);
}

#[test]
fn mbr_protector_tamanos_limite() {
    let size = |d: u64| {
        let mut mbr = [0u8; SECTOR];
        protective_mbr_fix(&mut mbr, d);
        le32(&mbr[458..462])
    };
    assert_eq!(size(0), 0);
    assert_eq!(size(1), 0);
    assert_eq!(size(2), 1);
    assert_eq!(size(0x1_0000_0000), 0xFFFF_FFFF);
    assert_eq!(size(0x1_0000_0001), 0xFFFF_FFFF);
    assert_eq!(size(u64::MAX), 0xFFFF_FFFF);
}
