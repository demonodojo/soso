//! Realtek RTL8111/8168/8211/8411 y RTL8125 (familia Linux `r8169`) — TX/RX polled.
//!
//! Bring-up calcado de `drivers/net/ethernet/realtek/r8169_main.c`: reset por
//! `ChipCmd`, anillos de descriptores Own/EOR, `TxPoll` NPQ, ERI/`RXDV_GATED_EN`
//! en 8168G/H, OCP y ephy de VER_61/VER_63 en el 8125, MDIO por PHYAR o GPHY OCP
//! según el XID de `TxConfig`.
//! No mueve tráfico si el enlace PHY sigue DOWN (cable).

#[path = "rtl_xid.rs"]
mod rtl_xid;

use crate::arch::irq;
use crate::drivers::{dma, pci};
use crate::mm;
use crate::{klogln, println};
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, Ordering, compiler_fence};
use spin::{Mutex, Once};

const VENDOR_REALTEK: u16 = 0x10ec;

const BAR_MMIO: u8 = 2;

const REG_MAC0: u32 = 0x00;
const REG_MAR0: u32 = 0x08;
const REG_TX_DESC_LO: u32 = 0x20;
const REG_TX_DESC_HI: u32 = 0x24;
const REG_CHIPCMD: u32 = 0x37;
const REG_TXPOLL: u32 = 0x38;
const REG_INTRMASK: u32 = 0x3c;
const REG_INTRSTATUS: u32 = 0x3e;
/// Linux `enum rtl8125_registers`. En el 8168, 0x38 es TxPoll y 0x3c la máscara.
const REG_INTRMASK_8125: u32 = 0x38;
const REG_INTRSTATUS_8125: u32 = 0x3c;
const REG_TXPOLL_8125: u32 = 0x90;
const REG_MAC0_BKP: u32 = 0x19e0;
const REG_TXCONFIG: u32 = 0x40;
const REG_RXCONFIG: u32 = 0x44;
const REG_CFG9346: u32 = 0x50;
const REG_CONFIG1: u32 = 0x52;
const REG_CONFIG2: u32 = 0x53;
const REG_CONFIG3: u32 = 0x54;
const REG_CONFIG5: u32 = 0x56;
const REG_PHYAR: u32 = 0x60;
const REG_PHYSTATUS: u32 = 0x6c;
const REG_ERIDR: u32 = 0x70;
const REG_ERIAR: u32 = 0x74;
const REG_OCPDR: u32 = 0xb0;
const REG_GPHY_OCP: u32 = 0xb8;
const REG_MCU: u32 = 0xd3;
const REG_RXMAXSIZE: u32 = 0xda;
const REG_CPLUSCMD: u32 = 0xe0;
const REG_INTRMITIGATE: u32 = 0xe2;
const REG_RX_DESC_LO: u32 = 0xe4;
const REG_RX_DESC_HI: u32 = 0xe8;
const REG_MAXTXPKT: u32 = 0xec;
const REG_MISC: u32 = 0xf0;
const REG_EPHYAR: u32 = 0x80;
const REG_DLLPR: u32 = 0xd0;
const REG_MISC_1: u32 = 0xf2;
const REG_EEE_LED: u32 = 0x1b;

const EPHYAR_FLAG: u32 = 0x8000_0000;
const EPHYAR_WRITE_CMD: u32 = 0x8000_0000;
const PFM_EN: u8 = 1 << 6;
const PFM_D3COLD_EN: u8 = 1 << 6;
const TX_10M_PS_EN: u8 = 1 << 7;

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
/// Linux `RX_FETCH_DFLT_8125` (8 << 27). En 8125B el bit 11 es pause-slot.
const RX_FETCH_DFLT_8125: u32 = 8 << 27;
const RX_PAUSE_SLOT_ON: u32 = 1 << 11;
const LINK_LIST_RDY: u8 = 1 << 1;
const RXTX_EMPTY: u8 = (1 << 5) | (1 << 4);
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
const ERIAR_MASK_0011: u32 = 0x3 << 12;
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
/// Linux `rtl_set_irq_mask` para VER_61/63: sin `RxFIFOOver`.
const IRQ_MASK_8125: u32 = (IRQ_RX_OK | IRQ_RX_ERR | IRQ_TX_OK | IRQ_TX_ERR | IRQ_RX_OVERFLOW
    | IRQ_LINK_CHG) as u32;

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
    _mac_ver: u16,
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
    match rtl_xid::family(ver) {
        rtl_xid::RtlFamily::Rtl8168h => "RTL_GIGA_MAC_VER_46 (8168H)",
        rtl_xid::RtlFamily::Rtl8125a => "RTL_GIGA_MAC_VER_61 (8125A)",
        rtl_xid::RtlFamily::Rtl8125b => "RTL_GIGA_MAC_VER_63 (8125B)",
        rtl_xid::RtlFamily::Other(v) => match v {
            0x4c0..=0x4cf => "RTL_GIGA_MAC_VER_40+ (8168G)",
            0x2c0..=0x2cf => "RTL_GIGA_MAC_VER_28+",
            _ => "desconocido",
        },
    }
}

fn is_8168h(ver: u16) -> bool {
    rtl_xid::is_8168h(ver)
}

fn is_8125(ver: u16) -> bool {
    rtl_xid::is_8125(ver)
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

fn delay_ms(ms: u32) {
    let end = crate::arch::pit::uptime_ms().saturating_add(u64::from(ms));
    while crate::arch::pit::uptime_ms() < end {
        core::hint::spin_loop();
    }
}

fn mac_ocp_write(mmio: u64, reg: u32, data: u16) {
    if reg & 1 != 0 {
        return;
    }
    w32(mmio, REG_OCPDR, OCP_FLAG | (reg << 15) | u32::from(data));
}

fn mac_ocp_read(mmio: u64, reg: u32) -> u16 {
    if reg & 1 != 0 {
        return 0;
    }
    w32(mmio, REG_OCPDR, reg << 15);
    (r32(mmio, REG_OCPDR) & 0xffff) as u16
}

fn mac_ocp_modify(mmio: u64, reg: u32, mask: u16, set: u16) {
    let data = mac_ocp_read(mmio, reg);
    mac_ocp_write(mmio, reg, (data & !mask) | set);
}

struct EphyInfo {
    offset: u32,
    mask: u16,
    bits: u16,
}

fn ephy_wait(mmio: u64) -> bool {
    for _ in 0..100 {
        if r32(mmio, REG_EPHYAR) & EPHYAR_FLAG != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn ephy_write(mmio: u64, reg: u32, val: u16) {
    w32(
        mmio,
        REG_EPHYAR,
        EPHYAR_WRITE_CMD | (u32::from(val) & 0xffff) | ((reg & 0x1f) << 16),
    );
    let _ = ephy_wait(mmio);
    spin_n(10_000);
}

fn ephy_read(mmio: u64, reg: u32) -> u16 {
    w32(mmio, REG_EPHYAR, (reg & 0x1f) << 16);
    if ephy_wait(mmio) {
        (r32(mmio, REG_EPHYAR) & 0xffff) as u16
    } else {
        0xffff
    }
}

fn ephy_init(mmio: u64, table: &[EphyInfo]) {
    for e in table {
        let w = (ephy_read(mmio, e.offset) & !e.mask) | e.bits;
        ephy_write(mmio, e.offset, w);
    }
}

fn pci_write_ext_config_byte(bus: u8, dev: u8, func: u8, off: u16, val: u8) {
    let aligned = off & !3;
    let shift = (off & 3) * 8;
    let e = pci::ecam();
    let addr = e.base
        + ((bus as u64) << 20 | (dev as u64) << 15 | (func as u64) << 12 | aligned as u64);
    mm::ensure_mmio_mapped(addr, 4);
    let old: u32 =
        unsafe { core::ptr::read_volatile(mm::phys_to_virt(addr).as_ptr()) };
    let mask: u32 = !(0xffu32 << shift);
    unsafe {
        core::ptr::write_volatile(
            mm::phys_to_virt(addr).as_mut_ptr(),
            (old & mask) | ((val as u32) << shift),
        );
    }
}

/// Linux `rtl_set_def_aspm_entry_latency` — L0 7 µs, L1 16 µs (0x27 @ 0x70f).
fn rtl_set_def_aspm_entry_latency(bus: u8, dev: u8, func: u8) {
    pci_write_ext_config_byte(bus, dev, func, 0x070f, 0x27);
}

/// Linux `rtl_hw_start_8168h_1` (`r8169_main.c:3288`).
fn rtl_hw_start_8168h_1(nic: &mut Nic, bus: u8, dev: u8, func: u8) {
    const EPHY_8168H_1: [EphyInfo; 6] = [
        EphyInfo {
            offset: 0x1e,
            mask: 0x0800,
            bits: 0x0001,
        },
        EphyInfo {
            offset: 0x1d,
            mask: 0x0000,
            bits: 0x0800,
        },
        EphyInfo {
            offset: 0x05,
            mask: 0xffff,
            bits: 0x2089,
        },
        EphyInfo {
            offset: 0x06,
            mask: 0xffff,
            bits: 0x5881,
        },
        EphyInfo {
            offset: 0x04,
            mask: 0xffff,
            bits: 0x854a,
        },
        EphyInfo {
            offset: 0x01,
            mask: 0xffff,
            bits: 0x068b,
        },
    ];

    ephy_init(nic.mmio, &EPHY_8168H_1);
    eri_write(nic.mmio, 0xc8, ERIAR_MASK_1111, (0x08 << 16) | 0x02);
    eri_write(nic.mmio, 0xe8, ERIAR_MASK_1111, (0x10 << 16) | 0x06);
    eri_write(nic.mmio, 0xcc, ERIAR_MASK_0001, 0x38);
    eri_write(nic.mmio, 0xd0, ERIAR_MASK_0001, 0x48);
    rtl_set_def_aspm_entry_latency(bus, dev, func);
    eri_clear_bits(nic.mmio, 0xdc, ERIAR_MASK_0001, 1);
    eri_set_bits(nic.mmio, 0xdc, ERIAR_MASK_0001, 1);
    eri_write(nic.mmio, 0x5f0, ERIAR_MASK_0011, 0x4f87);
    w32(nic.mmio, REG_MISC, r32(nic.mmio, REG_MISC) & !RXDV_GATED_EN);
    eri_write(nic.mmio, 0xc0, ERIAR_MASK_0011, 0);
    eri_write(nic.mmio, 0xb8, ERIAR_MASK_0011, 0);
    w8(nic.mmio, REG_EEE_LED, r8(nic.mmio, REG_EEE_LED) & !0x07);
    eri_set_bits(nic.mmio, 0x1b0, ERIAR_MASK_0011, 0x0003);
    w8(nic.mmio, REG_DLLPR, r8(nic.mmio, REG_DLLPR) & !PFM_EN);
    w8(nic.mmio, REG_MISC_1, r8(nic.mmio, REG_MISC_1) & !PFM_D3COLD_EN);
    w8(nic.mmio, REG_DLLPR, r8(nic.mmio, REG_DLLPR) & !TX_10M_PS_EN);
    eri_clear_bits(nic.mmio, 0x1b0, ERIAR_MASK_0011, 1 << 12);
    w8(nic.mmio, REG_CONFIG3, r8(nic.mmio, REG_CONFIG3) & !RDY_TO_L23);

    let rg_saw_cnt = u32::from(phy_read_paged(nic, 0x0c42, 0x13) & 0x3fff);
    if rg_saw_cnt > 0 {
        let sw_cnt_1ms_ini = (16_000_000u32 / rg_saw_cnt) & 0x0fff;
        mac_ocp_modify(nic.mmio, 0xd412, 0x0fff, sw_cnt_1ms_ini as u16);
    }
    mac_ocp_modify(nic.mmio, 0xe056, 0x00f0, 0x0070);
    mac_ocp_modify(nic.mmio, 0xe052, 0x6000, 0x8008);
    mac_ocp_modify(nic.mmio, 0xe0d6, 0x01ff, 0x017f);
    mac_ocp_modify(nic.mmio, 0xd420, 0x0fff, 0x047f);
    mac_ocp_write(nic.mmio, 0xe63e, 0x0001);
    mac_ocp_write(nic.mmio, 0xe63e, 0x0000);
    mac_ocp_write(nic.mmio, 0xc094, 0x0000);
    mac_ocp_write(nic.mmio, 0xc09e, 0x0000);
    klogln!("rtl8169: rtl_hw_start_8168h_1 (VER_46) ok");
}

const RTL_FW_OPCODE_SIZE: usize = 4;

const PHY_FW_READ: u32 = 0x0;
const PHY_FW_DATA_OR: u32 = 0x1;
const PHY_FW_DATA_AND: u32 = 0x2;
const PHY_FW_BJMPN: u32 = 0x3;
const PHY_FW_MDIO_CHG: u32 = 0x4;
const PHY_FW_CLEAR_READCOUNT: u32 = 0x7;
const PHY_FW_WRITE: u32 = 0x8;
const PHY_FW_READCOUNT_EQ_SKIP: u32 = 0x9;
const PHY_FW_COMP_EQ_SKIPN: u32 = 0xa;
const PHY_FW_COMP_NEQ_SKIPN: u32 = 0xb;
const PHY_FW_WRITE_PREVIOUS: u32 = 0xc;
const PHY_FW_SKIPN: u32 = 0xd;
const PHY_FW_DELAY_MS: u32 = 0xe;

struct RtlPhyAction<'a> {
    code: &'a [u8],
}

fn fw_word_at(code: &[u8], index: usize) -> u32 {
    let off = index * RTL_FW_OPCODE_SIZE;
    u32::from_le_bytes([
        code[off],
        code[off + 1],
        code[off + 2],
        code[off + 3],
    ])
}

fn rtl_fw_format_ok(fw: &[u8]) -> Option<RtlPhyAction<'_>> {
    if fw.len() < RTL_FW_OPCODE_SIZE {
        return None;
    }
    if fw.len() >= 45 {
        let magic = u32::from_le_bytes([fw[0], fw[1], fw[2], fw[3]]);
        if magic == 0 {
            let mut checksum: u8 = 0;
            for &b in fw {
                checksum = checksum.wrapping_add(b);
            }
            if checksum != 0 {
                return None;
            }
            let start = u32::from_le_bytes([fw[36], fw[37], fw[38], fw[39]]) as usize;
            let size = u32::from_le_bytes([fw[40], fw[41], fw[42], fw[43]]) as usize;
            if start > fw.len() || size > (fw.len() - start) / RTL_FW_OPCODE_SIZE {
                return None;
            }
            let end = start + size * RTL_FW_OPCODE_SIZE;
            return Some(RtlPhyAction { code: &fw[start..end] });
        }
    }
    if fw.len() % RTL_FW_OPCODE_SIZE != 0 {
        return None;
    }
    Some(RtlPhyAction { code: fw })
}

fn rtl_fw_data_ok(pa: &RtlPhyAction<'_>) -> bool {
    let nwords = pa.code.len() / RTL_FW_OPCODE_SIZE;
    for index in 0..nwords {
        let action = fw_word_at(pa.code, index);
        let val = action & 0xffff;
        let regno = (action & 0x0fff_0000) >> 16;
        match action >> 28 {
            PHY_FW_READ
            | PHY_FW_DATA_OR
            | PHY_FW_DATA_AND
            | PHY_FW_CLEAR_READCOUNT
            | PHY_FW_WRITE
            | PHY_FW_WRITE_PREVIOUS
            | PHY_FW_DELAY_MS => {}
            PHY_FW_MDIO_CHG => {
                if val > 1 {
                    return false;
                }
            }
            PHY_FW_BJMPN => {
                if regno > index as u32 {
                    return false;
                }
            }
            PHY_FW_READCOUNT_EQ_SKIP => {
                if index + 2 >= nwords {
                    return false;
                }
            }
            PHY_FW_COMP_EQ_SKIPN | PHY_FW_COMP_NEQ_SKIPN | PHY_FW_SKIPN => {
                if index + 1 + regno as usize >= nwords {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

fn fw_phy_write(nic: &mut Nic, reg: u32, val: u16) {
    phy_write(nic, reg, val);
}

fn fw_phy_read(nic: &mut Nic, reg: u32) -> u16 {
    phy_read(nic, reg)
}

fn fw_mac_mcu_write(nic: &mut Nic, reg: u32, val: u16) {
    if reg == 0x1f {
        nic.ocp_base = u32::from(val) << 4;
        return;
    }
    mac_ocp_write(nic.mmio, nic.ocp_base + reg, val);
}

fn fw_mac_mcu_read(nic: &mut Nic, reg: u32) -> u16 {
    mac_ocp_read(nic.mmio, nic.ocp_base + reg)
}

fn rtl_fw_apply(nic: &mut Nic, pa: &RtlPhyAction<'_>) {
    let mut fw_write: fn(&mut Nic, u32, u16) = fw_phy_write;
    let mut fw_read: fn(&mut Nic, u32) -> u16 = fw_phy_read;
    let mut predata: u16 = 0;
    let mut count: u32 = 0;
    let mut index: usize = 0;

    let nwords = pa.code.len() / RTL_FW_OPCODE_SIZE;
    while index < nwords {
        let action = fw_word_at(pa.code, index);
        let data = action & 0xffff;
        let regno = (action & 0x0fff_0000) >> 16;
        let opcode = action >> 28;

        if action == 0 {
            break;
        }

        match opcode {
            PHY_FW_READ => {
                predata = fw_read(nic, regno);
                count += 1;
            }
            PHY_FW_DATA_OR => predata |= data as u16,
            PHY_FW_DATA_AND => predata &= data as u16,
            PHY_FW_BJMPN => {
                index = index.saturating_sub(regno as usize + 1);
                continue;
            }
            PHY_FW_MDIO_CHG => {
                if data == 0 {
                    fw_write = fw_phy_write;
                    fw_read = fw_phy_read;
                } else {
                    fw_write = fw_mac_mcu_write;
                    fw_read = fw_mac_mcu_read;
                }
            }
            PHY_FW_CLEAR_READCOUNT => count = 0,
            PHY_FW_WRITE => fw_write(nic, regno, data as u16),
            PHY_FW_READCOUNT_EQ_SKIP => {
                if count == data {
                    index += 1;
                }
            }
            PHY_FW_COMP_EQ_SKIPN => {
                if predata == data as u16 {
                    index += regno as usize;
                }
            }
            PHY_FW_COMP_NEQ_SKIPN => {
                if predata != data as u16 {
                    index += regno as usize;
                }
            }
            PHY_FW_WRITE_PREVIOUS => fw_write(nic, regno, predata),
            PHY_FW_SKIPN => index += regno as usize,
            PHY_FW_DELAY_MS => delay_ms(data),
            _ => break,
        }
        index += 1;
    }
    nic.ocp_base = OCP_STD_PHY;
}

fn phy_firmware_path(ver: u16) -> &'static str {
    match rtl_xid::family(ver) {
        rtl_xid::RtlFamily::Rtl8125a => "/lib/firmware/rtl_nic/rtl8125a-3.fw",
        rtl_xid::RtlFamily::Rtl8125b => "/lib/firmware/rtl_nic/rtl8125b-2.fw",
        rtl_xid::RtlFamily::Rtl8168h => "/lib/firmware/rtl_nic/rtl8168h-2.fw",
        rtl_xid::RtlFamily::Other(_) => "/lib/firmware/rtl_nic/rtl8168g-3.fw",
    }
}

fn load_phy_firmware(nic: &mut Nic) -> bool {
    let path = phy_firmware_path(nic._mac_ver);
    let Ok(ino) = crate::vfs::resolve(path) else {
        println!("rtl8169: firmware PHY ausente ({path})");
        return false;
    };
    let Ok(fw) = crate::vfs::read_file(ino) else {
        return false;
    };
    let Some(pa) = rtl_fw_format_ok(&fw) else {
        println!("rtl8169: firmware PHY formato inválido ({path})");
        return false;
    };
    if !rtl_fw_data_ok(&pa) {
        println!("rtl8169: firmware PHY bytecode inválido ({path})");
        return false;
    }
    rtl_fw_apply(nic, &pa);
    wait_bmcr_reset_clear(nic);
    klogln!(
        "rtl8169: firmware PHY cargado ({}, {} opcodes)",
        path,
        pa.code.len() / RTL_FW_OPCODE_SIZE
    );
    true
}

/// Linux `phy_select_page`: MDIO 0x1f. En GPHY OCP, 0 → `OCP_STD_PHY`.
fn phy_select_page(nic: &mut Nic, page: u16) -> u16 {
    let old = if nic.ocp_base == OCP_STD_PHY {
        0
    } else {
        (nic.ocp_base >> 4) as u16
    };
    phy_write(nic, 0x1f, page);
    old
}

fn phy_restore_page(nic: &mut Nic, old: u16) {
    phy_write(nic, 0x1f, old);
}

fn phy_read_paged(nic: &mut Nic, page: u16, reg: u32) -> u16 {
    let old = phy_select_page(nic, page);
    let v = phy_read(nic, reg);
    phy_restore_page(nic, old);
    v
}

fn phy_write_paged(nic: &mut Nic, page: u16, reg: u32, val: u16) {
    let old = phy_select_page(nic, page);
    phy_write(nic, reg, val);
    phy_restore_page(nic, old);
}

fn phy_modify_paged(nic: &mut Nic, page: u16, reg: u32, mask: u16, set: u16) {
    let old = phy_select_page(nic, page);
    let v = phy_read(nic, reg);
    phy_write(nic, reg, (v & !mask) | set);
    phy_restore_page(nic, old);
}

/// Linux `r8168g_phy_param` (`r8169_phy_config.c:42`): página 0x0a43, parm@0x13, modify 0x14.
fn r8168g_phy_param(nic: &mut Nic, parm: u16, mask: u16, val: u16) {
    let old = phy_select_page(nic, 0x0a43);
    phy_write(nic, 0x13, parm);
    let v = phy_read(nic, 0x14);
    phy_write(nic, 0x14, (v & !mask) | val);
    phy_restore_page(nic, old);
}

/// Linux `r8169_apply_firmware`: el blob puede dejar BMCR_RESET; espera ≤600 ms.
fn wait_bmcr_reset_clear(nic: &mut Nic) {
    let end = crate::arch::pit::uptime_ms().saturating_add(600);
    loop {
        if phy_read(nic, MII_BMCR) & BMCR_RESET == 0 {
            return;
        }
        if crate::arch::pit::uptime_ms() >= end {
            return;
        }
        delay_ms(1);
    }
}

/// Linux `rtl8168h_2_get_adc_bias_ioffset` (`r8169_main.c:2241`).
fn rtl8168h_2_get_adc_bias_ioffset(nic: &Nic) -> u16 {
    mac_ocp_write(nic.mmio, 0xdd02, 0x807d);
    let data1 = mac_ocp_read(nic.mmio, 0xdd02);
    let data2 = mac_ocp_read(nic.mmio, 0xdd00);
    let mut ioffset = (data2 >> 1) & 0x7ff8;
    ioffset |= data2 & 0x0007;
    if data1 & (1 << 7) != 0 {
        ioffset |= 1 << 15;
    }
    ioffset
}

/// Linux `rtl8168h_2_hw_phy_config` (`r8169_phy_config.c:796`). El fw es
/// opcional (`r8169_apply_firmware` no-op si falta); la cola paged sigue.
fn rtl8168h_hw_phy_config(nic: &mut Nic) {
    let _ = load_phy_firmware(nic);

    r8168g_phy_param(nic, 0x808a, 0x003f, 0x000a);
    r8168g_phy_param(nic, 0x0811, 0x0000, 0x0800);
    phy_modify_paged(nic, 0x0a42, 0x16, 0x0000, 0x0002);
    phy_modify_paged(nic, 0x0a44, 0x11, 0, 1 << 11);

    let ioffset = rtl8168h_2_get_adc_bias_ioffset(nic);
    if ioffset != 0xffff {
        phy_write_paged(nic, 0x0bcf, 0x16, ioffset);
    }

    let nibble = phy_read_paged(nic, 0x0bcd, 0x16) & 0x000f;
    let rlen: u16 = if nibble > 3 { nibble - 3 } else { 0 };
    let packed = rlen | (rlen << 4) | (rlen << 8) | (rlen << 12);
    phy_write_paged(nic, 0x0bcd, 0x17, packed);

    phy_modify_paged(nic, 0x0a44, 0x11, 1 << 7, 0);
    phy_modify_paged(nic, 0x0a43, 0x10, 1 << 0, 0);
    phy_modify_paged(nic, 0x0a43, 0x10, 1 << 2, 0);
    phy_modify_paged(nic, 0x0a43, 0x11, 0, 1 << 4);

    klogln!("rtl8169: PHY 8168H config Linux (ioffset={ioffset:#06x} rlen={rlen})");
}

fn rtl_hw_init_8168g(bar: u64) {
    w32(bar, REG_MISC, r32(bar, REG_MISC) | RXDV_GATED_EN);
    w8(bar, REG_CHIPCMD, 0);
    spin_n(50_000);
    w32(bar, REG_MISC, r32(bar, REG_MISC) & !RXDV_GATED_EN);
}

fn wait_mcu_mask(bar: u64, mask: u8) {
    for _ in 0..42 {
        if r8(bar, REG_MCU) & mask == mask {
            return;
        }
        delay_ms(1);
    }
}

/// Linux `rtl_hw_init_8125` (`r8169_main.c`, VER_61..63). No es `rtl_hw_init_8168g`.
fn rtl_hw_init_8125(bar: u64) {
    w32(bar, REG_MISC, r32(bar, REG_MISC) | RXDV_GATED_EN);
    delay_ms(2);
    wait_mcu_mask(bar, RXTX_EMPTY);
    w8(bar, REG_CHIPCMD, r8(bar, REG_CHIPCMD) & !(CMD_TX_EN | CMD_RX_EN));
    delay_ms(1);
    w8(bar, REG_MCU, r8(bar, REG_MCU) & !NOW_IS_OOB);
    mac_ocp_modify(bar, 0xe8de, 1u16 << 14, 0);
    wait_mcu_mask(bar, LINK_LIST_RDY);
    mac_ocp_write(bar, 0xc0aa, 0x07d0);
    mac_ocp_write(bar, 0xc0a6, 0x0150);
    mac_ocp_write(bar, 0xc01e, 0x5555);
    wait_mcu_mask(bar, LINK_LIST_RDY);
    klogln!("rtl8169: rtl_hw_init_8125 ok");
}

fn rtl8125_config_eee_mac(mmio: u64, ver63: bool) {
    if ver63 {
        // MTU 1500 + ETH_HLEN + 0x20. Linux `rtl8125_set_eee_txidle_timer`.
        w16(mmio, 0x6048, 1500 + 14 + 0x20);
        mac_ocp_modify(mmio, 0xe040, 0, 0x0003);
    } else {
        mac_ocp_modify(mmio, 0xe040, 0, 0x0003);
        mac_ocp_modify(mmio, 0xeb62, 0, 0x0006);
    }
}

/// Linux `rtl_hw_start_8125_common`.
fn rtl_hw_start_8125_common(nic: &mut Nic, ver63: bool) {
    let mmio = nic.mmio;
    w8(mmio, REG_CONFIG3, r8(mmio, REG_CONFIG3) & !RDY_TO_L23);
    w16(mmio, 0x382, 0x221b);
    w8(mmio, 0x4500, 0);
    w16(mmio, 0x4800, 0);
    mac_ocp_modify(mmio, 0xd40a, 0x0010, 0x0000);
    w8(mmio, REG_CONFIG1, r8(mmio, REG_CONFIG1) & !0x10);
    mac_ocp_write(mmio, 0xc140, 0xffff);
    mac_ocp_write(mmio, 0xc142, 0xffff);
    mac_ocp_modify(mmio, 0xd3e2, 0x0fff, 0x03a9);
    mac_ocp_modify(mmio, 0xd3e4, 0x00ff, 0x0000);
    mac_ocp_modify(mmio, 0xe860, 0x0000, 0x0080);
    mac_ocp_modify(mmio, 0xeb58, 0x0001, 0x0000);
    if ver63 {
        mac_ocp_modify(mmio, 0xe614, 0x0700, 0x0200);
        mac_ocp_modify(mmio, 0xe63e, 0x0c30, 0x0000);
    } else {
        mac_ocp_modify(mmio, 0xe614, 0x0700, 0x0400);
        mac_ocp_modify(mmio, 0xe63e, 0x0c30, 0x0020);
    }
    mac_ocp_modify(mmio, 0xc0b4, 0x0000, 0x000c);
    mac_ocp_modify(mmio, 0xeb6a, 0x00ff, 0x0033);
    mac_ocp_modify(mmio, 0xeb50, 0x03e0, 0x0040);
    mac_ocp_modify(mmio, 0xe056, 0x00f0, 0x0030);
    mac_ocp_modify(mmio, 0xe040, 0x1000, 0x0000);
    mac_ocp_modify(mmio, 0xea1c, 0x0003, 0x0001);
    mac_ocp_modify(mmio, 0xe0c0, 0x4f0f, 0x4403);
    mac_ocp_modify(mmio, 0xe052, 0x0080, 0x0068);
    mac_ocp_modify(mmio, 0xd430, 0x0fff, 0x047f);
    mac_ocp_modify(mmio, 0xea1c, 0x0004, 0x0000);
    mac_ocp_modify(mmio, 0xeb54, 0x0000, 0x0001);
    spin_n(1_000);
    mac_ocp_modify(mmio, 0xeb54, 0x0001, 0x0000);
    w16(mmio, 0x1880, r16(mmio, 0x1880) & !0x0030);
    mac_ocp_write(mmio, 0xe098, 0xc302);
    for _ in 0..10 {
        if mac_ocp_read(mmio, 0xe00e) & (1 << 13) == 0 {
            break;
        }
        delay_ms(1);
    }
    rtl8125_config_eee_mac(mmio, ver63);
    w32(mmio, REG_MISC, r32(mmio, REG_MISC) & !RXDV_GATED_EN);
}

/// Linux `rtl_hw_start_8125`: borra la coalescencia 0xa00..0xb00 y aplica el
/// ephy de 8125A (`rtl_hw_start_8125a_2`) o 8125B. No llama a `rtl_hw_start_8168h_1`.
fn rtl_hw_start_8125(nic: &mut Nic, bus: u8, dev: u8, func: u8) {
    let ver63 = matches!(rtl_xid::family(nic.xid), rtl_xid::RtlFamily::Rtl8125b);
    let ephy: &[EphyInfo] = if ver63 {
        &[
            EphyInfo { offset: 0x0b, mask: 0xffff, bits: 0xa908 },
            EphyInfo { offset: 0x1e, mask: 0xffff, bits: 0x20eb },
            EphyInfo { offset: 0x4b, mask: 0xffff, bits: 0xa908 },
            EphyInfo { offset: 0x5e, mask: 0xffff, bits: 0x20eb },
            EphyInfo { offset: 0x22, mask: 0x0030, bits: 0x0020 },
            EphyInfo { offset: 0x62, mask: 0x0030, bits: 0x0020 },
        ]
    } else {
        &[
            EphyInfo { offset: 0x04, mask: 0xffff, bits: 0xd000 },
            EphyInfo { offset: 0x0a, mask: 0xffff, bits: 0x8653 },
            EphyInfo { offset: 0x23, mask: 0xffff, bits: 0xab66 },
            EphyInfo { offset: 0x20, mask: 0xffff, bits: 0x9455 },
            EphyInfo { offset: 0x21, mask: 0xffff, bits: 0x99ff },
            EphyInfo { offset: 0x29, mask: 0xffff, bits: 0xfe04 },
            EphyInfo { offset: 0x44, mask: 0xffff, bits: 0xd000 },
            EphyInfo { offset: 0x4a, mask: 0xffff, bits: 0x8653 },
            EphyInfo { offset: 0x63, mask: 0xffff, bits: 0xab66 },
            EphyInfo { offset: 0x60, mask: 0xffff, bits: 0x9455 },
            EphyInfo { offset: 0x61, mask: 0xffff, bits: 0x99ff },
            EphyInfo { offset: 0x69, mask: 0xffff, bits: 0xfe04 },
        ]
    };

    w8(nic.mmio, REG_CFG9346, CFG_UNLOCK);
    for off in (0xa00u32..0xb00).step_by(4) {
        w32(nic.mmio, off, 0);
    }
    rtl_set_def_aspm_entry_latency(bus, dev, func);
    ephy_init(nic.mmio, ephy);
    rtl_hw_start_8125_common(nic, ver63);
    w8(nic.mmio, REG_CFG9346, CFG_LOCK);
    klogln!(
        "rtl8169: rtl_hw_start_8125{} ok",
        if ver63 { "b" } else { "a_2" }
    );
}

fn rtl8125_config_eee_phy(nic: &mut Nic, ver63: bool) {
    if !ver63 {
        phy_modify_paged(nic, 0x0a43, 0x11, 0, 1 << 4);
        phy_modify_paged(nic, 0x0a4a, 0x11, 0, 0x0200);
        phy_modify_paged(nic, 0x0a42, 0x14, 0, 0x0080);
    }
    phy_modify_paged(nic, 0x0a6d, 0x12, 0x0001, 0);
    phy_modify_paged(nic, 0x0a6d, 0x14, 0x0010, 0);
    if ver63 {
        phy_modify_paged(nic, 0x0a42, 0x14, 0x0080, 0);
        phy_modify_paged(nic, 0x0a4a, 0x11, 0x0200, 0);
    }
}

/// Linux `rtl8125a_2_hw_phy_config` / `rtl8125b_hw_phy_config`.
/// El blob es `rtl8125a-3.fw` o `rtl8125b-2.fw`. Si no está, la carga no se hace.
fn rtl8125_hw_phy_config(nic: &mut Nic) {
    let ver63 = matches!(rtl_xid::family(nic.xid), rtl_xid::RtlFamily::Rtl8125b);
    if ver63 {
        let _ = load_phy_firmware(nic);
        phy_modify_paged(nic, 0x0a44, 0x11, 0, 0x0800);
        phy_modify_paged(nic, 0x0ac4, 0x13, 0x00f0, 0x0090);
        phy_modify_paged(nic, 0x0ad3, 0x10, 0x0003, 0x0001);
        phy_write(nic, 0x1f, 0x0b87);
        phy_write(nic, 0x16, 0x80f5);
        phy_write(nic, 0x17, 0x760e);
        phy_write(nic, 0x16, 0x8107);
        phy_write(nic, 0x17, 0x360e);
        phy_write(nic, 0x16, 0x8551);
        let v = phy_read(nic, 0x17);
        phy_write(nic, 0x17, (v & !0xff00) | 0x0800);
        phy_write(nic, 0x1f, 0);
        phy_modify_paged(nic, 0x0bf0, 0x10, 0xe000, 0xa000);
        phy_modify_paged(nic, 0x0bf4, 0x13, 0x0f00, 0x0300);
        for parm in [0x8044u16, 0x804a, 0x8050, 0x8056, 0x805c, 0x8062, 0x8068, 0x806e, 0x8074, 0x807a]
        {
            r8168g_phy_param(nic, parm, 0xffff, 0x2417);
        }
        phy_modify_paged(nic, 0x0a4c, 0x15, 0, 0x0040);
        phy_modify_paged(nic, 0x0bf8, 0x12, 0xe000, 0xa000);
        phy_modify_paged(nic, 0x0a5b, 0x12, 1 << 15, 0);
    } else {
        phy_modify_paged(nic, 0x0ad4, 0x17, 0, 0x0010);
        phy_modify_paged(nic, 0x0ad1, 0x13, 0x03ff, 0x03ff);
        phy_modify_paged(nic, 0x0ad3, 0x11, 0x003f, 0x0006);
        phy_modify_paged(nic, 0x0ac0, 0x14, 0x1100, 0);
        phy_modify_paged(nic, 0x0acc, 0x10, 0x0003, 0x0002);
        phy_modify_paged(nic, 0x0ad4, 0x10, 0x00e7, 0x0044);
        phy_modify_paged(nic, 0x0ac1, 0x12, 0x0080, 0);
        phy_modify_paged(nic, 0x0ac8, 0x10, 0x0300, 0);
        phy_modify_paged(nic, 0x0ac5, 0x17, 0x0007, 0x0002);
        phy_write_paged(nic, 0x0ad4, 0x16, 0x00a8);
        phy_write_paged(nic, 0x0ac5, 0x16, 0x01ff);
        phy_modify_paged(nic, 0x0ac8, 0x15, 0x00f0, 0x0030);
        phy_write(nic, 0x1f, 0x0b87);
        phy_write(nic, 0x16, 0x80a2);
        phy_write(nic, 0x17, 0x0153);
        phy_write(nic, 0x16, 0x809c);
        phy_write(nic, 0x17, 0x0153);
        phy_write(nic, 0x1f, 0);
        phy_write(nic, 0x1f, 0x0a43);
        phy_write(nic, 0x13, 0x81b3);
        for word in [
            0x0043u16, 0x00a7, 0x00d6, 0x00ec, 0x00f6, 0x00fb, 0x00fd, 0x00ff, 0x00bb, 0x0058,
            0x0029, 0x0013, 0x0009, 0x0004, 0x0002,
        ] {
            phy_write(nic, 0x14, word);
        }
        for _ in 0..25 {
            phy_write(nic, 0x14, 0);
        }
        phy_write(nic, 0x1f, 0);
        r8168g_phy_param(nic, 0x8257, 0xffff, 0x020f);
        r8168g_phy_param(nic, 0x80ea, 0xffff, 0x7843);
        let _ = load_phy_firmware(nic);
        phy_modify_paged(nic, 0x0d06, 0x14, 0, 0x2000);
        r8168g_phy_param(nic, 0x81a2, 0, 0x0100);
        phy_modify_paged(nic, 0x0b54, 0x16, 0xff00, 0xdb00);
        phy_modify_paged(nic, 0x0a45, 0x12, 0x0001, 0);
        phy_modify_paged(nic, 0x0a5d, 0x12, 0, 0x0020);
        phy_modify_paged(nic, 0x0ad4, 0x17, 0x0010, 0);
        phy_modify_paged(nic, 0x0a86, 0x15, 0x0001, 0);
        phy_modify_paged(nic, 0x0a44, 0x11, 0, 1 << 11);
    }
    rtl8125_config_eee_phy(nic, ver63);
    klogln!(
        "rtl8169: PHY 8125{} config Linux",
        if ver63 { "B" } else { "A" }
    );
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

fn log_link(nic: &Nic, phy: u8, bmsr: u16, up: bool) {
    let tbi = if phy & PHY_TBI != 0 && !is_8168h(nic._mac_ver) {
        " TBI"
    } else {
        ""
    };
    klogln!(
        "rtl8169: phystatus {phy:#04x} bmsr {bmsr:#06x}{tbi} → enlace {} {} {}",
        if up { "UP" } else { "DOWN" },
        velocidad(phy),
        if phy & PHY_FULL_DUP != 0 { "full" } else { "half" }
    );
}

/// Último estado del enlace que vio el sondeo. Si la NIC está tomada no se
/// sabe: se contesta «arriba», que es lo que no inventa una caída.
pub fn link_up() -> bool {
    NIC.get()
        .and_then(|n| n.try_lock())
        .map(|n| n.link_up)
        .unwrap_or(true)
}

/// Sondeo periódico del enlace: `Some(true)` si pasó de DOWN a UP,
/// `Some(false)` si pasó de UP a DOWN, `None` si no cambió.
pub fn poll_link() -> Option<bool> {
    let nic_m = NIC.get()?;
    let now = crate::arch::pit::uptime_ms();
    let last = LAST_LINK_POLL.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 1000 {
        return None;
    }
    LAST_LINK_POLL.store(now, Ordering::Relaxed);
    let mut nic = nic_m.lock();
    let (up, phy, bmsr) = read_link(&mut nic);
    let was = nic.link_up;
    nic.link_up = up;
    if up != was {
        log_link(&nic, phy, bmsr, up);
        return Some(up);
    }
    None
}

fn mac_usable(mac: &[u8; 6]) -> bool {
    mac[0] & 1 == 0 && *mac != [0; 6]
}

fn read_mac_at(bar: u64, off: u32) -> [u8; 6] {
    let mut mac = [0u8; 6];
    for (i, b) in mac.iter_mut().enumerate() {
        *b = r8(bar, off + i as u32);
    }
    mac
}

/// Linux `rtl_read_mac_address`: en el 8125 la estación está en `MAC0_BKP`.
fn read_station_mac(bar: u64, bar_size: u64, ver: u16) -> Option<[u8; 6]> {
    if is_8125(ver) && bar_size >= u64::from(REG_MAC0_BKP) + 6 {
        let mac = read_mac_at(bar, REG_MAC0_BKP);
        if mac_usable(&mac) {
            return Some(mac);
        }
    }
    let mac = read_mac_at(bar, REG_MAC0);
    if mac_usable(&mac) { Some(mac) } else { None }
}

/// Linux `rtl_rar_set`: escribe la estación en MAC4 y luego en MAC0.
fn rar_set(bar: u64, mac: &[u8; 6]) {
    w32(
        bar,
        REG_MAC0 + 4,
        u32::from(u16::from_le_bytes([mac[4], mac[5]])),
    );
    let _ = r8(bar, REG_CHIPCMD);
    w32(
        bar,
        REG_MAC0,
        u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]),
    );
    let _ = r8(bar, REG_CHIPCMD);
}

fn ack_intr(mmio: u64, ver: u16) -> u32 {
    if is_8125(ver) {
        let st = r32(mmio, REG_INTRSTATUS_8125);
        w32(mmio, REG_INTRSTATUS_8125, st);
        st
    } else {
        let st = u32::from(r16(mmio, REG_INTRSTATUS));
        w16(mmio, REG_INTRSTATUS, st as u16);
        st
    }
}

fn arm_intr(mmio: u64, ver: u16) {
    if is_8125(ver) {
        w32(mmio, REG_INTRSTATUS_8125, 0xffff_ffff);
        w32(mmio, REG_INTRMASK_8125, IRQ_MASK_8125);
    } else {
        w16(mmio, REG_INTRSTATUS, 0xffff);
        w16(mmio, REG_INTRMASK, IRQ_MASK);
    }
}

fn tx_kick(mmio: u64, ver: u16) {
    if is_8125(ver) {
        w16(mmio, REG_TXPOLL_8125, 1);
    } else {
        w8(mmio, REG_TXPOLL, TXPOLL_NPQ);
    }
}

fn rtl_irq() {
    if let Some(n) = NIC.get() {
        if let Some(mut nic) = n.try_lock() {
            let st = ack_intr(nic.mmio, nic._mac_ver);
            if st & u32::from(IRQ_LINK_CHG) != 0 {
                let (up, phy, bmsr) = read_link(&mut nic);
                let was = nic.link_up;
                nic.link_up = up;
                if up != was {
                    log_link(&nic, phy, bmsr, up);
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
        .find(|d| d.vendor_id == VENDOR_REALTEK && rtl_xid::PCI_DEVICE_IDS.contains(&d.device_id))?;

    let rev = pci::read8(dev.bus, dev.device, dev.function, 0x08);
    klogln!(
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
    klogln!(
        "rtl8169: xid {xid_raw:#05x} → {ver:#05x} ({})",
        mac_version_name(ver)
    );

    if is_8168h(ver) {
        rtl_hw_init_8168g(bar);
    } else if is_8125(ver) {
        rtl_hw_init_8125(bar);
    }

    let Some(mac) = read_station_mac(bar, bar_size, ver) else {
        println!("rtl8169: MAC ilegible");
        return None;
    };
    klogln!(
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
    if !is_8125(ver) {
        w8(bar, REG_MAXTXPKT, TX_PKT_MAX);
    }
    w16(bar, REG_RXMAXSIZE, BUF_LEN as u16);
    if is_8125(ver) {
        rar_set(bar, &mac);
    }

    if phy_ocp(xid_raw) && !is_8125(ver) {
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

    let mut rxcfg = match rtl_xid::family(ver) {
        rtl_xid::RtlFamily::Rtl8125a => RX_FETCH_DFLT_8125 | RX_DMA_BURST,
        rtl_xid::RtlFamily::Rtl8125b => RX_FETCH_DFLT_8125 | RX_DMA_BURST | RX_PAUSE_SLOT_ON,
        _ => RX128_INT_EN | RX_DMA_BURST,
    };
    if !is_8125(ver) && phy_ocp(xid_raw) {
        rxcfg |= RX_MULTI_EN | RX_EARLY_OFF;
    }
    w32(bar, REG_RXCONFIG, rxcfg);
    let mut txcfg = (7u32 << 8) | (3u32 << 24);
    // Linux `rtl_is_8168evl_up` llega hasta VER_53. VER_61/63 no llevan este bit.
    if xid_raw >= 0x2c0 && !is_8125(ver) {
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
    if !is_8125(ver) {
        w16(bar, REG_INTRMITIGATE, 0);
    }

    let mut nic = Nic {
        mmio: bar,
        mac,
        xid: xid_raw,
        _mac_ver: ver,
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
        rtl_hw_start_8168h_1(&mut nic, dev.bus, dev.device, dev.function);
        rtl8168h_hw_phy_config(&mut nic);
    } else if is_8125(ver) {
        rtl_hw_start_8125(&mut nic, dev.bus, dev.device, dev.function);
        rtl8125_hw_phy_config(&mut nic);
    }
    phy_power_up(&mut nic);
    phy_autoneg(&mut nic);
    spin_n(200_000);
    let bmcr = phy_read(&mut nic, MII_BMCR);
    let adv = phy_read(&mut nic, MII_ADVERTISE);
    klogln!(
        "rtl8169: autoneg bmcr={bmcr:#06x} advertise={adv:#06x} ocp={:#x}",
        nic.ocp_base
    );
    let (link_up, phy, bmsr) = read_link(&mut nic);
    nic.link_up = link_up;
    log_link(&nic, phy, bmsr, link_up);

    if let Some(msix) = pci::find_msix(dev.bus, dev.device, dev.function) {
        if let Some(vec) = irq::allocate(rtl_irq) {
            if pci::msix_setup(&msix, 0, vec, pci::msix_default_dest()).is_ok() {
                klogln!("rtl8169: MSI-X vector={vec:#x}");
            }
        }
    }
    arm_intr(bar, ver);

    let _ = (nic.rx_buf_phys, nic.tx_buf_phys);
    NIC.call_once(|| Mutex::new(nic));
    PRESENT.store(true, Ordering::Release);
    Some(mac)
}

pub fn receive(out: &mut [u8]) -> Option<usize> {
    let nic_m = NIC.get()?;
    let mut nic = nic_m.lock();
    if !nic.link_up {
        return None;
    }
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
    if !nic.link_up {
        return false;
    }
    let tx = unsafe {
        core::slice::from_raw_parts(dma::virt(nic.tx_phys).as_ptr() as *const Desc, TX_DESC)
    };
    tx_free(tx, nic.tx_i as usize)
}

pub fn send(packet: &[u8]) -> Result<(), ()> {
    let nic_m = NIC.get().ok_or(())?;
    let mut nic = nic_m.lock();
    if !nic.link_up {
        return Err(());
    }
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
    tx_kick(nic.mmio, nic._mac_ver);
    nic.tx_i = ((i + 1) % TX_DESC) as u16;
    for _ in 0..500_000 {
        if tx_free(tx, i) {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(())
}
