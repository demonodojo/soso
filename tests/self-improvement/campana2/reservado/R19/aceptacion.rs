//! Aceptación reservada de R19.

use gptdisk::{last_used, used_count, Guid, Header};

fn hdr(n: u32) -> Header {
    Header {
        revision: 0x0001_0000,
        header_size: 92,
        my_lba: 1,
        alternate_lba: 2047,
        first_usable: 34,
        last_usable: 2014,
        disk_guid: Guid([0x11; 16]),
        entries_lba: 2,
        num_entries: n,
        entry_size: 128,
    }
}

fn pon(entradas: &mut [u8], i: usize, first: u64, last: u64) {
    let e = &mut entradas[i * 128..(i + 1) * 128];
    e[0..16].copy_from_slice(&[0xAB; 16]);
    e[32..40].copy_from_slice(&first.to_le_bytes());
    e[40..48].copy_from_slice(&last.to_le_bytes());
}

#[test]
fn un_empate_lo_gana_el_menor_indice() {
    let h = hdr(4);
    let mut e = vec![0u8; 4 * 128];
    pon(&mut e, 1, 100, 500);
    pon(&mut e, 3, 200, 500);
    pon(&mut e, 2, 10, 90);
    assert_eq!(last_used(&e, &h), Some(1));
    // Tres empatadas.
    let mut e = vec![0u8; 4 * 128];
    pon(&mut e, 0, 1, 700);
    pon(&mut e, 2, 1, 700);
    pon(&mut e, 3, 1, 700);
    assert_eq!(last_used(&e, &h), Some(0));
}

#[test]
fn con_last_distintos_gana_el_mayor_y_lo_demas_igual() {
    let h = hdr(4);
    let mut e = vec![0u8; 4 * 128];
    pon(&mut e, 0, 1, 100);
    pon(&mut e, 3, 101, 900);
    pon(&mut e, 1, 50, 800);
    assert_eq!(last_used(&e, &h), Some(3));
    assert_eq!(used_count(&e, &h), 3);
    assert_eq!(last_used(&vec![0u8; 4 * 128], &h), None);
}
