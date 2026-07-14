//! Driver GPU mínimo: detección Intel iGPU, allocación VRAM simulada y DMA.

use crate::drivers::pci;
use crate::println;
use alloc::vec::Vec;
use soso_abi::{self as abi, GpuInfo};
use spin::{Mutex, Once};

const VENDOR_INTEL: u16 = 0x8086;

struct GpuState {
    present: bool,
    vendor: u8,
    name: [u8; 32],
    vram_total: u64,
    vram_used: u64,
    buffers: Vec<Option<Vec<u8>>>,
}

static GPU: Once<Mutex<GpuState>> = Once::new();

pub fn init() {
    let devs = pci::enumerate();
    let intel_gpu = devs.iter().find(|d| d.class == 0x03 && d.vendor_id == VENDOR_INTEL);
    let state = if let Some(gpu) = intel_gpu {
        if gpu.bar0 != 0 && gpu.bar0_size > 0 {
            crate::mm::ensure_mmio_mapped(gpu.bar0, gpu.bar0_size);
        }
        let vram = gpu.bar0_size.max(64 * 1024 * 1024).min(512 * 1024 * 1024);
        let mut name = [0u8; 32];
        let label = b"Intel iGPU (soso)";
        name[..label.len()].copy_from_slice(label);
        println!("gpu: Intel detectada, VRAM estimada {} MiB", vram / (1024 * 1024));
        GpuState {
            present: true,
            vendor: 1,
            name,
            vram_total: vram,
            vram_used: 0,
            buffers: Vec::new(),
        }
    } else {
        println!("gpu: sin GPU Intel; modo CPU");
        GpuState {
            present: false,
            vendor: 0,
            name: [0; 32],
            vram_total: 0,
            vram_used: 0,
            buffers: Vec::new(),
        }
    };
    GPU.call_once(|| Mutex::new(state));
}

fn gpu() -> &'static Mutex<GpuState> {
    GPU.get().expect("gpu no inicializada")
}

pub fn info() -> GpuInfo {
    let g = gpu().lock();
    GpuInfo {
        present: g.present as u8,
        vendor: g.vendor,
        _pad: [0; 6],
        vram_total: g.vram_total,
        vram_free: g.vram_total.saturating_sub(g.vram_used),
        name: g.name,
    }
}

pub fn alloc(size: u64) -> Result<u64, i64> {
    if size == 0 {
        return Err(abi::EINVAL);
    }
    let mut g = gpu().lock();
    if !g.present {
        return Err(abi::ENOSYS);
    }
    if g.vram_used + size > g.vram_total {
        return Err(abi::ENOMEM);
    }
    let handle = g.buffers.len() as u64;
    g.buffers.push(Some(alloc::vec![0u8; size as usize]));
    g.vram_used += size;
    Ok(handle)
}

pub fn map_to_user(handle: u64, user_ptr: u64, len: u64) -> Result<u64, i64> {
    let g = gpu().lock();
    let buf = g
        .buffers
        .get(handle as usize)
        .and_then(|b| b.as_ref())
        .ok_or(abi::EINVAL)?;
    let n = len.min(buf.len() as u64) as usize;
    crate::task::with_current(|p| {
        let space = p.space.as_ref().ok_or(abi::EFAULT)?;
        space.write(user_ptr, &buf[..n]).ok_or(abi::EFAULT)?;
        Ok(0)
    })
}

pub fn submit(cmd: &[u8]) -> Result<u64, i64> {
    let g = gpu().lock();
    if !g.present {
        return Err(abi::ENOSYS);
    }
    if cmd.is_empty() {
        return Err(abi::EINVAL);
    }
    Ok(0)
}
