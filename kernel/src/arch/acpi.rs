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
    #[allow(dead_code)]
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
static RSDP: Once<u64> = Once::new();
static ENERGIA: Once<Option<Energia>> = Once::new();

/// Registro fijo del FADT: PM1 o RESET. `space` 0 = memoria, 1 = E/S.
#[derive(Clone, Copy)]
pub struct RegAcpi {
    pub space: u8,
    pub width: u8,
    pub addr: u64,
}

/// Lo que hace falta para S5 y para el reset ACPI. `None` si no hay FADT.
#[derive(Clone, Copy)]
pub struct Energia {
    pub smi_cmd: u32,
    pub acpi_enable: u8,
    pub pm1a: Option<RegAcpi>,
    pub pm1b: Option<RegAcpi>,
    pub slp_typa: Option<u8>,
    pub slp_typb: Option<u8>,
    pub reset_reg: Option<RegAcpi>,
    pub reset_val: u8,
}

/// Recorre RSDT/XSDT y cachea MADT + MCFG. Idempotente.
pub fn init(rsdp_phys: u64) {
    RSDP.call_once(|| rsdp_phys);
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
    ENERGIA.call_once(|| parse_energia(rsdp_phys));
    anunciar_energia();
}

pub fn madt() -> &'static MadtInfo {
    MADT.get().expect("acpi::init no llamado")
}

pub fn mcfg() -> Option<&'static McfgAllocation> {
    MCFG.get().and_then(|o| o.as_ref())
}

/// FADT ya parseado. `None` si `init` no corrió o el firmware no trae FACP.
pub fn energia() -> Option<&'static Energia> {
    ENERGIA.get().and_then(|o| o.as_ref())
}

/// Compat: APIC IDs (vacío si aún no hay init).
pub fn apic_ids(rsdp_phys: u64) -> Vec<u32> {
    if MADT.get().is_none() {
        init(rsdp_phys);
    }
    madt().apic_ids.clone()
}

/// Contenido de una tabla ACPI cualquiera, para los consumidores que no
/// necesitan un parser propio aquí (IVRS lo parsea `soso-hw`).
/// Requiere `init` previo: sin RSDP no hay dónde buscar.
pub fn tabla_bytes(sig: &[u8; 4]) -> Option<&'static [u8]> {
    let rsdp_phys = *RSDP.get()?;
    let table = find_table(rsdp_phys, sig)?;
    let len = leer::<u32>(table + 4) as usize;
    if len < 36 {
        return None;
    }
    Some(bytes(table, len))
}

fn find_table(rsdp_phys: u64, sig: &[u8; 4]) -> Option<u64> {
    let mut found = None;
    let ok = visitar_tablas(rsdp_phys, sig, |table| {
        found = Some(table);
        true
    });
    if !ok {
        crate::println!("acpi: RSDP inválido");
    }
    found
}

/// `true` si el RSDP es válido. `f` devuelve `true` para dejar de recorrer.
fn visitar_tablas(rsdp_phys: u64, sig: &[u8; 4], mut f: impl FnMut(u64) -> bool) -> bool {
    let rsdp = bytes(rsdp_phys, 36);
    if &rsdp[0..8] != b"RSD PTR " {
        return false;
    }
    let revision = rsdp[15];
    let (sdt_phys, wide) = if revision >= 2 {
        (leer::<u64>(rsdp_phys + 24), true)
    } else {
        (leer::<u32>(rsdp_phys + 16) as u64, false)
    };
    let sdt_len = leer::<u32>(sdt_phys + 4) as u64;
    if !(36..1024 * 1024).contains(&sdt_len) {
        return true;
    }
    let entry_size = if wide { 8 } else { 4 };
    let n = (sdt_len - 36) / entry_size;
    for i in 0..n {
        let ptr = sdt_phys + 36 + i * entry_size;
        let table = if wide {
            leer::<u64>(ptr)
        } else {
            leer::<u32>(ptr) as u64
        };
        if table != 0 && bytes(table, 4) == sig && f(table) {
            break;
        }
    }
    true
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

fn anunciar_energia() {
    let Some(e) = energia() else {
        crate::println!("acpi: FADT no encontrado; halt no corta la alimentación");
        return;
    };
    match (e.pm1a, e.slp_typa) {
        (Some(pm), Some(typ)) => {
            let b = e.slp_typb.unwrap_or(typ);
            match e.reset_reg {
                Some(r) => crate::println!(
                    "acpi: apagado {} S5={typ}/{b} reset={}",
                    fmt_reg(pm),
                    fmt_reg(r)
                ),
                None => crate::println!("acpi: apagado {} S5={typ}/{b}", fmt_reg(pm)),
            }
        }
        _ => crate::println!("acpi: FADT sin _S5_; halt no corta la alimentación"),
    }
}

fn fmt_reg(r: RegAcpi) -> alloc::string::String {
    let clase = if r.space == 0 { "mem" } else { "io" };
    alloc::format!("{clase}:{:#x}", r.addr)
}

fn parse_energia(rsdp: u64) -> Option<Energia> {
    let fadt = find_table(rsdp, b"FACP")?;
    if bytes(fadt, 4) != b"FACP" {
        return None;
    }
    let len = leer::<u32>(fadt + 4) as usize;
    // ACPI 1.0 llega hasta Flags (offset 112). PM1a_CNT está en 64.
    if !(68..1024 * 1024).contains(&len) {
        return None;
    }
    let smi_cmd = if len >= 52 {
        leer::<u32>(fadt + 48)
    } else {
        0
    };
    let acpi_enable = if len >= 53 { leer::<u8>(fadt + 52) } else { 0 };
    let mut pm1a = reg_io(leer::<u32>(fadt + 64));
    let mut pm1b = if len >= 72 {
        reg_io(leer::<u32>(fadt + 68))
    } else {
        None
    };
    // Los X_ del FADT mandan cuando la dirección no es cero.
    if len >= 184 {
        if let Some(r) = reg_gas(fadt + 172, 16) {
            pm1a = Some(r);
        }
    }
    if len >= 196 {
        if let Some(r) = reg_gas(fadt + 184, 16) {
            pm1b = Some(r);
        }
    }
    let mut reset_reg = None;
    let mut reset_val = 0u8;
    if len >= 129 {
        let flags = leer::<u32>(fadt + 112);
        if flags & (1 << 10) != 0 {
            reset_val = leer::<u8>(fadt + 128);
            reset_reg = reg_gas(fadt + 116, 8);
        }
    }
    let (slp_typa, slp_typb) = buscar_s5(rsdp, dsdt_phys(fadt, len));
    Some(Energia {
        smi_cmd,
        acpi_enable,
        pm1a,
        pm1b,
        slp_typa,
        slp_typb,
        reset_reg,
        reset_val,
    })
}

fn reg_io(port: u32) -> Option<RegAcpi> {
    if port == 0 || port > 0xffff {
        return None;
    }
    Some(RegAcpi {
        space: 1,
        width: 16,
        addr: port as u64,
    })
}

/// Generic Address Structure de 12 bytes. `ancho` si el bit width del GAS no vale.
fn reg_gas(phys: u64, ancho: u8) -> Option<RegAcpi> {
    let space = leer::<u8>(phys);
    let bit_width = leer::<u8>(phys + 1);
    let addr = leer::<u64>(phys + 4);
    if addr == 0 || (space != 0 && space != 1) {
        return None;
    }
    if space == 1 && addr > 0xffff {
        return None;
    }
    // PM1 pide al menos 16 bits: SLP_EN es el bit 13. Un GAS que diga 8
    // no puede expresar el apagado.
    let width = match bit_width {
        8 | 16 | 32 if bit_width >= ancho => bit_width,
        _ => ancho,
    };
    Some(RegAcpi { space, width, addr })
}

fn dsdt_phys(fadt: u64, len: usize) -> Option<u64> {
    if len >= 148 {
        let x = leer::<u64>(fadt + 140);
        if x != 0 {
            return Some(x);
        }
    }
    if len >= 44 {
        let d = leer::<u32>(fadt + 40);
        if d != 0 {
            return Some(d as u64);
        }
    }
    None
}

fn buscar_s5(rsdp: u64, dsdt: Option<u64>) -> (Option<u8>, Option<u8>) {
    if let Some(p) = dsdt {
        if let Some(v) = slp_s5_en(p, b"DSDT") {
            return (Some(v.0), Some(v.1));
        }
    }
    let mut hallado = None;
    let _ = visitar_tablas(rsdp, b"SSDT", |p| {
        if hallado.is_some() {
            return true;
        }
        if let Some(v) = slp_s5_en(p, b"SSDT") {
            hallado = Some(v);
            return true;
        }
        false
    });
    match hallado {
        Some((a, b)) => (Some(a), Some(b)),
        None => (None, None),
    }
}

fn slp_s5_en(phys: u64, sig: &[u8; 4]) -> Option<(u8, u8)> {
    if bytes(phys, 4) != sig {
        return None;
    }
    let len = leer::<u32>(phys + 4) as usize;
    if !(36..=1024 * 1024).contains(&len) {
        return None;
    }
    slp_s5(bytes(phys, len))
}

/// `_S5_` del DSDT/SSDT: `NameOp` + paquete AML con SLP_TYPa y SLP_TYPb.
fn slp_s5(aml: &[u8]) -> Option<(u8, u8)> {
    let mut i = 0;
    while i + 5 < aml.len() {
        if &aml[i..i + 4] == b"_S5_" {
            let name_op = (i >= 1 && aml[i - 1] == 0x08)
                || (i >= 2 && aml[i - 2] == 0x08 && aml[i - 1] == b'\\');
            if name_op && aml[i + 4] == 0x12 {
                if let Some(v) = enteros_paquete(aml, i + 4) {
                    return Some(v);
                }
            }
        }
        i += 1;
    }
    None
}

fn enteros_paquete(aml: &[u8], p: usize) -> Option<(u8, u8)> {
    let mut j = p + 1;
    let extra = (*aml.get(j)? >> 6) as usize;
    j = j.checked_add(1 + extra)?;
    if j >= aml.len() {
        return None;
    }
    let n = aml[j] as usize;
    j += 1;
    if n == 0 {
        return None;
    }
    let (a, j) = aml_entero(aml, j)?;
    let b = if n >= 2 {
        aml_entero(aml, j).map(|(v, _)| v).unwrap_or(a)
    } else {
        a
    };
    Some(((a as u8) & 7, (b as u8) & 7))
}

fn aml_entero(aml: &[u8], j: usize) -> Option<(u64, usize)> {
    let b = *aml.get(j)?;
    match b {
        0x00 => Some((0, j + 1)),
        0x01 => Some((1, j + 1)),
        0xff => Some((u64::MAX, j + 1)),
        0x0a => Some((*aml.get(j + 1)? as u64, j + 2)),
        0x0b => {
            let lo = *aml.get(j + 1)?;
            let hi = *aml.get(j + 2)?;
            Some((u16::from_le_bytes([lo, hi]) as u64, j + 3))
        }
        0x0c => {
            let mut buf = [0u8; 4];
            buf.copy_from_slice(aml.get(j + 1..j + 5)?);
            Some((u32::from_le_bytes(buf) as u64, j + 5))
        }
        0x0e => {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(aml.get(j + 1..j + 9)?);
            Some((u64::from_le_bytes(buf), j + 9))
        }
        _ => None,
    }
}
