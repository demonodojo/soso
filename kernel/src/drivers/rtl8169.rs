//! Realtek RTL8111/8168/8211/8411 (familia Linux `r8169`) — TX/RX polled.
//!
//! Bring-up calcado de `drivers/net/ethernet/realtek/r8169_main.c`: reset por
//! `ChipCmd`, anillos de descriptores Own/EOR, `TxPoll` NPQ, ERI/`RXDV_GATED_EN`
//! en 8168G/H, MDIO por PHYAR o GPHY OCP según el XID de `TxConfig`.
//! No mueve tráfico si el enlace PHY sigue DOWN (cable).

use crate::arch::irq;
use crate::drivers::{dma, pci};
use crate::mm;
use crate::println;
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, Ordering, compiler_fence};
use spin::{Mutex, Once};

const VENDOR_REALTEK: u16 = 0x10ec;
const DEVICE_IDS: &[u16] = &[0x8168, 0x8161, 0x8162, 0x8167, 0x8136];

const BAR_MMIO: u8 = 2;

const REG_MAC0: u32 = 0x00;
const REG_MAR0: u32 = 0x08;
const REG_TX_DESC_LO: u32 = 0x20;
const REG_TX_DESC_HI: u32 = 0x24;
const REG_CHIPCMD: u32 = 0x37;
const REG_TXPOLL: u32 = 0x38;
const REG_INTRMASK: u32 = 0x3c;
const REG_INTRSTATUS: u32 = 0x3e;
const REG_TXCONFIG: u32 = 0x40;
const REG_RXCONFIG: u32 = 0x44;
const REG_CFG9346: u32 = 0x50;
const REG_CONFIG2: u32 = 0x53;
const REG_CONFIG3: u32 = 0x54;
const REG_CONFIG5: u32 = 0x56;
const REG_PHYAR: u32 = 0x60;
const REG_PHYSTATUS: u32 = 0x6c;
const REG_ERIDR: u32 = 0x70;
const REG_ERIAR: u32 = 0x74;
const REG_GPHY_OCP: u32 = 0xb8;
const REG_MCU: u32 = 0xd3;
const REG_RXMAXSIZE: u32 = 0xda;
const REG_CPLUSCMD: u32 = 0xe0;
const REG_INTRMITIGATE: u32 = 0xe2;
const REG_RX_DESC_LO: u32 = 0xe4;
const REG_RX_DESC_HI: u32 = 0xe8;
const REG_MAXTXPKT: u32 = 0xec;
const REG_MISC: u32 = 0xf0;

const CMD_RESET: u8 = 0x10;
const CMD_RX_EN: u8 = 0x08;
const CMD_TX_EN: u8 = 0x04;
const TXPOLL_NPQ: u8 = 0x40;
const CFG_UNLOCK: u8 = 0xc0;
const CFG_LOCK: u8 = 0x00;

const CLKREQ_EN: u8 = 1 << 7;
const RDY_TO_L23: u8 = 1 << 1;
const ASPM_EN: u8 = 1 << 0;
const NOW_IS_OOB: u8 = 1 << 7;

const RX128_INT_EN: u32 = 1 << 15;
const RX_MULTI_EN: u32 = 1 << 14;
const RX_EARLY_OFF: u32 = 1 << 11;
const RX_DMA_BURST: u32 = 7 << 8;
const RX_ACCEPT: u32 = 0x0e; // broadcast + multicast + my phys
const TX_AUTO_FIFO: u32 = 1 << 7;
const RXDV_GATED_EN: u32 = 1 << 19;

const DESC_OWN: u32 = 1 << 31;
const DESC_EOR: u32 = 1 << 30;
const DESC_FS: u32 = 1 << 29;
const DESC_LS: u32 = 1 << 28;
const RX_RES: u32 = 1 << 21;
const RX_LEN_MASK: u32 = 0x3fff;

const CPLUS_PCIMULRW: u16 = 1 << 3;
const TX_PKT_MAX: u8 = (8064 / 128) as u8;

const ERIAR_FLAG: u32 = 0x8000_0000;
const ERIAR_EXGMAC: u32 = 0;
const ERIAR_MASK_0001: u32 = 0x1 << 12;
const ERIAR_MASK_1111: u32 = 0xf << 12;
const OCP_FLAG: u32 = 0x8000_0000;
const OCP_STD_PHY: u32 = 0xa400;

const PHY_LINK_OK: u8 = 0x02;
const PHY_FULL_DUP: u8 = 0x01;
const PHY_10: u8 = 0x04;
const PHY_100: u8 = 0x08;
const PHY_1000: u8 = 0x10;

const MII_BMCR: u32 = 0;
const MII_BMSR: u32 = 1;
const MII_ADVERTISE: u32 = 4;
const MII_CTRL1000: u32 = 9;
const PHY_TBI: u8 = 0x80;

const BMCR_RESET: u16 = 0x8000;
const BMCR_POWERDOWN: u16 = 0x0800;
const BMCR_ANENABLE: u16 = 0x1000;
const BMCR_ANRESTART: u16 = 0x0200;

const IRQ_RX_OK: u16 = 0x0001;
const IRQ_RX_ERR: u16 = 0x0002;
const IRQ_TX_OK: u16 = 0x0004;
const IRQ_TX_ERR: u16 = 0x0008;
const IRQ_RX_OVERFLOW: u16 = 0x0010;
const IRQ_LINK_CHG: u16 = 0x0020;
const IRQ_RX_FIFO: u16 = 0x0040;
const IRQ_MASK: u16 =
    IRQ_RX_OK | IRQ_RX_ERR | IRQ_TX_OK | IRQ_TX_ERR | IRQ_RX_OVERFLOW | IRQ_LINK_CHG | IRQ_RX_FIFO;

const RX_DESC: usize = 64;
const TX_DESC: usize = 64;
const BUF_LEN: usize = 2048;

const CAP_PCIE: u8 = 0x10;
const PCIE_LNKCTL: u8 = 0x10;
const LNKCTL_ASPM: u16 = 0x0003;
const LNKCTL_CLKREQ: u16 = 0x0100;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Desc {
    opts1: u32,
    opts2: u32,
    addr: u64,
}

struct Nic {
    mmio: u64,
    mac: [u8; 6],
    xid: u16,
    mac_ver: u16,
    ocp_base: u32,
    rx_phys: dma::PhysAddr,
    tx_phys: dma::PhysAddr,
    rx_buf_phys: dma::PhysAddr,
    tx_buf_phys: dma::PhysAddr,
    rx_i: u16,
    tx_i: u16,
    link_up: bool,
}

static NIC: Once<Mutex<Nic>> = Once::new();
static PRESENT: AtomicBool = AtomicBool::new(false);
static LAST_LINK_POLL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

fn r8(base: u64, off: u32) -> u8 {
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(base + off as u64).as_ptr()) }
}

fn r16(base: u64, off: u32) -> u16 {
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(base + off as u64).as_ptr()) }
}

fn r32(base: u64, off: u32) -> u32 {
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(base + off as u64).as_ptr()) }
}

fn w8(base: u64, off: u32, v: u8) {
    unsafe {
        core::ptr::write_volatile(mm::phys_to_virt(base + off as u64).as_mut_ptr(), v);
    }
}

fn w16(base: u64, off: u32, v: u16) {
    unsafe {
        core::ptr::write_volatile(mm::phys_to_virt(base + off as u64).as_mut_ptr(), v);
    }
}

fn w32(base: u64, off: u32, v: u32) {
    unsafe {
        core::ptr::write_volatile(mm::phys_to_virt(base + off as u64).as_mut_ptr(), v);
    }
}

fn spin_n(n: u32) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}

fn pages_for(bytes: usize) -> usize {
    bytes.div_ceil(dma::PAGE_SIZE).max(1)
}

fn phy_ocp(xid: u16) -> bool {
    xid >= 0x4c0
}

fn opts1(d: &Desc) -> u32 {
    unsafe { core::ptr::read_unaligned(addr_of!(d.opts1)) }
}

fn set_opts1(d: &mut Desc, v: u32) {
    unsafe { core::ptr::write_unaligned(addr_of_mut!(d.opts1), v) };
}

fn set_opts2(d: &mut Desc, v: u32) {
    unsafe { core::ptr::write_unaligned(addr_of_mut!(d.opts2), v) };
}

fn set_addr(d: &mut Desc, v: u64) {
    unsafe { core::ptr::write_unaligned(addr_of_mut!(d.addr), v) };
}

fn desc_addr(d: &Desc) -> u64 {
    unsafe { core::ptr::read_unaligned(addr_of!(d.addr)) }
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

fn pcie_disable_aspm(bus: u8, dev: u8, func: u8) {
    let status = pci::read16(bus, dev, func, 0x06);
    if status & (1 << 4) == 0 {
        return;
    }
    let mut off = pci::read8(bus, dev, func, 0x34);
    for _ in 0..48 {
        if off < 0x40 || off == 0xff {
            break;
        }
        let id = pci::read8(bus, dev, func, off);
        let next = pci::read8(bus, dev, func, off + 1);
        if id == CAP_PCIE {
            let mut lnk = pci::read16(bus, dev, func, off + PCIE_LNKCTL);
            lnk &= !(LNKCTL_ASPM | LNKCTL_CLKREQ);
            pci::write16(bus, dev, func, off + PCIE_LNKCTL, lnk);
            return;
        }
        if next == 0 || next == off {
            break;
        }
        off = next;
    }
}

fn eri_wait_clear(mmio: u64) -> bool {
    for _ in 0..100_000 {
        if r32(mmio, REG_ERIAR) & ERIAR_FLAG == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn eri_wait_set(mmio: u64) -> bool {
    for _ in 0..100_000 {
        if r32(mmio, REG_ERIAR) & ERIAR_FLAG != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn eri_write(mmio: u64, addr: u32, mask: u32, val: u32) {
    if !eri_wait_clear(mmio) {
        return;
    }
    w32(mmio, REG_ERIDR, val);
    w32(
        mmio,
        REG_ERIAR,
        ERIAR_FLAG | ERIAR_EXGMAC | mask | (addr & 0x0fff),
    );
    let _ = eri_wait_clear(mmio);
    spin_n(2_000);
}

fn eri_read(mmio: u64, addr: u32) -> u32 {
    if !eri_wait_clear(mmio) {
        return 0;
    }
    w32(
        mmio,
        REG_ERIAR,
        ERIAR_EXGMAC | ERIAR_MASK_1111 | (addr & 0x0fff),
    );
    if !eri_wait_set(mmio) {
        return 0;
    }
    r32(mmio, REG_ERIDR)
}

fn eri_set_bits(mmio: u64, addr: u32, mask: u32, bits: u32) {
    let v = eri_read(mmio, addr);
    eri_write(mmio, addr, mask, v | bits);
}

fn eri_clear_bits(mmio: u64, addr: u32, mask: u32, bits: u32) {
    let v = eri_read(mmio, addr);
    eri_write(mmio, addr, mask, v & !bits);
}

fn mac_version(xid_raw: u16) -> u16 {
    xid_raw & 0xfcf
}

fn mac_version_name(ver: u16) -> &'static str {
    match ver {
        0x540 | 0x541 => "RTL_GIGA_MAC_VER_46 (8168H)",
        0x4c0..=0x4cf => "RTL_GIGA_MAC_VER_40+ (8168G)",
        0x2c0..=0x2cf => "RTL_GIGA_MAC_VER_28+",
        _ => "desconocido",
    }
}

fn is_8168h(ver: u16) -> bool {
    matches!(ver, 0x540 | 0x541)
}

fn ocp_wait(mmio: u64, want_set: bool) -> bool {
    for _ in 0..100_000 {
        let flag = r32(mmio, REG_GPHY_OCP) & OCP_FLAG != 0;
        if flag == want_set {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn phy_ocp_write(mmio: u64, reg: u32, data: u16) {
    if reg & 1 != 0 {
        return;
    }
    w32(mmio, REG_GPHY_OCP, OCP_FLAG | (reg << 15) | u32::from(data));
    let _ = ocp_wait(mmio, false);
}

fn phy_ocp_read(mmio: u64, reg: u32) -> Option<u16> {
    if reg & 1 != 0 {
        return None;
    }
    w32(mmio, REG_GPHY_OCP, reg << 15);
    if !ocp_wait(mmio, true) {
        return None;
    }
    Some((r32(mmio, REG_GPHY_OCP) & 0xffff) as u16)
}

fn phy_write(nic: &mut Nic, reg: u32, val: u16) {
    if phy_ocp(nic.xid) {
        if reg == 0x1f {
            nic.ocp_base = if val == 0 {
                OCP_STD_PHY
            } else {
                u32::from(val) << 4
            };
            return;
        }
        let mut r = reg;
        if nic.ocp_base != OCP_STD_PHY {
            r = r.saturating_sub(0x10);
        }
        phy_ocp_write(nic.mmio, nic.ocp_base + r * 2, val);
        return;
    }
    w32(
        nic.mmio,
        REG_PHYAR,
        0x8000_0000 | ((reg & 0x1f) << 16) | u32::from(val),
    );
    for _ in 0..20_000 {
        if r32(nic.mmio, REG_PHYAR) & 0x8000_0000 == 0 {
            break;
        }
        core::hint::spin_loop();
    }
    spin_n(4_000);
}

fn phy_power_up(nic: &mut Nic) {
    let bmcr = phy_read(nic, MII_BMCR);
    if bmcr != 0 {
        phy_write(nic, MII_BMCR, bmcr & !BMCR_POWERDOWN);
    }
}

fn rtl8168h_hw_phy_config(nic: &mut Nic) {
    // Tabla ephy mínima (rtl8168h_2_hw_phy_config, sin firmware rtl8168h-2.fw).
    const EPHY: &[(u16, u16)] = &[
        (0x06, 0x001f),
        (0x08, 0x0000),
        (0x11, 0x9600),
        (0x12, 0x0000),
        (0x1d, 0x0000),
        (0x1e, 0x0000),
    ];
    for &(reg, val) in EPHY {
        phy_ocp_write(nic.mmio, 0xa400 + u32::from(reg) * 2, val);
    }
}

fn rtl_hw_init_8168g(bar: u64) {
    w32(bar, REG_MISC, r32(bar, REG_MISC) | RXDV_GATED_EN);
    w8(bar, REG_CHIPCMD, 0);
    spin_n(50_000);
    w32(bar, REG_MISC, r32(bar, REG_MISC) & !RXDV_GATED_EN);
}

fn phy_autoneg(nic: &mut Nic) {
    phy_write(nic, MII_BMCR, BMCR_RESET);
    spin_n(80_000);
    phy_write(nic, MII_ADVERTISE, 0x01e1);
    phy_write(nic, MII_CTRL1000, 0x0200);
    phy_write(nic, MII_BMCR, BMCR_ANENABLE | BMCR_ANRESTART);
}

fn phy_read(nic: &mut Nic, reg: u32) -> u16 {
    if phy_ocp(nic.xid) {
        if reg == 0x1f {
            return if nic.ocp_base == OCP_STD_PHY {
                0
            } else {
                (nic.ocp_base >> 4) as u16
            };
        }
        let mut r = reg;
        if nic.ocp_base != OCP_STD_PHY {
            r = r.saturating_sub(0x10);
        }
        return phy_ocp_read(nic.mmio, nic.ocp_base + r * 2).unwrap_or(0);
    }
    w32(nic.mmio, REG_PHYAR, (reg & 0x1f) << 16);
    for _ in 0..20_000 {
        if r32(nic.mmio, REG_PHYAR) & 0x8000_0000 != 0 {
            let v = (r32(nic.mmio, REG_PHYAR) & 0xffff) as u16;
            spin_n(4_000);
            return v;
        }
        core::hint::spin_loop();
    }
    0
}

fn read_link(nic: &mut Nic) -> (bool, u8, u16) {
    let phy = r8(nic.mmio, REG_PHYSTATUS);
    let bmsr = phy_read(nic, MII_BMSR);
    let up = phy & PHY_LINK_OK != 0 && bmsr != 0;
    (up, phy, bmsr)
}

fn log_link(nic: &mut Nic, phy: u8, bmsr: u16, up: bool) {
    let tbi = if phy & PHY_TBI != 0 { " TBI" } else { "" };
    println!(
        "rtl8169: phystatus {phy:#04x} bmsr {bmsr:#06x}{tbi} → enlace {} {} {}",
        if up { "UP" } else { "DOWN" },
        velocidad(phy),
        if phy & PHY_FULL_DUP != 0 { "full" } else { "half" }
    );
}

/// Sondeo periódico del enlace; devuelve true si pasó de DOWN a UP.
pub fn poll_link() -> bool {
    let Some(nic_m) = NIC.get() else {
        return false;
    };
    let now = crate::arch::pit::uptime_ms();
    let last = LAST_LINK_POLL.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 1000 {
        return false;
    }
    LAST_LINK_POLL.store(now, Ordering::Relaxed);
    let mut nic = nic_m.lock();
    let (up, phy, bmsr) = read_link(&mut nic);
    let was = nic.link_up;
    nic.link_up = up;
    if up && !was {
        log_link(&mut nic, phy, bmsr, up);
        return true;
    }
    false
}

fn rtl_irq() {
    if let Some(n) = NIC.get() {
        if let Some(mut nic) = n.try_lock() {
            let st = r16(nic.mmio, REG_INTRSTATUS);
            w16(nic.mmio, REG_INTRSTATUS, st);
            if st & IRQ_LINK_CHG != 0 {
                let (up, phy, bmsr) = read_link(&mut nic);
                let was = nic.link_up;
                nic.link_up = up;
                if up != was {
                    log_link(&mut nic, phy, bmsr, up);
                }
            }
        }
    }
    crate::net::marcar_trabajo_pendiente();
}

pub fn present() -> bool {
    PRESENT.load(Ordering::Acquire)
}

pub fn mac() -> Option<[u8; 6]> {
    NIC.get().map(|n| n.lock().mac)
}

/// Identifica el chip, arma anillos y deja la NIC lista para smoltcp.
pub fn init() -> Option<[u8; 6]> {
    if present() {
        return mac();
    }
    let devs = pci::devices();
    let dev = devs
        .iter()
        .find(|d| d.vendor_id == VENDOR_REALTEK && DEVICE_IDS.contains(&d.device_id))?;

    let rev = pci::read8(dev.bus, dev.device, dev.function, 0x08);
    println!(
        "rtl8169: {:04x}:{:04x} rev {:02x} en {:02x}:{:02x}.{}",
        dev.vendor_id, dev.device_id, rev, dev.bus, dev.device, dev.function
    );

    let mut cmd = pci::read16(dev.bus, dev.device, dev.function, 0x04);
    cmd |= 0x6;
    pci::write16(dev.bus, dev.device, dev.function, 0x04, cmd);
    pcie_disable_aspm(dev.bus, dev.device, dev.function);

    let (bar, bar_size) = pci::bar_info(dev.bus, dev.device, dev.function, BAR_MMIO)?;
    mm::ensure_mmio_mapped(bar, bar_size.max(0x1000));

    w8(bar, REG_CFG9346, CFG_UNLOCK);
    w8(bar, REG_CONFIG2, r8(bar, REG_CONFIG2) & !CLKREQ_EN);
    w8(bar, REG_CONFIG5, r8(bar, REG_CONFIG5) & !ASPM_EN);
    w8(bar, REG_CONFIG3, r8(bar, REG_CONFIG3) & !RDY_TO_L23);
    w8(bar, REG_MCU, r8(bar, REG_MCU) & !NOW_IS_OOB);

    w8(bar, REG_CHIPCMD, CMD_RESET);
    for _ in 0..100_000 {
        if r8(bar, REG_CHIPCMD) & CMD_RESET == 0 {
            break;
        }
        core::hint::spin_loop();
    }
    if r8(bar, REG_CHIPCMD) & CMD_RESET != 0 {
        println!("rtl8169: el reset no bajó");
        return None;
    }

    let xid_raw = (r32(bar, REG_TXCONFIG) >> 20) as u16;
    let ver = mac_version(xid_raw);
    println!(
        "rtl8169: xid {xid_raw:#05x} → {ver:#05x} ({})",
        mac_version_name(ver)
    );

    if is_8168h(ver) {
        rtl_hw_init_8168g(bar);
    }

    let mut mac = [0u8; 6];
    for (i, b) in mac.iter_mut().enumerate() {
        *b = r8(bar, REG_MAC0 + i as u32);
    }
    if mac[0] & 1 != 0 || mac == [0; 6] {
        println!("rtl8169: MAC ilegible");
        return None;
    }
    println!(
        "rtl8169: mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    let (rx_phys, rx_ptr) = dma::alloc_zeroed(pages_for(RX_DESC * core::mem::size_of::<Desc>()));
    let (tx_phys, tx_ptr) = dma::alloc_zeroed(pages_for(TX_DESC * core::mem::size_of::<Desc>()));
    let (rx_buf_phys, _) = dma::alloc_zeroed(pages_for(RX_DESC * BUF_LEN));
    let (tx_buf_phys, _) = dma::alloc_zeroed(pages_for(TX_DESC * BUF_LEN));

    let rx = unsafe { core::slice::from_raw_parts_mut(rx_ptr.as_ptr() as *mut Desc, RX_DESC) };
    let tx = unsafe { core::slice::from_raw_parts_mut(tx_ptr.as_ptr() as *mut Desc, TX_DESC) };
    for (i, d) in rx.iter_mut().enumerate() {
        set_addr(d, (rx_buf_phys + i * BUF_LEN) as u64);
        set_opts2(d, 0);
        let mut o = DESC_OWN | BUF_LEN as u32;
        if i + 1 == RX_DESC {
            o |= DESC_EOR;
        }
        compiler_fence(Ordering::SeqCst);
        set_opts1(d, o);
    }
    for (i, d) in tx.iter_mut().enumerate() {
        set_addr(d, 0);
        set_opts2(d, 0);
        set_opts1(d, if i + 1 == TX_DESC { DESC_EOR } else { 0 });
    }

    w8(bar, REG_CFG9346, CFG_UNLOCK);
    w16(bar, REG_CPLUSCMD, CPLUS_PCIMULRW);
    w8(bar, REG_MAXTXPKT, TX_PKT_MAX);
    w16(bar, REG_RXMAXSIZE, BUF_LEN as u16);

    if phy_ocp(xid_raw) {
        w32(bar, REG_MISC, r32(bar, REG_MISC) & !RXDV_GATED_EN);
        eri_clear_bits(bar, 0xdc, ERIAR_MASK_0001, 1);
        eri_set_bits(bar, 0xdc, ERIAR_MASK_0001, 1);
    }

    w32(bar, REG_TX_DESC_HI, ((tx_phys as u64) >> 32) as u32);
    w32(bar, REG_TX_DESC_LO, tx_phys as u32);
    w32(bar, REG_RX_DESC_HI, ((rx_phys as u64) >> 32) as u32);
    w32(bar, REG_RX_DESC_LO, rx_phys as u32);
    w8(bar, REG_CFG9346, CFG_LOCK);

    let _ = r16(bar, REG_CPLUSCMD);
    w8(bar, REG_CHIPCMD, CMD_TX_EN | CMD_RX_EN);

    let mut rxcfg = RX128_INT_EN | RX_DMA_BURST;
    if phy_ocp(xid_raw) {
        rxcfg |= RX_MULTI_EN | RX_EARLY_OFF;
    }
    w32(bar, REG_RXCONFIG, rxcfg);
    let mut txcfg = (7u32 << 8) | (3u32 << 24);
    if xid_raw >= 0x2c0 {
        txcfg |= TX_AUTO_FIFO;
    }
    w32(bar, REG_TXCONFIG, txcfg);
    w32(bar, REG_MAR0, 0xffff_ffff);
    w32(bar, REG_MAR0 + 4, 0xffff_ffff);
    w32(
        bar,
        REG_RXCONFIG,
        (r32(bar, REG_RXCONFIG) & !0x3f) | RX_ACCEPT,
    );
    w16(bar, REG_INTRMITIGATE, 0);

    let mut nic = Nic {
        mmio: bar,
        mac,
        xid: xid_raw,
        mac_ver: ver,
        ocp_base: OCP_STD_PHY,
        rx_phys,
        tx_phys,
        rx_buf_phys,
        tx_buf_phys,
        rx_i: 0,
        tx_i: 0,
        link_up: false,
    };
    if is_8168h(ver) {
        rtl8168h_hw_phy_config(&mut nic);
    }
    phy_power_up(&mut nic);
    phy_autoneg(&mut nic);
    spin_n(200_000);
    let (link_up, phy, bmsr) = read_link(&mut nic);
    nic.link_up = link_up;
    log_link(&mut nic, phy, bmsr, link_up);

    w16(bar, REG_INTRSTATUS, 0xffff);
    if let Some(msix) = pci::find_msix(dev.bus, dev.device, dev.function) {
        if let Some(vec) = irq::allocate(rtl_irq) {
            if pci::msix_setup(&msix, 0, vec, pci::msix_default_dest()).is_ok() {
                println!("rtl8169: MSI-X vector={vec:#x}");
            }
        }
    }
    w16(bar, REG_INTRMASK, IRQ_MASK);

    let _ = (nic.rx_buf_phys, nic.tx_buf_phys);
    NIC.call_once(|| Mutex::new(nic));
    PRESENT.store(true, Ordering::Release);
    Some(mac)
}

pub fn receive(out: &mut [u8]) -> Option<usize> {
    let nic_m = NIC.get()?;
    let mut nic = nic_m.lock();
    let rx = unsafe {
        core::slice::from_raw_parts_mut(dma::virt(nic.rx_phys).as_ptr() as *mut Desc, RX_DESC)
    };
    let i = nic.rx_i as usize;
    let st = opts1(&rx[i]);
    if st & DESC_OWN != 0 {
        return None;
    }
    let eor = st & DESC_EOR;
    if st & RX_RES != 0 {
        compiler_fence(Ordering::SeqCst);
        set_opts1(&mut rx[i], DESC_OWN | eor | BUF_LEN as u32);
        nic.rx_i = ((i + 1) % RX_DESC) as u16;
        return None;
    }
    let pkt = ((st & RX_LEN_MASK) as usize).saturating_sub(4);
    let n = pkt.min(out.len()).min(BUF_LEN);
    if n >= 14 {
        unsafe {
            core::ptr::copy_nonoverlapping(
                mm::phys_to_virt(desc_addr(&rx[i])).as_ptr::<u8>(),
                out.as_mut_ptr(),
                n,
            );
        }
    }
    compiler_fence(Ordering::SeqCst);
    set_opts1(&mut rx[i], DESC_OWN | eor | BUF_LEN as u32);
    nic.rx_i = ((i + 1) % RX_DESC) as u16;
    if n >= 14 { Some(n) } else { None }
}

fn tx_free(tx: &[Desc], i: usize) -> bool {
    opts1(&tx[i]) & DESC_OWN == 0
}

pub fn can_send() -> bool {
    let Some(nic_m) = NIC.get() else {
        return false;
    };
    let nic = nic_m.lock();
    let tx = unsafe {
        core::slice::from_raw_parts(dma::virt(nic.tx_phys).as_ptr() as *const Desc, TX_DESC)
    };
    tx_free(tx, nic.tx_i as usize)
}

pub fn send(packet: &[u8]) -> Result<(), ()> {
    let nic_m = NIC.get().ok_or(())?;
    let mut nic = nic_m.lock();
    let len = packet.len().min(BUF_LEN).max(14);
    let i = nic.tx_i as usize;
    let tx = unsafe {
        core::slice::from_raw_parts_mut(dma::virt(nic.tx_phys).as_ptr() as *mut Desc, TX_DESC)
    };
    for _ in 0..500_000 {
        if tx_free(tx, i) {
            break;
        }
        core::hint::spin_loop();
    }
    if !tx_free(tx, i) {
        return Err(());
    }
    let buf_phys = (nic.tx_buf_phys + i * BUF_LEN) as u64;
    unsafe {
        core::ptr::copy_nonoverlapping(
            packet.as_ptr(),
            mm::phys_to_virt(buf_phys).as_mut_ptr(),
            packet.len().min(BUF_LEN),
        );
    }
    set_addr(&mut tx[i], buf_phys);
    set_opts2(&mut tx[i], 0);
    let mut o = DESC_OWN | DESC_FS | DESC_LS | len as u32;
    if i + 1 == TX_DESC {
        o |= DESC_EOR;
    }
    compiler_fence(Ordering::SeqCst);
    set_opts1(&mut tx[i], o);
    w8(nic.mmio, REG_TXPOLL, TXPOLL_NPQ);
    nic.tx_i = ((i + 1) % TX_DESC) as u16;
    for _ in 0..500_000 {
        if tx_free(tx, i) {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(())
}
