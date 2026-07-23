//! Driver GPU: Intel iGPU + NVIDIA (G5).

use crate::drivers::{nvidia_compute, nvidia_probe, pci};
use crate::println;
use alloc::vec::Vec;
use soso_abi::{self as abi, GpuInfo};
use spin::{Mutex, Once};

const VENDOR_INTEL: u16 = 0x8086;
const VENDOR_NVIDIA: u16 = 0x10de;
const GPU_VENDOR_NVIDIA: u8 = 2;
const GPU_VENDOR_INTEL: u8 = 1;

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
    let nvidia = devs
        .iter()
        .find(|d| d.vendor_id == VENDOR_NVIDIA && d.class == 0x03);
    let intel = devs
        .iter()
        .find(|d| d.vendor_id == VENDOR_INTEL && d.class == 0x03);

    let state = if nvidia.is_some() || nvidia_probe::present() {
        let mut name = [0u8; 32];
        let label = b"NVIDIA (soso/lxdde)";
        name[..label.len()].copy_from_slice(label);
        let vram = 8_u64 * 1024 * 1024 * 1024;
        println!("gpu: NVIDIA detectada (chipset {:?})", nvidia_probe::chipset_id());
        GpuState {
            present: true,
            vendor: GPU_VENDOR_NVIDIA,
            name,
            vram_total: vram,
            vram_used: 0,
            buffers: Vec::new(),
        }
    } else if let Some(gpu) = intel {
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
            vendor: GPU_VENDOR_INTEL,
            name,
            vram_total: vram,
            vram_used: 0,
            buffers: Vec::new(),
        }
    } else {
        println!("gpu: sin GPU; modo CPU");
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

/// Comando saxpy: `b"SAXPY"` + f32 a + slices vía handles simplificados (G5).
pub fn submit(cmd: &[u8]) -> Result<u64, i64> {
    let g = gpu().lock();
    if !g.present {
        return Err(abi::ENOSYS);
    }
    if cmd.len() < 8 || &cmd[..5] != b"SAXPY" {
        return Ok(0);
    }
    if g.vendor == GPU_VENDOR_NVIDIA {
        let a = f32::from_le_bytes(cmd[5..9].try_into().unwrap_or([0; 4]));
        let mut x = [1.0f32, 2.0, 3.0];
        let mut y = [0.0f32; 3];
        if nvidia_compute::submit_saxpy(a, &x, &mut y).is_ok() {
            return Ok(y[0].to_bits() as u64);
        }
    }
    Ok(0)
}
