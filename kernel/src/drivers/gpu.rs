//! Driver GPU: NVIDIA (L6) + Intel iGPU fallback.

use crate::drivers::{nvidia_compute, nvidia_probe, pci};
use crate::println;
use alloc::vec::Vec;
use soso_abi::{self as abi, GpuInfo};
use spin::{Mutex, Once};

const VENDOR_INTEL: u16 = 0x8086;
const VENDOR_NVIDIA: u16 = 0x10de;
const GPU_VENDOR_NVIDIA: u8 = 2;
const GPU_VENDOR_INTEL: u8 = 1;

struct GpuBuffer {
    data: Vec<u8>,
}

struct GpuState {
    present: bool,
    vendor: u8,
    name: [u8; 32],
    vram_total: u64,
    vram_used: u64,
    buffers: Vec<Option<GpuBuffer>>,
}

static GPU: Once<Mutex<GpuState>> = Once::new();

fn nvidia_vram() -> u64 {
    #[cfg(feature = "lxdde")]
    {
        let v = crate::lxdde::nouveau_vram_total();
        if v > 0 {
            return v;
        }
    }
    8_u64 * 1024 * 1024 * 1024
}

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
        let label = b"NVIDIA GB205 (soso/lxdde)";
        name[..label.len()].copy_from_slice(label);
        let vram = nvidia_vram();
        println!(
            "gpu: NVIDIA detectada (chipset {:?}, GSP={})",
            nvidia_probe::chipset_id(),
            gsp_label()
        );
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

fn gsp_label() -> &'static str {
    #[cfg(feature = "lxdde")]
    {
        return crate::lxdde::gsp_phase();
    }
    #[cfg(not(feature = "lxdde"))]
    "off"
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
    g.buffers.push(Some(GpuBuffer {
        data: alloc::vec![0u8; size as usize],
    }));
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
    let n = len.min(buf.data.len() as u64) as usize;
    crate::task::with_current(|p| {
        let space = p.space.as_ref().ok_or(abi::EFAULT)?;
        space.write(user_ptr, &buf.data[..n]).ok_or(abi::EFAULT)?;
        Ok(0)
    })
}

pub fn upload_from_user(handle: u64, user_ptr: u64, len: u64) -> Result<u64, i64> {
    let mut g = gpu().lock();
    let slot = g
        .buffers
        .get_mut(handle as usize)
        .and_then(|b| b.as_mut())
        .ok_or(abi::EINVAL)?;
    let n = len.min(slot.data.len() as u64) as usize;
    crate::task::with_current(|p| {
        let space = p.space.as_ref().ok_or(abi::EFAULT)?;
        space.read(user_ptr, &mut slot.data[..n]).ok_or(abi::EFAULT)?;
        Ok(0)
    })
}

/// Comandos GPU (userspace):
/// - `b"SAXPY"` + f32 a + u64 x_handle + u64 y_handle + u32 n
/// - `b"MATVF"` + u64 w_handle + u32 rows + u32 cols + u64 x_handle + u64 y_handle
pub fn submit(cmd: &[u8]) -> Result<u64, i64> {
    let g = gpu().lock();
    if !g.present {
        return Err(abi::ENOSYS);
    }
    if g.vendor != GPU_VENDOR_NVIDIA {
        return Ok(0);
    }
    if cmd.len() >= 5 && &cmd[..5] == b"SAXPY" && cmd.len() >= 29 {
        let a = f32::from_le_bytes(cmd[5..9].try_into().unwrap_or([0; 4]));
        let x_h = u64::from_le_bytes(cmd[9..17].try_into().unwrap_or([0; 8]));
        let y_h = u64::from_le_bytes(cmd[17..25].try_into().unwrap_or([0; 8]));
        let n = u32::from_le_bytes(cmd[25..29].try_into().unwrap_or([0; 4])) as usize;
        let x = read_f32_buffer(&g, x_h, n)?;
        let mut y = read_f32_buffer(&g, y_h, n)?;
        drop(g);
        let on_gpu = nvidia_compute::submit_saxpy(a, &x, &mut y).map_err(|_| abi::EIO)?;
        write_f32_buffer(&mut gpu().lock(), y_h, &y)?;
        return Ok(((on_gpu as u64) << 32) | y[0].to_bits() as u64);
    }
    if cmd.len() >= 5 && &cmd[..5] == b"MATVF" && cmd.len() >= 37 {
        let w_h = u64::from_le_bytes(cmd[5..13].try_into().unwrap_or([0; 8]));
        let rows = u32::from_le_bytes(cmd[13..17].try_into().unwrap_or([0; 4])) as usize;
        let cols = u32::from_le_bytes(cmd[17..21].try_into().unwrap_or([0; 4])) as usize;
        let x_h = u64::from_le_bytes(cmd[21..29].try_into().unwrap_or([0; 8]));
        let y_h = u64::from_le_bytes(cmd[29..37].try_into().unwrap_or([0; 8]));
        let w = read_f32_buffer(&g, w_h, rows * cols)?;
        let x = read_f32_buffer(&g, x_h, cols)?;
        let mut y = read_f32_buffer(&g, y_h, rows)?;
        drop(g);
        let on_gpu = nvidia_compute::submit_matvec_f32(&w, rows, cols, &x, &mut y).map_err(|_| abi::EIO)?;
        write_f32_buffer(&mut gpu().lock(), y_h, &y)?;
        return Ok((on_gpu as u64) << 32);
    }
    // Legacy: SAXPY con datos embebidos (tests)
    if cmd.len() >= 8 && &cmd[..5] == b"SAXPY" {
        let a = f32::from_le_bytes(cmd[5..9].try_into().unwrap_or([0; 4]));
        let mut x = [1.0f32, 2.0, 3.0];
        let mut y = [0.0f32; 3];
        if nvidia_compute::submit_saxpy(a, &x, &mut y).is_ok() {
            return Ok(y[0].to_bits() as u64);
        }
    }
    Ok(0)
}

fn read_f32_buffer(g: &GpuState, handle: u64, elems: usize) -> Result<Vec<f32>, i64> {
    let bytes = elems * 4;
    let buf = g
        .buffers
        .get(handle as usize)
        .and_then(|b| b.as_ref())
        .ok_or(abi::EINVAL)?;
    if buf.data.len() < bytes {
        return Err(abi::EINVAL);
    }
    let mut out = alloc::vec![0f32; elems];
    for (i, chunk) in buf.data[..bytes].chunks_exact(4).enumerate() {
        out[i] = f32::from_le_bytes(chunk.try_into().unwrap());
    }
    Ok(out)
}

fn write_f32_buffer(g: &mut GpuState, handle: u64, data: &[f32]) -> Result<(), i64> {
    let slot = g.buffers.get_mut(handle as usize).and_then(|b| b.as_mut()).ok_or(abi::EINVAL)?;
    let bytes = data.len() * 4;
    if slot.data.len() < bytes {
        return Err(abi::EINVAL);
    }
    for (i, &v) in data.iter().enumerate() {
        slot.data[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    Ok(())
}
