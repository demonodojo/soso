//! Driver Intel e1000e — 82574/82540 (QEMU) e I217/I219 (MDIO copper).
//!
//! Rings RX/TX en DMA; MSI-X si existe, si no INTx vía IOAPIC.

use crate::arch::{apic, ioapic, irq};
use crate::drivers::{dma, pci};
use crate::mm;
use crate::println;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use spin::{Mutex, Once};

const VENDOR_INTEL: u16 = 0x8086;
/// QEMU emulados: fallback slirp 10.0.2.x permitido.
const DEVICE_IDS_QEMU: &[u16] = &[0x10d3, 0x100e];
/// Placa: I217/I219 y familia PCH — PHY copper vía MDIC, sin slirp.
const DEVICE_IDS_ICH: &[u16] = &[
    0x10f5, 0x10a4, 0x15fc, 0x15f8, 0x15b8, 0x15b7, 0x15d8, 0x0d4f, 0x15e3,
];

const REG_CTRL: u32 = 0x0000;
const REG_STATUS: u32 = 0x0008;
const REG_MDIC: u32 = 0x0020;
const STATUS_LU: u32 = 1 << 1;
const MDIC_PHY: u32 = 1;
const MDIC_READY: u32 = 1 << 28;
const MDIC_ERROR: u32 = 1 << 30;
const MDIC_OP_READ: u32 = 1 << 27;
const MDIC_OP_WRITE: u32 = 1 << 26;

const MII_BMCR: u32 = 0;
const MII_BMSR: u32 = 1;
const MII_ADVERTISE: u32 = 4;
const MII_CTRL1000: u32 = 9;
const BMCR_RESET: u16 = 0x8000;
const BMCR_ANENABLE: u16 = 0x1000;
const BMCR_ANRESTART: u16 = 0x0200;
const BMCR_POWERDOWN: u16 = 0x0800;
const REG_EECD: u32 = 0x0010;
const REG_EERD: u32 = 0x0014;
const REG_ICR: u32 = 0x00c0;
const REG_IMS: u32 = 0x00d0;
const REG_IMC: u32 = 0x00d8;
const REG_RCTL: u32 = 0x0100;
const REG_TCTL: u32 = 0x0400;
const REG_TIPG: u32 = 0x0410;
const REG_RDBAL: u32 = 0x2800;
const REG_RDBAH: u32 = 0x2804;
const REG_RDLEN: u32 = 0x2808;
const REG_RDH: u32 = 0x2810;
const REG_RDT: u32 = 0x2818;
const REG_TDBAL: u32 = 0x3800;
const REG_TDBAH: u32 = 0x3804;
const REG_TDLEN: u32 = 0x3808;
const REG_TDH: u32 = 0x3810;
const REG_TDT: u32 = 0x3818;
const REG_RAL: u32 = 0x5400;
const REG_RAH: u32 = 0x5404;
const REG_MTA: u32 = 0x5200;

const CTRL_RST: u32 = 1 << 26;
const CTRL_SLU: u32 = 1 << 6;
const CTRL_ASDE: u32 = 1 << 5;

const RCTL_EN: u32 = 1 << 1;
const RCTL_SBP: u32 = 1 << 2;
const RCTL_UPE: u32 = 1 << 3;
const RCTL_MPE: u32 = 1 << 4;
const RCTL_LPE: u32 = 1 << 5;
const RCTL_BAM: u32 = 1 << 15;
const RCTL_BSIZE_2048: u32 = 0;
const RCTL_SECRC: u32 = 1 << 26;

const TCTL_EN: u32 = 1 << 1;
const TCTL_PSP: u32 = 1 << 3;

const IMS_TXDW: u32 = 1 << 0;
const IMS_TXQE: u32 = 1 << 1;
const IMS_LSC: u32 = 1 << 2;
const IMS_RXO: u32 = 1 << 6;
const IMS_RXT0: u32 = 1 << 7;

const RX_DD: u8 = 1 << 0;
const RX_EOP: u8 = 1 << 1;
const TX_DD: u8 = 1 << 0;

const RX_DESC: usize = 32;
const TX_DESC: usize = 32;
const BUF_LEN: usize = 2048;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct RxDesc {
    addr: u64,
    length: u16,
    checksum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct TxDesc {
    addr: u64,
    length: u16,
    cso: u8,
    cmd: u8,
    status: u8,
    css: u8,
    special: u16,
}

struct Nic {
    mmio: u64,
    mac: [u8; 6],
    device_id: u16,
    qemu: bool,
    link_up: bool,
    rx_phys: dma::PhysAddr,
    tx_phys: dma::PhysAddr,
    rx_buf_phys: dma::PhysAddr,
    tx_buf_phys: dma::PhysAddr,
    rx_tail: u16,
    tx_tail: u16,
}

static NIC: Once<Mutex<Nic>> = Once::new();
static PRESENT: AtomicBool = AtomicBool::new(false);
static LAST_LINK_POLL: AtomicU64 = AtomicU64::new(0);

fn rr(base: u64, off: u32) -> u32 {
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(base + off as u64).as_ptr()) }
}

fn rw(base: u64, off: u32, v: u32) {
    unsafe {
        core::ptr::write_volatile(mm::phys_to_virt(base + off as u64).as_mut_ptr(), v);
    }
}

/// Igual que `virtio_net::net_irq_handler`: sólo ack y agendar. Ejecutar
/// `net::poll()` desde aquí (IF=0) reentraba en PROCS/RX/TX/VFS/heap sostenidos
/// por una syscall con IF=1, y colgaba el core entero (avería del 2026-08-01).
fn e1000_irq() {
    if let Some(n) = NIC.get() {
        if let Some(nic) = n.try_lock() {
            let _ = rr(nic.mmio, REG_ICR); // ack
        }
    }
    crate::net::marcar_trabajo_pendiente();
}

pub fn present() -> bool {
    PRESENT.load(Ordering::Acquire)
}

pub fn is_qemu_emulated() -> bool {
    NIC.get()
        .map(|n| n.lock().qemu)
        .unwrap_or(false)
}

fn device_supported(id: u16) -> bool {
    DEVICE_IDS_QEMU.contains(&id) || DEVICE_IDS_ICH.contains(&id)
}

fn mdic_wait(bar: u64) -> bool {
    for _ in 0..20_000 {
        if rr(bar, REG_MDIC) & MDIC_READY != 0 {
            return rr(bar, REG_MDIC) & MDIC_ERROR == 0;
        }
        core::hint::spin_loop();
    }
    false
}

fn mdic_read(bar: u64, reg: u32) -> u16 {
    let cmd = (MDIC_PHY << 21) | ((reg & 0x1f) << 16) | MDIC_OP_READ;
    rw(bar, REG_MDIC, cmd);
    if !mdic_wait(bar) {
        return 0;
    }
    (rr(bar, REG_MDIC) & 0xffff) as u16
}

fn mdic_write(bar: u64, reg: u32, val: u16) {
    let cmd = (MDIC_PHY << 21) | ((reg & 0x1f) << 16) | MDIC_OP_WRITE | u32::from(val);
    rw(bar, REG_MDIC, cmd);
    let _ = mdic_wait(bar);
}

fn ich_phy_bringup(bar: u64) {
    let bmcr = mdic_read(bar, MII_BMCR);
    if bmcr != 0 {
        mdic_write(bar, MII_BMCR, bmcr & !BMCR_POWERDOWN);
    }
    mdic_write(bar, MII_BMCR, BMCR_RESET);
    for _ in 0..80_000 {
        if mdic_read(bar, MII_BMCR) & BMCR_RESET == 0 {
            break;
        }
        core::hint::spin_loop();
    }
    mdic_write(bar, MII_ADVERTISE, 0x01e1);
    mdic_write(bar, MII_CTRL1000, 0x0200);
    mdic_write(bar, MII_BMCR, BMCR_ANENABLE | BMCR_ANRESTART);
}

fn read_link(bar: u64, ich: bool) -> (bool, u32, u16) {
    let status = rr(bar, REG_STATUS);
    let lu = status & STATUS_LU != 0;
    if ich {
        let bmsr = mdic_read(bar, MII_BMSR);
        return (lu && bmsr != 0, status, bmsr);
    }
    (lu || true, status, 0)
}

/// Sondeo periódico del enlace (I217/I219); devuelve true si pasó de DOWN a UP.
pub fn poll_link() -> bool {
    let Some(nic_m) = NIC.get() else {
        return false;
    };
    let mut nic = nic_m.lock();
    if nic.qemu {
        return false;
    }
    let now = crate::arch::pit::uptime_ms();
    let last = LAST_LINK_POLL.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 1000 {
        return false;
    }
    LAST_LINK_POLL.store(now, Ordering::Relaxed);
    let (up, status, bmsr) = read_link(nic.mmio, true);
    let was = nic.link_up;
    nic.link_up = up;
    if up && !was {
        println!(
            "e1000e: enlace UP status={status:#010x} bmsr={bmsr:#06x}"
        );
        return true;
    }
    false
}

/// Inicializa la NIC si hay un e1000e; devuelve la MAC.
pub fn init() -> Option<[u8; 6]> {
    let devs = pci::devices();
    let dev = devs
        .iter()
        .find(|d| d.vendor_id == VENDOR_INTEL && device_supported(d.device_id))?;

    println!(
        "e1000e: {:04x}:{:04x} en {:02x}:{:02x}.{}",
        dev.vendor_id, dev.device_id, dev.bus, dev.device, dev.function
    );

    let mut cmd = pci::read16(dev.bus, dev.device, dev.function, 0x04);
    cmd |= 0x6;
    pci::write16(dev.bus, dev.device, dev.function, 0x04, cmd);

    let (bar, bar_size) = (dev.bar0, dev.bar0_size);
    if bar == 0 || bar_size == 0 {
        return None;
    }
    mm::ensure_mmio_mapped(bar, bar_size.max(0x20000));

    // MAC de RAL antes del reset (QEMU ya la programa).
    let mac_before = mac_from_ral(bar);

    // Reset
    rw(bar, REG_CTRL, rr(bar, REG_CTRL) | CTRL_RST);
    for _ in 0..100_000 {
        if rr(bar, REG_CTRL) & CTRL_RST == 0 {
            break;
        }
        core::hint::spin_loop();
    }
    rw(bar, REG_IMC, 0xffff_ffff);
    let _ = rr(bar, REG_ICR);

    let mut mac = read_mac(bar, dev.device_id);
    if !mac_ok(&mac) {
        mac = mac_before;
    }
    if !mac_ok(&mac) {
        mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    }
    println!(
        "e1000e: mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    let qemu = DEVICE_IDS_QEMU.contains(&dev.device_id);
    if qemu {
        rw(bar, REG_CTRL, rr(bar, REG_CTRL) | CTRL_SLU | CTRL_ASDE);
    } else {
        ich_phy_bringup(bar);
        for _ in 0..200_000 {
            core::hint::spin_loop();
        }
    }

    // Clear MTA
    for i in 0..128 {
        rw(bar, REG_MTA + i * 4, 0);
    }

    // Set MAC in RAL/RAH
    let ral = u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]);
    let rah = u32::from_le_bytes([mac[4], mac[5], 0, 0]) | (1 << 31);
    rw(bar, REG_RAL, ral);
    rw(bar, REG_RAH, rah);

    let rx_pages = pages_for(RX_DESC * core::mem::size_of::<RxDesc>());
    let tx_pages = pages_for(TX_DESC * core::mem::size_of::<TxDesc>());
    let rx_buf_pages = pages_for(RX_DESC * BUF_LEN);
    let tx_buf_pages = pages_for(TX_DESC * BUF_LEN);
    let (rx_phys, rx_ptr) = dma::alloc_zeroed(rx_pages);
    let (tx_phys, tx_ptr) = dma::alloc_zeroed(tx_pages);
    let (rx_buf_phys, _) = dma::alloc_zeroed(rx_buf_pages);
    let (tx_buf_phys, _) = dma::alloc_zeroed(tx_buf_pages);

    unsafe {
        let rx = core::slice::from_raw_parts_mut(rx_ptr.as_ptr() as *mut RxDesc, RX_DESC);
        for (i, d) in rx.iter_mut().enumerate() {
            d.addr = (rx_buf_phys + i * BUF_LEN) as u64;
            d.length = 0;
            d.status = 0;
        }
        let tx = core::slice::from_raw_parts_mut(tx_ptr.as_ptr() as *mut TxDesc, TX_DESC);
        for d in tx.iter_mut() {
            *d = core::mem::zeroed();
        }
    }

    rw(bar, REG_RDBAL, rx_phys as u32);
    rw(bar, REG_RDBAH, ((rx_phys as u64) >> 32) as u32);
    rw(bar, REG_RDLEN, (RX_DESC * 16) as u32);
    rw(bar, REG_RDH, 0);
    rw(bar, REG_RDT, (RX_DESC - 1) as u32);

    rw(bar, REG_TDBAL, tx_phys as u32);
    rw(bar, REG_TDBAH, ((tx_phys as u64) >> 32) as u32);
    rw(bar, REG_TDLEN, (TX_DESC * 16) as u32);
    rw(bar, REG_TDH, 0);
    rw(bar, REG_TDT, 0);

    rw(
        bar,
        REG_RCTL,
        RCTL_EN | RCTL_SBP | RCTL_UPE | RCTL_MPE | RCTL_LPE | RCTL_BAM | RCTL_BSIZE_2048 | RCTL_SECRC,
    );
    rw(bar, REG_TCTL, TCTL_EN | TCTL_PSP | (0x10 << 4) | (0x40 << 12));
    rw(bar, REG_TIPG, 0x0060_200a);

    // Interrupts
    if let Some(msix) = pci::find_msix(dev.bus, dev.device, dev.function) {
        if let Some(vec) = irq::allocate(e1000_irq) {
            if pci::msix_setup(&msix, 0, vec, pci::msix_default_dest()).is_ok() {
                println!("e1000e: MSI-X vector={vec:#x}");
            }
        }
    } else if let Some(vec) = irq::allocate(e1000_irq) {
        // INTx: leer Interrupt Line (offset 0x3C) como IRQ ISA aproximado.
        let line = pci::read8(dev.bus, dev.device, dev.function, 0x3c);
        let dest = apic::id();
        if ioapic::route_isa(line, vec, dest).is_ok() {
            println!("e1000e: INTx IRQ{line} → vector={vec:#x}");
        } else {
            println!("e1000e: INTx route falló; solo polled");
        }
    }
    rw(bar, REG_IMS, IMS_TXDW | IMS_TXQE | IMS_LSC | IMS_RXO | IMS_RXT0);

    let (link_up, status, bmsr) = read_link(bar, !qemu);
    if !qemu {
        println!(
            "e1000e: ich phy status={status:#010x} bmsr={bmsr:#06x} → {}",
            if link_up { "UP" } else { "DOWN" }
        );
    }

    let nic = Nic {
        mmio: bar,
        mac,
        device_id: dev.device_id,
        qemu,
        link_up: qemu || link_up,
        rx_phys,
        tx_phys,
        rx_buf_phys,
        tx_buf_phys,
        rx_tail: 0,
        tx_tail: 0,
    };
    let _ = (nic.rx_buf_phys, nic.tx_buf_phys, nic.rx_phys, nic.tx_phys);
    NIC.call_once(|| Mutex::new(nic));
    PRESENT.store(true, Ordering::Release);
    let _ = REG_EECD;
    let _ = REG_STATUS;
    Some(mac)
}

fn pages_for(bytes: usize) -> usize {
    bytes.div_ceil(dma::PAGE_SIZE).max(1)
}

fn mac_ok(mac: &[u8; 6]) -> bool {
    mac != &[0; 6] && mac[0] & 1 == 0 && mac[3..] != [0, 0, 0]
}

fn mac_from_ral(bar: u64) -> [u8; 6] {
    let ral = rr(bar, REG_RAL);
    let rah = rr(bar, REG_RAH);
    [
        ral as u8,
        (ral >> 8) as u8,
        (ral >> 16) as u8,
        (ral >> 24) as u8,
        rah as u8,
        (rah >> 8) as u8,
    ]
}

fn read_mac(bar: u64, device_id: u16) -> [u8; 6] {
    // Intentar EEPROM; si falla, RAL/RAH (QEMU los rellena).
    if device_id == 0x10d3 || device_id == 0x100e {
        // EERD: bit 0 start, address << 2 (82540) o << 8 (82574)
        let shift = if device_id == 0x10d3 { 8 } else { 2 };
        let mut mac = [0u8; 6];
        for i in 0..3u32 {
            rw(bar, REG_EERD, (i << shift) | 1);
            let mut v = 0u32;
            for _ in 0..10_000 {
                v = rr(bar, REG_EERD);
                if v & (1 << 4) != 0 || v & (1 << 1) != 0 {
                    // DONE bit differs; take data in high bits
                    break;
                }
                core::hint::spin_loop();
            }
            let word = (v >> 16) as u16;
            mac[i as usize * 2] = (word & 0xff) as u8;
            mac[i as usize * 2 + 1] = (word >> 8) as u8;
        }
        if mac != [0; 6] && mac != [0xff; 6] {
            return mac;
        }
    }
    let ral = rr(bar, REG_RAL);
    let rah = rr(bar, REG_RAH);
    [
        ral as u8,
        (ral >> 8) as u8,
        (ral >> 16) as u8,
        (ral >> 24) as u8,
        rah as u8,
        (rah >> 8) as u8,
    ]
}

/// Copia un frame RX a `out`; devuelve longitud o None.
pub fn receive(out: &mut [u8]) -> Option<usize> {
    let nic_m = NIC.get()?;
    let mut nic = nic_m.lock();
    if !nic.qemu && !nic.link_up {
        return None;
    }
    let rx = unsafe {
        core::slice::from_raw_parts_mut(
            dma::virt(nic.rx_phys).as_ptr() as *mut RxDesc,
            RX_DESC,
        )
    };
    let i = nic.rx_tail as usize;
    let st = rx[i].status;
    if st & RX_DD == 0 {
        return None;
    }
    if st & RX_EOP == 0 || rx[i].errors != 0 {
        rx[i].status = 0;
        nic.rx_tail = ((i + 1) % RX_DESC) as u16;
        rw(
            nic.mmio,
            REG_RDT,
            ((nic.rx_tail + RX_DESC as u16 - 1) % RX_DESC as u16) as u32,
        );
        return None;
    }
    let len = rx[i].length as usize;
    let n = len.min(out.len()).min(BUF_LEN);
    unsafe {
        let src = mm::phys_to_virt(rx[i].addr).as_ptr::<u8>();
        core::ptr::copy_nonoverlapping(src, out.as_mut_ptr(), n);
    }
    rx[i].status = 0;
    nic.rx_tail = ((i + 1) % RX_DESC) as u16;
    rw(nic.mmio, REG_RDT, ((nic.rx_tail + RX_DESC as u16 - 1) % RX_DESC as u16) as u32);
    Some(n)
}

fn tx_slot_free(tx: &[TxDesc], i: usize) -> bool {
    tx[i].status & TX_DD != 0 || tx[i].cmd == 0
}

pub fn can_send() -> bool {
    let Some(nic_m) = NIC.get() else {
        return false;
    };
    let nic = nic_m.lock();
    if !nic.qemu && !nic.link_up {
        return false;
    }
    let i = nic.tx_tail as usize;
    let tx = unsafe {
        core::slice::from_raw_parts(
            dma::virt(nic.tx_phys).as_ptr() as *const TxDesc,
            TX_DESC,
        )
    };
    tx_slot_free(tx, i)
}

/// Envia `packet` (frame ethernet completo).
pub fn send(packet: &[u8]) -> Result<(), ()> {
    let nic_m = NIC.get().ok_or(())?;
    let mut nic = nic_m.lock();
    if !nic.qemu && !nic.link_up {
        return Err(());
    }
    let len = packet.len().min(BUF_LEN);
    let i = nic.tx_tail as usize;
    let tx = unsafe {
        core::slice::from_raw_parts_mut(
            dma::virt(nic.tx_phys).as_ptr() as *mut TxDesc,
            TX_DESC,
        )
    };
    // Esperar descriptor libre (no sobrescribir TX en vuelo).
    for _ in 0..500_000 {
        if tx_slot_free(tx, i) {
            break;
        }
        core::hint::spin_loop();
    }
    if !tx_slot_free(tx, i) {
        return Err(());
    }
    let buf_phys = (nic.tx_buf_phys + i * BUF_LEN) as u64;
    unsafe {
        core::ptr::copy_nonoverlapping(
            packet.as_ptr(),
            mm::phys_to_virt(buf_phys).as_mut_ptr(),
            len,
        );
    }
    tx[i].addr = buf_phys;
    tx[i].length = len as u16;
    tx[i].cso = 0;
    tx[i].cmd = (1 << 0) | (1 << 1) | (1 << 3); // EOP | IFCS | RS
    tx[i].status = 0;
    nic.tx_tail = ((i + 1) % TX_DESC) as u16;
    rw(nic.mmio, REG_TDT, nic.tx_tail as u32);
    // RS: esperar DD antes de reutilizar (SSH/TCP burst).
    for _ in 0..500_000 {
        if tx[i].status & TX_DD != 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(())
}

pub fn mac() -> Option<[u8; 6]> {
    NIC.get().map(|n| n.lock().mac)
}
