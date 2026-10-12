//! Intel HD Audio — captura PCM16 mono 16 kHz (QEMU ich9 + hda-duplex).

use crate::drivers::dma;
use crate::drivers::pci;
use crate::mm;
use crate::println;
use spin::Mutex;

const CLASS_HDA: u8 = 0x04;
const SUBCLASS_HDA: u8 = 0x03;
const VENDOR_NVIDIA: u16 = 0x10de;
const VENDOR_AMD: u16 = 0x1002;

struct HdaCandidate {
    bus: u8,
    device: u8,
    function: u8,
    vendor_id: u16,
    device_id: u16,
    bar0: u64,
    bar0_size: u64,
    score: i32,
}

fn hda_score(vendor: u16) -> i32 {
    match vendor {
        VENDOR_NVIDIA | VENDOR_AMD => 0,
        _ => 10,
    }
}

const GCTL: u32 = 0x08;
const CORBLBASE: u32 = 0x40;
const CORBUBASE: u32 = 0x44;
const CORBWP: u32 = 0x48;
const CORBRP: u32 = 0x4A;
const CORBCTL: u32 = 0x4C;
const RIRBLBASE: u32 = 0x50;
const RIRBUBASE: u32 = 0x54;
const RIRBWP: u32 = 0x58;
const RIRBCTL: u32 = 0x5C;
const ICW: u32 = 0x60;
const IRS: u32 = 0x64;

const SD0_BASE: u32 = 0x180;
const SD_CTL: u32 = 0x00;
const SD_LPIB: u32 = 0x08;
const SD_CBL: u32 = 0x10;
const SD_CBID: u32 = 0x18;
const SD_FMT: u32 = 0x1C;

const BDL_COUNT: usize = 8;
const BUF_BYTES: usize = 4096;

struct HdaState {
    bar: u64,
    corb: dma::PhysAddr,
    rirb: dma::PhysAddr,
    bdl: dma::PhysAddr,
    bufs: [dma::PhysAddr; BDL_COUNT],
    read_pos: usize,
    open: bool,
}

impl HdaState {
    fn rr(&self, off: u32) -> u32 {
        unsafe { core::ptr::read_volatile(mm::phys_to_virt(self.bar + off as u64).as_ptr()) }
    }

    fn rw(&self, off: u32, v: u32) {
        unsafe {
            core::ptr::write_volatile(mm::phys_to_virt(self.bar + off as u64).as_mut_ptr(), v);
        }
    }

    fn rw16(&self, off: u32, v: u16) {
        unsafe {
            core::ptr::write_volatile(
                mm::phys_to_virt(self.bar + off as u64).as_mut_ptr(),
                v,
            );
        }
    }

    fn reset(&self) {
        self.rw(GCTL, 0);
        for _ in 0..100_000 {
            if self.rr(GCTL) & 1 == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        self.rw(GCTL, 1);
        for _ in 0..100_000 {
            if self.rr(GCTL) & 1 != 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }

    fn send_verb(&self, verb: u32) -> u64 {
        for _ in 0..500_000 {
            if self.rr(IRS) & 1 != 0 {
                self.rw(ICW, verb);
                for _ in 0..500_000 {
                    if self.rr(IRS) & 2 != 0 {
                        let resp = (self.rr(IRS) as u64) >> 8;
                        self.rw(IRS, 1);
                        return resp;
                    }
                    core::hint::spin_loop();
                }
                return 0;
            }
            core::hint::spin_loop();
        }
        0
    }

    fn setup_rings(&mut self) {
        self.corb = dma::alloc_pages(1);
        self.rirb = dma::alloc_pages(1);
        unsafe {
            core::ptr::write_bytes(dma::virt(self.corb).as_ptr(), 0, dma::PAGE_SIZE);
            core::ptr::write_bytes(dma::virt(self.rirb).as_ptr(), 0, dma::PAGE_SIZE);
        }
        self.rw(CORBLBASE, self.corb as u32);
        self.rw(CORBUBASE, (self.corb >> 32) as u32);
        self.rw16(CORBRP, 0);
        self.rw16(CORBWP, 0);
        self.rw(CORBCTL, 0x02);
        self.rw(RIRBLBASE, self.rirb as u32);
        self.rw(RIRBUBASE, (self.rirb >> 32) as u32);
        self.rw16(RIRBWP, 0);
        self.rw(RIRBCTL, 0x02);
    }

    fn start_capture(&mut self) {
        self.bdl = dma::alloc_pages(1);
        let bdl_ptr = dma::virt(self.bdl).as_ptr();
        unsafe { core::ptr::write_bytes(bdl_ptr, 0, dma::PAGE_SIZE) };
        for i in 0..BDL_COUNT {
            self.bufs[i] = dma::alloc_pages(1);
            unsafe { core::ptr::write_bytes(dma::virt(self.bufs[i]).as_ptr(), 0, dma::PAGE_SIZE) };
            let entry = unsafe { bdl_ptr.add(i * 16) };
            unsafe {
                core::ptr::write(entry as *mut u64, self.bufs[i] as u64);
                core::ptr::write(entry.add(8) as *mut u32, BUF_BYTES as u32);
            }
        }
        self.rw(SD0_BASE + SD_FMT, 0x9010);
        self.rw(SD0_BASE + SD_CBL, (BDL_COUNT * BUF_BYTES) as u32);
        self.rw(SD0_BASE + SD_CBID, self.bdl as u32);
        self.rw(SD0_BASE + SD_CBID + 4, (self.bdl >> 32) as u32);
        self.rw(SD0_BASE + SD_CTL, 0x06);
        self.read_pos = 0;
    }

    fn bringup_codec(&self) {
        let _ = self.send_verb(0x0017_0700);
        let _ = self.send_verb(0x0017_707 | (0x05 << 8));
        let _ = self.send_verb(0x0017_706 | (0x05 << 8));
    }
}

static HDA: Mutex<Option<HdaState>> = Mutex::new(None);

pub fn init() {
    let mut best: Option<HdaCandidate> = None;
    for d in pci::devices() {
        if d.class != CLASS_HDA || d.subclass != SUBCLASS_HDA {
            continue;
        }
        if d.bar0 == 0 {
            continue;
        }
        let cand = HdaCandidate {
            bus: d.bus,
            device: d.device,
            function: d.function,
            vendor_id: d.vendor_id,
            device_id: d.device_id,
            bar0: d.bar0,
            bar0_size: d.bar0_size,
            score: hda_score(d.vendor_id),
        };
        if best.as_ref().map(|b| cand.score > b.score).unwrap_or(true) {
            best = Some(cand);
        } else if best.as_ref().map(|b| cand.score == b.score).unwrap_or(false) {
            println!(
                "hda: descartado {:04x}:{:04x} (prefiero códec analógico sobre HDMI)",
                d.vendor_id, d.device_id
            );
        }
    }
    let Some(d) = best else {
        println!("hda: no se encontró controlador Intel HD Audio");
        return;
    };
    pci::write16(d.bus, d.device, d.function, 0x04, pci::read16(d.bus, d.device, d.function, 0x04) | 0x6);
    mm::ensure_mmio_mapped(d.bar0, d.bar0_size.max(0x10000));
    println!(
        "hda: Intel HD Audio {:04x}:{:04x} BAR0={:#x}",
        d.vendor_id, d.device_id, d.bar0
    );
    let mut st = HdaState {
        bar: d.bar0,
        corb: 0,
        rirb: 0,
        bdl: 0,
        bufs: [0; BDL_COUNT],
        read_pos: 0,
        open: false,
    };
    st.reset();
    st.setup_rings();
    st.bringup_codec();
    *HDA.lock() = Some(st);
}

pub fn open(_rate: u32, _channels: u16, _bits: u16) -> Result<(), i64> {
    let mut g = HDA.lock();
    let st = g.as_mut().ok_or(-soso_abi::ENOTSUP)?;
    if st.open {
        return Err(-soso_abi::EBUSY);
    }
    st.start_capture();
    st.open = true;
    Ok(())
}

pub fn read(dst: &mut [u8]) -> Result<(usize, bool), i64> {
    let mut g = HDA.lock();
    let st = g.as_mut().ok_or(-soso_abi::ENOTSUP)?;
    if !st.open {
        return Err(-soso_abi::EINVAL);
    }
    let lpib = st.rr(SD0_BASE + SD_LPIB) as usize;
    let total = BDL_COUNT * BUF_BYTES;
    let mut copied = 0usize;
    let mut overrun = false;
    while copied < dst.len() {
        if st.read_pos == lpib {
            break;
        }
        if lpib.wrapping_sub(st.read_pos) > total / 2 {
            overrun = true;
            st.read_pos = lpib.saturating_sub(BUF_BYTES);
        }
        let buf_idx = (st.read_pos / BUF_BYTES) % BDL_COUNT;
        let off = st.read_pos % BUF_BYTES;
        let src = dma::virt(st.bufs[buf_idx]).as_ptr();
        let avail = (BUF_BYTES - off).min(dst.len() - copied);
        unsafe {
            core::ptr::copy_nonoverlapping(src.add(off), dst.as_mut_ptr().add(copied), avail);
        }
        st.read_pos = (st.read_pos + avail) % total;
        copied += avail;
    }
    Ok((copied, overrun))
}

pub fn close() -> Result<(), i64> {
    let mut g = HDA.lock();
    if let Some(st) = g.as_mut() {
        if st.open {
            st.rw(SD0_BASE + SD_CTL, 0);
            st.open = false;
        }
    }
    Ok(())
}

pub fn present() -> bool {
    HDA.lock().is_some()
}
