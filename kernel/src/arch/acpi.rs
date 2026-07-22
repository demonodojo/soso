//! Parseo mínimo de ACPI: RSDP → RSDT/XSDT → MADT + MCFG.
//!
//! MADT: APIC IDs (SMP), IOAPIC e Interrupt Source Override.
//! MCFG: base ECAM y rango de buses PCI.

use crate::mm::phys_to_virt;
use alloc::vec::Vec;
use spin::Once;

fn leer<T: Copy>(phys: u64) -> T {
    unsafe { core::ptr::read_unaligned(phys_to_virt(phys).as_ptr::<T>()) }
}

fn bytes(phys: u64, len: usize) -> &'static [u8] {
    unsafe { core::slice::from_raw_parts(phys_to_virt(phys).as_ptr::<u8>(), len) }
}

#[derive(Clone, Copy, Debug)]
pub struct IoApicInfo {
    pub id: u8,
    pub address: u32,
    pub gsi_base: u32,
}

/// Interrupt Source Override (MADT tipo 2): IRQ legacy → GSI.
#[derive(Clone, Copy, Debug)]
pub struct Iso {
    pub irq: u8,
    pub gsi: u32,
    pub flags: u16,
}

pub struct MadtInfo {
    pub apic_ids: Vec<u32>,
    pub ioapics: Vec<IoApicInfo>,
    pub overrides: Vec<Iso>,
}

/// Primera asignación MCFG (segmento 0).
#[derive(Clone, Copy, Debug)]
pub struct McfgAllocation {
    pub base: u64,
    pub bus_start: u8,
    pub bus_end: u8,
}

static MADT: Once<MadtInfo> = Once::new();
static MCFG: Once<Option<McfgAllocation>> = Once::new();

/// Recorre RSDT/XSDT y cachea MADT + MCFG. Idempotente.
pub fn init(rsdp_phys: u64) {
    MADT.call_once(|| {
        let mut info = MadtInfo {
            apic_ids: Vec::new(),
            ioapics: Vec::new(),
            overrides: Vec::new(),
        };
        if let Some(madt) = find_table(rsdp_phys, b"APIC") {
            parse_madt(madt, &mut info);
        } else {
            crate::println!("acpi: MADT no encontrado");
        }
        info
    });
    MCFG.call_once(|| {
        let Some(mcfg) = find_table(rsdp_phys, b"MCFG") else {
            crate::println!("acpi: MCFG no encontrado; ECAM por fallback");
            return None;
        };
        parse_mcfg(mcfg)
    });
    if let Some(m) = mcfg() {
        crate::println!(
            "acpi: MCFG ecam={:#x} buses {}-{}",
            m.base,
            m.bus_start,
            m.bus_end
        );
    }
    let madt = madt();
    crate::println!(
        "acpi: MADT {} CPUs, {} IOAPIC, {} ISO",
        madt.apic_ids.len(),
        madt.ioapics.len(),
        madt.overrides.len()
    );
}

pub fn madt() -> &'static MadtInfo {
    MADT.get().expect("acpi::init no llamado")
}

pub fn mcfg() -> Option<&'static McfgAllocation> {
    MCFG.get().and_then(|o| o.as_ref())
}

/// Compat: APIC IDs (vacío si aún no hay init).
pub fn apic_ids(rsdp_phys: u64) -> Vec<u32> {
    if MADT.get().is_none() {
        init(rsdp_phys);
    }
    madt().apic_ids.clone()
}

fn find_table(rsdp_phys: u64, sig: &[u8; 4]) -> Option<u64> {
    let rsdp = bytes(rsdp_phys, 36);
    if &rsdp[0..8] != b"RSD PTR " {
        crate::println!("acpi: RSDP inválido");
        return None;
    }
    let revision = rsdp[15];
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
        if bytes(table, 4) == sig {
            return Some(table);
        }
    }
    None
}

fn parse_madt(madt: u64, info: &mut MadtInfo) {
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
            // Processor Local APIC
            0 if elen >= 8 => {
                let apic_id = leer::<u8>(madt + off + 3) as u32;
                let flags = leer::<u32>(madt + off + 4);
                if flags & 0b11 != 0 {
                    info.apic_ids.push(apic_id);
                }
            }
            // I/O APIC
            1 if elen >= 12 => {
                info.ioapics.push(IoApicInfo {
                    id: leer::<u8>(madt + off + 2),
                    address: leer::<u32>(madt + off + 4),
                    gsi_base: leer::<u32>(madt + off + 8),
                });
            }
            // Interrupt Source Override
            2 if elen >= 10 => {
                info.overrides.push(Iso {
                    irq: leer::<u8>(madt + off + 3),
                    gsi: leer::<u32>(madt + off + 4),
                    flags: leer::<u16>(madt + off + 8),
                });
            }
            // Processor Local x2APIC
            9 if elen >= 16 => {
                let apic_id = leer::<u32>(madt + off + 4);
                let flags = leer::<u32>(madt + off + 8);
                if flags & 0b11 != 0 {
                    info.apic_ids.push(apic_id);
                }
            }
            _ => {}
        }
        off += elen;
    }
}

fn parse_mcfg(mcfg: u64) -> Option<McfgAllocation> {
    let len = leer::<u32>(mcfg + 4) as u64;
    // cabecera SDT (36) + reserved (8) = 44; cada allocation = 16
    if len < 44 + 16 {
        return None;
    }
    let base = leer::<u64>(mcfg + 44);
    let bus_start = leer::<u8>(mcfg + 44 + 10);
    let bus_end = leer::<u8>(mcfg + 44 + 11);
    Some(McfgAllocation {
        base,
        bus_start,
        bus_end,
    })
}
