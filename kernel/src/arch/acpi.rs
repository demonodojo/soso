//! Parseo mínimo de ACPI: RSDP → RSDT/XSDT → MADT, solo para enumerar los
//! APIC IDs de las CPUs. Sin crates: son tres estructuras de tamaño fijo.

use crate::mm::phys_to_virt;
use alloc::vec::Vec;

fn leer<T: Copy>(phys: u64) -> T {
    unsafe { core::ptr::read_unaligned(phys_to_virt(phys).as_ptr::<T>()) }
}

fn bytes(phys: u64, len: usize) -> &'static [u8] {
    unsafe { core::slice::from_raw_parts(phys_to_virt(phys).as_ptr::<u8>(), len) }
}

/// APIC IDs de todas las CPUs habilitadas (el primero suele ser la BSP).
pub fn apic_ids(rsdp_phys: u64) -> Vec<u32> {
    let mut ids = Vec::new();
    let rsdp = bytes(rsdp_phys, 36);
    if &rsdp[0..8] != b"RSD PTR " {
        crate::println!("acpi: RSDP inválido");
        return ids;
    }
    let revision = rsdp[15];
    // rev >= 2: XSDT (punteros de 64 bits); si no, RSDT (32 bits)
    let (sdt_phys, wide) = if revision >= 2 {
        (leer::<u64>(rsdp_phys + 24), true)
    } else {
        (leer::<u32>(rsdp_phys + 16) as u64, false)
    };

    let sdt_len = leer::<u32>(sdt_phys + 4) as u64;
    let entry_size = if wide { 8 } else { 4 };
    let n = (sdt_len - 36) / entry_size;
    for i in 0..n {
        let ptr = sdt_phys + 36 + i * entry_size;
        let table = if wide {
            leer::<u64>(ptr)
        } else {
            leer::<u32>(ptr) as u64
        };
        let sig = bytes(table, 4);
        if sig == b"APIC" {
            parse_madt(table, &mut ids);
            break;
        }
    }
    ids
}

fn parse_madt(madt: u64, ids: &mut Vec<u32>) {
    let len = leer::<u32>(madt + 4) as u64;
    // cabecera SDT (36) + local APIC addr (4) + flags (4)
    let mut off = 44u64;
    while off + 2 <= len {
        let tipo = leer::<u8>(madt + off);
        let elen = leer::<u8>(madt + off + 1) as u64;
        if elen < 2 {
            break;
        }
        match tipo {
            // Processor Local APIC: [type, len, acpi_uid, apic_id, flags]
            0 => {
                let apic_id = leer::<u8>(madt + off + 3) as u32;
                let flags = leer::<u32>(madt + off + 4);
                // bit 0 = enabled, bit 1 = online capable
                if flags & 0b11 != 0 {
                    ids.push(apic_id);
                }
            }
            // Processor Local x2APIC: [type, len, rsvd(2), x2apic_id(4), flags(4), uid(4)]
            9 => {
                let apic_id = leer::<u32>(madt + off + 4);
                let flags = leer::<u32>(madt + off + 8);
                if flags & 0b11 != 0 {
                    ids.push(apic_id);
                }
            }
            _ => {}
        }
        off += elen;
    }
}
