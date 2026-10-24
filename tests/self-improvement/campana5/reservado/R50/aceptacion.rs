//! Aceptación reservada de R50.

use gptdisk::{entry_mut, set_entry_first_lba, set_entry_last_lba, Guid, Header};
use soso_resize_core::*;

struct Mem {
    data: Vec<u8>,
}

impl Disk512 for Mem {
    fn read_sector(&self, lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), Error> {
        let off = lba as usize * SECTOR;
        if off + SECTOR > self.data.len() {
            return Err(Error::OutOfRange);
        }
        buf.copy_from_slice(&self.data[off..off + SECTOR]);
        Ok(())
    }
    fn write_sector(&mut self, lba: u64, buf: &[u8; SECTOR]) -> Result<(), Error> {
        let off = lba as usize * SECTOR;
        if off + SECTOR > self.data.len() {
            return Err(Error::OutOfRange);
        }
        self.data[off..off + SECTOR].copy_from_slice(buf);
        Ok(())
    }
}

/// Disco de 5000 sectores con una GPT primaria escrita tal cual, sin validar
/// las particiones (rangos inválidos incluidos).
fn disco(p2: (u64, u64), p3: (u64, u64), p4: (u64, u64)) -> Mem {
    let n = 5000u64;
    let mut d = Mem { data: vec![0; n as usize * SECTOR] };
    let hdr = Header {
        revision: 0x0001_0000,
        header_size: 92,
        my_lba: 1,
        alternate_lba: n - 1,
        first_usable: 34,
        last_usable: n - 34,
        disk_guid: Guid::ZERO,
        entries_lba: 2,
        num_entries: 128,
        entry_size: 128,
    };
    let mut entries = vec![0u8; hdr.entries_bytes()];
    for (idx, (a, b)) in [(GPT_ROOT, p2), (GPT_MODELS, p3), (GPT_INSTALL, p4)] {
        let e = entry_mut(&mut entries, &hdr, idx).unwrap();
        e[0] = 0x01;
        set_entry_first_lba(e, a);
        set_entry_last_lba(e, b);
    }
    let sec = hdr.render(&entries);
    d.write_sector(1, &sec).unwrap();
    for (i, chunk) in entries.chunks(SECTOR).enumerate() {
        let mut s = [0u8; SECTOR];
        s[..chunk.len()].copy_from_slice(chunk);
        d.write_sector(2 + i as u64, &s).unwrap();
    }
    d
}

fn bueno() -> Mem {
    disco((34, 999), (1000, 1099), (1100, 1149))
}

const MAL: Result<(), Error> = Err(Error::InvalidGeometry);

#[test]
fn el_caso_normal_sigue_valiendo() {
    validate_geometry(&bueno(), 16).unwrap();
    validate_geometry(&bueno(), 8).unwrap();
    validate_geometry(&bueno(), 64).unwrap();
    // con huecos entre particiones también
    validate_geometry(&disco((34, 900), (1000, 1099), (1200, 1249)), 16).unwrap();
}

#[test]
fn delta_cero_es_error() {
    assert_eq!(validate_geometry(&bueno(), 0), MAL);
}

#[test]
fn delta_no_multiplo_de_ocho_sectores_es_error() {
    for d in [1u64, 7, 9, 15, 17, 63] {
        assert_eq!(validate_geometry(&bueno(), d), MAL, "delta {d}");
    }
}

#[test]
fn delta_mayor_o_igual_que_p3_es_error() {
    // p3 tiene 100 sectores
    assert_eq!(validate_geometry(&bueno(), 96), Ok(()));
    assert_eq!(validate_geometry(&bueno(), 104), MAL);
    assert_eq!(validate_geometry(&bueno(), 9999), MAL);
    // y un delta gigante alineado a 8 no desborda ni entra en panico
    assert_eq!(validate_geometry(&bueno(), u64::MAX - 7), MAL);
}

#[test]
fn particion_con_ultimo_menor_que_primero_es_error() {
    assert_eq!(validate_geometry(&disco((34, 20), (1000, 1099), (1100, 1149)), 16), MAL);
    assert_eq!(validate_geometry(&disco((34, 999), (1099, 1000), (1100, 1149)), 16), MAL);
    assert_eq!(validate_geometry(&disco((34, 999), (1000, 1099), (1149, 1100)), 16), MAL);
}

#[test]
fn particiones_solapadas_o_desordenadas_son_error() {
    // p2 invade p3 (comparten el sector 1000)
    assert_eq!(validate_geometry(&disco((34, 1000), (1000, 1099), (1100, 1149)), 16), MAL);
    // p3 invade p4 (comparten el sector 1100)
    assert_eq!(validate_geometry(&disco((34, 999), (1000, 1100), (1100, 1149)), 16), MAL);
    // p4 antes que p3
    assert_eq!(validate_geometry(&disco((34, 999), (2000, 2099), (1100, 1149)), 16), MAL);
    // p3 contenida en p2
    assert_eq!(validate_geometry(&disco((34, 1500), (1000, 1099), (1600, 1649)), 16), MAL);
}

#[test]
fn desbordes_de_u64_son_error_y_no_panico() {
    let max = u64::MAX;
    assert_eq!(validate_geometry(&disco((34, 999), (1000, 1099), (1100, max - 5)), 16), MAL);
    assert_eq!(validate_geometry(&disco((34, 999), (1000, 1099), (1100, max)), 16), MAL);
    assert_eq!(validate_geometry(&disco((0, max), (1000, 1099), (1100, 1149)), 16), MAL);
    assert_eq!(validate_geometry(&disco((34, 999), (1000, max), (1100, 1149)), 16), MAL);
    assert_eq!(validate_geometry(&disco((34, max - 3), (max - 2, max - 1), (max, max)), 16), MAL);
}
