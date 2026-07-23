//! Driver NVMe mínimo: admin queue + 1 I/O queue, MSI-X, Identify, R/W síncrono.
//!
//! Disco de modelos (sosomfs). Namespace 1; LBA 512 o 4096.

use crate::arch::irq;
use crate::drivers::{dma, pci};
use crate::mm;
use crate::println;
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use spin::{Mutex, Once};

const NVME_CLASS: u8 = 0x01;
const NVME_SUBCLASS: u8 = 0x08;

const REG_CAP: u64 = 0x00;
const REG_CC: u64 = 0x14;
const REG_CSTS: u64 = 0x1c;
const REG_AQA: u64 = 0x24;
const REG_ASQ: u64 = 0x28;
const REG_ACQ: u64 = 0x30;

const CC_EN: u32 = 1;
const CSTS_RDY: u32 = 1;

const QUEUE_ENTRIES: u16 = 16;
const SQE_SIZE: usize = 64;
const CQE_SIZE: usize = 16;

const OPC_DELETE_SQ: u8 = 0x00;
const OPC_CREATE_SQ: u8 = 0x01;
const OPC_CREATE_CQ: u8 = 0x05;
const OPC_IDENTIFY: u8 = 0x06;
const OPC_IO_WRITE: u8 = 0x01;
const OPC_IO_READ: u8 = 0x02;

struct Queue {
    sq_phys: dma::PhysAddr,
    cq_phys: dma::PhysAddr,
    /// VA uncached (no usar phys_to_virt WB).
    sq_virt: *mut u8,
    cq_virt: *mut u8,
    sq_tail: u16,
    cq_head: u16,
    phase: bool,
    qid: u16,
}

// SAFETY: acceso solo bajo Mutex del controlador.
unsafe impl Send for Queue {}

struct NvmeCtrl {
    regs: u64,
    dstrd: u32,
    admin: Queue,
    io: Queue,
    nsid: u32,
    lba_shift: u8,
    n_lba: u64,
    next_cid: AtomicU16,
}

static CTRL: Once<Mutex<NvmeCtrl>> = Once::new();
static CTRL1: Once<Mutex<NvmeCtrl>> = Once::new();
static PRESENT: AtomicBool = AtomicBool::new(false);
static PRESENT1: AtomicBool = AtomicBool::new(false);
static IRQ_FIRED: AtomicBool = AtomicBool::new(false);

const MAX_SLOTS: usize = 2;

fn slot_ctrl(slot: usize) -> Option<&'static Mutex<NvmeCtrl>> {
    match slot {
        0 => CTRL.get(),
        1 => CTRL1.get(),
        _ => None,
    }
}

fn slot_present(slot: usize) -> bool {
    match slot {
        0 => PRESENT.load(Ordering::Acquire),
        1 => PRESENT1.load(Ordering::Acquire),
        _ => false,
    }
}

fn set_slot_present(slot: usize) {
    match slot {
        0 => PRESENT.store(true, Ordering::Release),
        1 => PRESENT1.store(true, Ordering::Release),
        _ => {}
    }
}

fn reg_read32(base: u64, off: u64) -> u32 {
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(base + off).as_ptr()) }
}

fn reg_write32(base: u64, off: u64, v: u32) {
    unsafe {
        core::ptr::write_volatile(mm::phys_to_virt(base + off).as_mut_ptr(), v);
    }
}

fn reg_read64(base: u64, off: u64) -> u64 {
    unsafe { core::ptr::read_volatile(mm::phys_to_virt(base + off).as_ptr()) }
}

fn reg_write64(base: u64, off: u64, v: u64) {
    unsafe {
        core::ptr::write_volatile(mm::phys_to_virt(base + off).as_mut_ptr(), v);
    }
}

fn doorbell_sq(ctrl: &NvmeCtrl, qid: u16, tail: u16) {
    let off = 0x1000 + (2 * qid as u64) * (4 << ctrl.dstrd);
    reg_write32(ctrl.regs, off, tail as u32);
}

fn doorbell_cq(ctrl: &NvmeCtrl, qid: u16, head: u16) {
    let off = 0x1000 + (2 * qid as u64 + 1) * (4 << ctrl.dstrd);
    reg_write32(ctrl.regs, off, head as u32);
}

fn nvme_irq() {
    IRQ_FIRED.store(true, Ordering::Release);
}

pub fn present() -> bool {
    present_slot(0) || present_slot(1)
}

pub fn present_slot(slot: usize) -> bool {
    slot_present(slot)
}

pub fn init() {
    let devs: alloc::vec::Vec<_> = pci::enumerate()
        .into_iter()
        .filter(|d| d.class == NVME_CLASS && d.subclass == NVME_SUBCLASS)
        .collect();
    for (slot, dev) in devs.into_iter().take(MAX_SLOTS).enumerate() {
        if init_controller(&dev, slot).is_ok() {
            set_slot_present(slot);
        }
    }
}

fn init_controller(dev: &pci::PciDevice, slot: usize) -> Result<(), ()> {
    println!(
        "nvme[{slot}]: {:04x}:{:04x} en {:02x}:{:02x}.{}",
        dev.vendor_id, dev.device_id, dev.bus, dev.device, dev.function
    );

    // Bus master + memory space.
    let mut cmd = pci::read16(dev.bus, dev.device, dev.function, 0x04);
    cmd |= 0x6; // mem + bus master
    pci::write16(dev.bus, dev.device, dev.function, 0x04, cmd);

    let Some((bar, bar_size)) = pci::bar_info(dev.bus, dev.device, dev.function, 0) else {
        println!("nvme[{slot}]: sin BAR0");
        return Err(());
    };
    mm::ensure_mmio_mapped(bar, bar_size.max(0x2000));

    let cap = reg_read64(bar, REG_CAP);
    let dstrd = ((cap >> 32) & 0xf) as u32;
    let mps_min = ((cap >> 48) & 0xf) as u32;
    if mps_min > 0 {
        println!("nvme[{slot}]: MPSMIN={mps_min} (solo 4KiB soportado)");
        return Err(());
    }

    // Deshabilitar controlador si estaba activo.
    let cc = reg_read32(bar, REG_CC);
    if cc & CC_EN != 0 {
        reg_write32(bar, REG_CC, 0);
        wait_csts(bar, false);
    }

    let (asq_phys, asq_v) = dma::alloc_zeroed_uc(pages_for(QUEUE_ENTRIES as usize * SQE_SIZE));
    let (acq_phys, acq_v) = dma::alloc_zeroed_uc(pages_for(QUEUE_ENTRIES as usize * CQE_SIZE));

    // AQA: CQ size | SQ size (entries - 1 en cada campo de 16 bits)
    let aqa = ((QUEUE_ENTRIES as u32 - 1) << 16) | (QUEUE_ENTRIES as u32 - 1);
    reg_write32(bar, REG_AQA, aqa);
    reg_write64(bar, REG_ASQ, asq_phys as u64);
    reg_write64(bar, REG_ACQ, acq_phys as u64);

    // CC: EN=1, IOSQES=6 (64B), IOCQES=4 (16B), MPS=0 (4KiB)
    let cc = CC_EN | (6 << 16) | (4 << 20);
    reg_write32(bar, REG_CC, cc);
    if !wait_csts(bar, true) {
        println!("nvme[{slot}]: timeout esperando CSTS.RDY");
        return Err(());
    }

    let mut ctrl = NvmeCtrl {
        regs: bar,
        dstrd,
        admin: Queue {
            sq_phys: asq_phys,
            cq_phys: acq_phys,
            sq_virt: asq_v.as_ptr(),
            cq_virt: acq_v.as_ptr(),
            sq_tail: 0,
            cq_head: 0,
            phase: true,
            qid: 0,
        },
        io: Queue {
            sq_phys: 0,
            cq_phys: 0,
            sq_virt: core::ptr::null_mut(),
            cq_virt: core::ptr::null_mut(),
            sq_tail: 0,
            cq_head: 0,
            phase: true,
            qid: 1,
        },
        nsid: 1,
        lba_shift: 9,
        n_lba: 0,
        next_cid: AtomicU16::new(1),
    };

    // Identify Controller (valida admin queue).
    let (idc_phys, _) = dma::alloc_zeroed_uc(1);
    if let Err(e) = admin_identify_ctrl(&mut ctrl, idc_phys) {
        println!("nvme[{slot}]: identify ctrl: {e}");
        dump_admin_cq(&ctrl);
        dma::free_pages(idc_phys, 1);
        return Err(());
    }
    dma::free_pages(idc_phys, 1);
    println!("nvme[{slot}]: identify ctrl OK (dstrd={dstrd})");

    // Create I/O CQ then SQ (antes de MSI-X: el admin queue se polea).
    let (io_cq_phys, io_cq_v) = dma::alloc_zeroed_uc(pages_for(QUEUE_ENTRIES as usize * CQE_SIZE));
    let (io_sq_phys, io_sq_v) = dma::alloc_zeroed_uc(pages_for(QUEUE_ENTRIES as usize * SQE_SIZE));
    ctrl.io.cq_phys = io_cq_phys;
    ctrl.io.sq_phys = io_sq_phys;
    ctrl.io.cq_virt = io_cq_v.as_ptr();
    ctrl.io.sq_virt = io_sq_v.as_ptr();

    if let Err(e) = admin_create_cq(&mut ctrl) {
        println!("nvme[{slot}]: create CQ: {e}");
        dump_admin_cq(&ctrl);
        return Err(());
    }
    println!("nvme[{slot}]: I/O CQ creada");
    if let Err(e) = admin_create_sq(&mut ctrl) {
        println!("nvme[{slot}]: create SQ: {e}");
        dump_admin_cq(&ctrl);
        return Err(());
    }
    println!("nvme[{slot}]: I/O SQ creada");

    // MSI-X tras las colas (solo wake; el I/O sigue siendo síncrono por phase bit).
    if let Some(msix) = pci::find_msix(dev.bus, dev.device, dev.function) {
        if let Some(vec) = irq::allocate(nvme_irq) {
            if pci::msix_setup(&msix, 0, vec, pci::msix_default_dest()).is_ok() {
                println!("nvme[{slot}]: MSI-X vector={vec:#x}");
            }
        }
    }

    // Identify namespace 1.
    let (id_phys, id_ptr) = dma::alloc_zeroed_uc(1);
    if let Err(e) = admin_identify_ns(&mut ctrl, id_phys) {
        println!("nvme[{slot}]: identify: {e}");
        dma::free_pages(id_phys, 1);
        return Err(());
    }
    unsafe {
        let p = id_ptr.as_ptr();
        // NSZE at offset 0 (u64), FLBAS at 26
        let nsze = core::ptr::read_unaligned(p as *const u64);
        let flbas = *p.add(26);
        let lba_fmt = (flbas & 0xf) as usize;
        // LBAF starts at offset 128; each 4 bytes: MS(16) | LBADS(8) | RP(8)
        let lbads = *p.add(128 + lba_fmt * 4 + 2);
        ctrl.n_lba = nsze;
        ctrl.lba_shift = lbads;
    }
    dma::free_pages(id_phys, 1);

    let block_bytes = 1u64 << ctrl.lba_shift;
    let size_mib = ctrl.n_lba * block_bytes / (1024 * 1024);
    println!(
        "nvme[{slot}]: ns1 {} LBA × {} B ({} MiB)",
        ctrl.n_lba, block_bytes, size_mib
    );

    match slot {
        0 => {
            CTRL.call_once(|| Mutex::new(ctrl));
        }
        1 => {
            CTRL1.call_once(|| Mutex::new(ctrl));
        }
        _ => return Err(()),
    }
    Ok(())
}

fn pages_for(bytes: usize) -> usize {
    bytes.div_ceil(dma::PAGE_SIZE).max(1)
}

fn wait_csts(bar: u64, want_rdy: bool) -> bool {
    for _ in 0..1_000_000 {
        let rdy = reg_read32(bar, REG_CSTS) & CSTS_RDY != 0;
        if rdy == want_rdy {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn submit_sync(ctrl: &mut NvmeCtrl, admin: bool, sqe: &mut [u8; SQE_SIZE]) -> Result<u16, &'static str> {
    let q = if admin { &mut ctrl.admin } else { &mut ctrl.io };
    let cid = ctrl.next_cid.fetch_add(1, Ordering::Relaxed);
    sqe[2] = (cid & 0xff) as u8;
    sqe[3] = (cid >> 8) as u8;

    let idx = q.sq_tail as usize;
    let sq_ptr = q.sq_virt;
    unsafe {
        core::ptr::copy_nonoverlapping(sqe.as_ptr(), sq_ptr.add(idx * SQE_SIZE), SQE_SIZE);
    }
    q.sq_tail = (q.sq_tail + 1) % QUEUE_ENTRIES;
    let tail = q.sq_tail;
    let qid = q.qid;
    doorbell_sq(ctrl, qid, tail);

    // Esperar CQE (VA uncached; TCG puede ser lento).
    let deadline = 50_000_000u32;
    for _ in 0..deadline {
        let q = if admin { &ctrl.admin } else { &ctrl.io };
        let cq_ptr = q.cq_virt;
        let entry = unsafe { cq_ptr.add(q.cq_head as usize * CQE_SIZE) };
        // DW3: command_id [15:0], status [31:16] (phase = status bit 0).
        let dw3 = unsafe { core::ptr::read_volatile(entry.add(12) as *const u32) };
        let status = (dw3 >> 16) as u16;
        let phase = (status & 1) != 0;
        if phase == q.phase {
            let sc = (status >> 1) & 0xff;
            let q = if admin {
                &mut ctrl.admin
            } else {
                &mut ctrl.io
            };
            q.cq_head = (q.cq_head + 1) % QUEUE_ENTRIES;
            if q.cq_head == 0 {
                q.phase = !q.phase;
            }
            let head = q.cq_head;
            let qid = q.qid;
            doorbell_cq(ctrl, qid, head);
            if sc != 0 {
                println!("nvme: cmd status SC={sc:#x} raw={dw3:#x}");
                return Err("nvme status != 0");
            }
            return Ok(cid);
        }
        core::hint::spin_loop();
        let _ = IRQ_FIRED.swap(false, Ordering::AcqRel);
    }
    Err("nvme timeout")
}

fn admin_create_cq(ctrl: &mut NvmeCtrl) -> Result<(), &'static str> {
    let mut sqe = [0u8; SQE_SIZE];
    sqe[0] = OPC_CREATE_CQ;
    // PRP1
    let prp = ctrl.io.cq_phys as u64;
    sqe[24..32].copy_from_slice(&prp.to_le_bytes());
    // CDW10: QSIZE | QID
    let cdw10 = ((QUEUE_ENTRIES as u32 - 1) << 16) | 1;
    sqe[40..44].copy_from_slice(&cdw10.to_le_bytes());
    // CDW11: PC=1, IEN=0 (polled; MSI-X opcional más adelante)
    let cdw11 = 0b1u32;
    sqe[44..48].copy_from_slice(&cdw11.to_le_bytes());
    submit_sync(ctrl, true, &mut sqe).map(|_| ())
}

fn admin_create_sq(ctrl: &mut NvmeCtrl) -> Result<(), &'static str> {
    let mut sqe = [0u8; SQE_SIZE];
    sqe[0] = OPC_CREATE_SQ;
    let prp = ctrl.io.sq_phys as u64;
    sqe[24..32].copy_from_slice(&prp.to_le_bytes());
    let cdw10 = ((QUEUE_ENTRIES as u32 - 1) << 16) | 1;
    sqe[40..44].copy_from_slice(&cdw10.to_le_bytes());
    // CDW11: PC=1, QPRIO=0, CQID=1
    let cdw11 = (1u32 << 16) | 1;
    sqe[44..48].copy_from_slice(&cdw11.to_le_bytes());
    submit_sync(ctrl, true, &mut sqe).map(|_| ())
}

fn dump_admin_cq(ctrl: &NvmeCtrl) {
    let cq = ctrl.admin.cq_virt;
    println!(
        "nvme: dump head={} phase={} sq_tail={}",
        ctrl.admin.cq_head, ctrl.admin.phase as u8, ctrl.admin.sq_tail
    );
    for i in 0..4usize {
        unsafe {
            let e = cq.add(i * CQE_SIZE);
            let d0 = core::ptr::read_volatile(e as *const u32);
            let d1 = core::ptr::read_volatile(e.add(4) as *const u32);
            let d2 = core::ptr::read_volatile(e.add(8) as *const u32);
            let d3 = core::ptr::read_volatile(e.add(12) as *const u32);
            println!("nvme: CQ[{i}] {d0:#x} {d1:#x} {d2:#x} {d3:#x}");
        }
    }
}

fn admin_identify_ctrl(ctrl: &mut NvmeCtrl, buf_phys: dma::PhysAddr) -> Result<(), &'static str> {
    let mut sqe = [0u8; SQE_SIZE];
    sqe[0] = OPC_IDENTIFY;
    // NSID = 0
    let prp = buf_phys as u64;
    sqe[24..32].copy_from_slice(&prp.to_le_bytes());
    // CDW10 CNS = 1 (controller)
    sqe[40] = 1;
    submit_sync(ctrl, true, &mut sqe).map(|_| ())
}

fn admin_identify_ns(ctrl: &mut NvmeCtrl, buf_phys: dma::PhysAddr) -> Result<(), &'static str> {
    let mut sqe = [0u8; SQE_SIZE];
    sqe[0] = OPC_IDENTIFY;
    // NSID = 1
    sqe[4..8].copy_from_slice(&1u32.to_le_bytes());
    let prp = buf_phys as u64;
    sqe[24..32].copy_from_slice(&prp.to_le_bytes());
    // CDW10 CNS = 0 (namespace)
    sqe[40] = 0;
    submit_sync(ctrl, true, &mut sqe).map(|_| ())
}

/// Capacidad en sectores lógicos del dispositivo (slot 0, compat).
pub fn capacity_lba() -> Option<u64> {
    capacity_lba_slot(0)
}

pub fn capacity_lba_slot(slot: usize) -> Option<u64> {
    slot_ctrl(slot).map(|c| c.lock().n_lba)
}

pub fn lba_size() -> Option<usize> {
    lba_size_slot(0)
}

pub fn lba_size_slot(slot: usize) -> Option<usize> {
    slot_ctrl(slot).map(|c| 1usize << c.lock().lba_shift)
}

/// Lee `buf.len()` bytes desde LBA `lba` (debe ser múltiplo de LBA size).
pub fn read_lba(lba: u64, buf: &mut [u8]) -> Result<(), &'static str> {
    read_lba_slot(0, lba, buf)
}

pub fn read_lba_slot(slot: usize, lba: u64, buf: &mut [u8]) -> Result<(), &'static str> {
    let ctrl_m = slot_ctrl(slot).ok_or("nvme no init")?;
    let mut ctrl = ctrl_m.lock();
    let lba_size = 1usize << ctrl.lba_shift;
    if buf.len() % lba_size != 0 {
        return Err("len no alineado a LBA");
    }
    let nlb = (buf.len() / lba_size) as u32;
    if nlb == 0 {
        return Ok(());
    }
    // Una página DMA de rebote (máx 4 KiB por comando para simplicidad).
    let max = dma::PAGE_SIZE;
    let mut done = 0usize;
    while done < buf.len() {
        let chunk = (buf.len() - done).min(max);
        let chunk_nlb = (chunk / lba_size) as u32;
        let (phys, ptr) = dma::alloc_zeroed_uc(1);
        let mut sqe = [0u8; SQE_SIZE];
        sqe[0] = OPC_IO_READ;
        sqe[4..8].copy_from_slice(&ctrl.nsid.to_le_bytes());
        let prp = phys as u64;
        sqe[24..32].copy_from_slice(&prp.to_le_bytes());
        let slba = lba + (done / lba_size) as u64;
        sqe[40..48].copy_from_slice(&slba.to_le_bytes());
        // CDW12: NLB (0-based)
        let cdw12 = chunk_nlb - 1;
        sqe[48..52].copy_from_slice(&cdw12.to_le_bytes());
        let r = submit_sync(&mut ctrl, false, &mut sqe);
        if r.is_ok() {
            unsafe {
                core::ptr::copy_nonoverlapping(ptr.as_ptr(), buf.as_mut_ptr().add(done), chunk);
            }
        }
        dma::free_pages(phys, 1);
        r?;
        done += chunk;
        let _ = chunk_nlb;
    }
    Ok(())
}

/// Escribe `buf` en LBA `lba`.
pub fn write_lba(lba: u64, buf: &[u8]) -> Result<(), &'static str> {
    write_lba_slot(0, lba, buf)
}

pub fn write_lba_slot(slot: usize, lba: u64, buf: &[u8]) -> Result<(), &'static str> {
    let ctrl_m = slot_ctrl(slot).ok_or("nvme no init")?;
    let mut ctrl = ctrl_m.lock();
    let lba_size = 1usize << ctrl.lba_shift;
    if buf.len() % lba_size != 0 {
        return Err("len no alineado a LBA");
    }
    let max = dma::PAGE_SIZE;
    let mut done = 0usize;
    while done < buf.len() {
        let chunk = (buf.len() - done).min(max);
        let chunk_nlb = (chunk / lba_size) as u32;
        let (phys, ptr) = dma::alloc_zeroed_uc(1);
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr().add(done), ptr.as_ptr(), chunk);
        }
        let mut sqe = [0u8; SQE_SIZE];
        sqe[0] = OPC_IO_WRITE;
        sqe[4..8].copy_from_slice(&ctrl.nsid.to_le_bytes());
        let prp = phys as u64;
        sqe[24..32].copy_from_slice(&prp.to_le_bytes());
        let slba = lba + (done / lba_size) as u64;
        sqe[40..48].copy_from_slice(&slba.to_le_bytes());
        let cdw12 = chunk_nlb - 1;
        sqe[48..52].copy_from_slice(&cdw12.to_le_bytes());
        let r = submit_sync(&mut ctrl, false, &mut sqe);
        dma::free_pages(phys, 1);
        r?;
        done += chunk;
    }
    Ok(())
}

/// Lee un bloque de 4 KiB (índice de bloque sosofs/sosomfs).
pub fn read_block4k(block: u64, buf: &mut [u8; 4096]) -> Result<(), &'static str> {
    read_block4k_slot(0, block, buf)
}

pub fn read_block4k_slot(slot: usize, block: u64, buf: &mut [u8; 4096]) -> Result<(), &'static str> {
    let lba_size = lba_size_slot(slot).ok_or("nvme no init")?;
    let lba = block * (4096 / lba_size as u64);
    read_lba_slot(slot, lba, buf)
}

pub fn write_block4k(block: u64, buf: &[u8; 4096]) -> Result<(), &'static str> {
    write_block4k_slot(0, block, buf)
}

pub fn write_block4k_slot(slot: usize, block: u64, buf: &[u8; 4096]) -> Result<(), &'static str> {
    let lba_size = lba_size_slot(slot).ok_or("nvme no init")?;
    let lba = block * (4096 / lba_size as u64);
    write_lba_slot(slot, lba, buf)
}

pub fn block_count_4k() -> Option<u64> {
    block_count_4k_slot(0)
}

pub fn block_count_4k_slot(slot: usize) -> Option<u64> {
    let n = capacity_lba_slot(slot)?;
    let ls = lba_size_slot(slot)? as u64;
    Some(n * ls / 4096)
}

// Silenciar warning si create opcodes no usados en paths.
#[allow(dead_code)]
const _OPC: (u8, u8) = (OPC_DELETE_SQ, OPC_IO_WRITE);
