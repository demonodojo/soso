//! Realtek RRTL8111/8168/8211/8411 gigabit (familia r8169) — **sondeo**.
//!
//! Todavía no mueve tráfico: identifica el chip y deja en la consola lo que
//! decide el resto del bring-up. Dos cosas hay que leer del silicio antes de
//! escribir una línea de anillos:
//!
//! 1. **La versión de MAC no es la revisión PCI.** El r8169 de Linux la saca de
//!    `TxConfig` (0x40) enmascarando `0x7cf00000`; la familia tiene decenas de
//!    variantes con secuencias de arranque distintas y la revisión no las
//!    distingue.
//! 2. **Los registros no están en BAR0.** En estas PCIe, BAR0 es E/S y el
//!    espacio MMIO es **BAR2** (4 KiB). `pci::bar_info(.., 0)` devuelve None
//!    para un BAR de E/S, así que quien pida el 0 se queda sin nada.

use crate::drivers::pci;
use crate::mm;
use crate::println;

const VENDOR_REALTEK: u16 = 0x10ec;
/// 8168 cubre RTL8111/8168/8211/8411; 8161 y 8136 son de la misma familia.
const DEVICE_IDS: &[u16] = &[0x8168, 0x8161, 0x8136];

/// Índice de BAR con los registros (BAR0 es E/S en las PCIe).
const BAR_MMIO: u8 = 2;

const REG_IDR0: u32 = 0x00;
const REG_CMD: u32 = 0x37;
const REG_TXCONFIG: u32 = 0x40;
const REG_RXCONFIG: u32 = 0x44;
const REG_PHYSTATUS: u32 = 0x6c;

/// Máscara de versión de MAC que usa el r8169 moderno.
const MAC_VER_MASK: u32 = 0x7cf0_0000;
/// La que usaban los kernels antiguos; se imprime para poder cotejar tablas.
const MAC_VER_MASK_VIEJA: u32 = 0x7c80_0000;

const PHY_LINK_OK: u8 = 0x02;
const PHY_FULL_DUP: u8 = 0x01;
const PHY_10: u8 = 0x04;
const PHY_100: u8 = 0x08;
const PHY_1000: u8 = 0x10;

fn r8(base: u64, off: u32) -> u8 {
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(base + off as u64).as_ptr()) }
}

fn r32(base: u64, off: u32) -> u32 {
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(base + off as u64).as_ptr()) }
}

fn velocidad(phy: u8) -> &'static str {
    if phy & PHY_1000 != 0 {
        "1000M"
    } else if phy & PHY_100 != 0 {
        "100M"
    } else if phy & PHY_10 != 0 {
        "10M"
    } else {
        "?"
    }
}

/// Identifica la tarjeta y vuelca lo que hace falta para el bring-up. No la toca.
pub fn probe() {
    let devs = pci::devices();
    let Some(dev) = devs
        .iter()
        .find(|d| d.vendor_id == VENDOR_REALTEK && DEVICE_IDS.contains(&d.device_id))
    else {
        return;
    };

    let rev = pci::read8(dev.bus, dev.device, dev.function, 0x08);
    println!(
        "rtl8169: {:04x}:{:04x} rev {:02x} en {:02x}:{:02x}.{}",
        dev.vendor_id, dev.device_id, rev, dev.bus, dev.device, dev.function
    );

    // MEMORY_SPACE + BUS_MASTER: los registros son MMIO y el chip hace DMA.
    let mut cmd = pci::read16(dev.bus, dev.device, dev.function, 0x04);
    cmd |= 0x6;
    pci::write16(dev.bus, dev.device, dev.function, 0x04, cmd);

    let Some((bar, bar_size)) = pci::bar_info(dev.bus, dev.device, dev.function, BAR_MMIO) else {
        println!("rtl8169: sin BAR{BAR_MMIO} de memoria — no hay dónde leer los registros");
        return;
    };
    mm::ensure_mmio_mapped(bar, bar_size.max(0x1000));
    println!("rtl8169: bar{BAR_MMIO} {bar:#x} ({} B)", bar_size);

    let txcfg = r32(bar, REG_TXCONFIG);
    println!(
        "rtl8169: txconfig {:#010x} → mac_version {:#010x} (máscara vieja {:#010x})",
        txcfg,
        txcfg & MAC_VER_MASK,
        txcfg & MAC_VER_MASK_VIEJA
    );

    let mut mac = [0u8; 6];
    for (i, b) in mac.iter_mut().enumerate() {
        *b = r8(bar, REG_IDR0 + i as u32);
    }
    println!(
        "rtl8169: mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    let phy = r8(bar, REG_PHYSTATUS);
    println!(
        "rtl8169: phystatus {:#04x} → enlace {} {} {}",
        phy,
        if phy & PHY_LINK_OK != 0 { "UP" } else { "DOWN" },
        velocidad(phy),
        if phy & PHY_FULL_DUP != 0 { "full" } else { "half" }
    );

    println!(
        "rtl8169: cmd {:#04x} rxconfig {:#010x}",
        r8(bar, REG_CMD),
        r32(bar, REG_RXCONFIG)
    );

    match pci::find_msix(dev.bus, dev.device, dev.function) {
        Some(m) => println!("rtl8169: MSI-X disponible, {} entradas", m.table_size),
        None => println!("rtl8169: sin MSI-X; habría que ir por INTx o polled"),
    }
}
