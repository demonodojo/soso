//! Despacho GPU vía syscalls del kernel (G5).

use alloc::vec::Vec;
use libsoso::sys;
use soso_abi as abi;
use soso_llm_core::gpu::GpuDispatch;
use soso_llm_core::layer::TensorView;

pub struct SysGpu {
    vram_free: u64,
    w_handle: u64,
    x_handle: u64,
    y_handle: u64,
}

impl SysGpu {
    pub fn new() -> Option<Self> {
        let mut info = abi::GpuInfo::default();
        if sys::gpu_info(&mut info) < 0 || info.present == 0 {
            return None;
        }
        Some(Self {
            vram_free: info.vram_free,
            w_handle: u64::MAX,
            x_handle: u64::MAX,
            y_handle: u64::MAX,
        })
    }

    fn ensure_buffer(handle: &mut u64, bytes: u64, vram_free: &mut u64) -> Result<(), ()> {
        if *handle != u64::MAX {
            return Ok(());
        }
        if bytes > *vram_free {
            return Err(());
        }
        let h = sys::gpu_alloc(bytes);
        if h < 0 {
            return Err(());
        }
        *handle = h as u64;
        *vram_free = vram_free.saturating_sub(bytes);
        Ok(())
    }

    fn write_f32(&self, handle: u64, data: &[f32]) -> Result<(), ()> {
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
        let map = sys::mmap(0, bytes.len() as u64, u64::MAX, 0);
        if map < 0 {
            return Err(());
        }
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), map as *mut u8, bytes.len());
        }
        if sys::gpu_map(handle, map as u64, bytes.len() as u64) < 0 {
            sys::munmap(map as u64, bytes.len() as u64);
            return Err(());
        }
        sys::munmap(map as u64, bytes.len() as u64);
        Ok(())
    }

    fn read_f32(&self, handle: u64, elems: usize) -> Result<Vec<f32>, ()> {
        let bytes = elems * 4;
        let map = sys::mmap(0, bytes as u64, u64::MAX, 0);
        if map < 0 {
            return Err(());
        }
        if sys::gpu_read(handle, map as u64, bytes as u64) < 0 {
            sys::munmap(map as u64, bytes as u64);
            return Err(());
        }
        let mut out = alloc::vec![0f32; elems];
        unsafe {
            let src = map as *const f32;
            for (i, slot) in out.iter_mut().enumerate() {
                *slot = src.add(i).read();
            }
        }
        sys::munmap(map as u64, bytes as u64);
        Ok(out)
    }

    fn submit_matvf(
        &self,
        w_h: u64,
        rows: u32,
        cols: u32,
        x_h: u64,
        y_h: u64,
    ) -> Result<bool, ()> {
        let mut cmd = [0u8; 37];
        cmd[0..5].copy_from_slice(b"MATVF");
        cmd[5..13].copy_from_slice(&w_h.to_le_bytes());
        cmd[13..17].copy_from_slice(&rows.to_le_bytes());
        cmd[17..21].copy_from_slice(&cols.to_le_bytes());
        cmd[21..29].copy_from_slice(&x_h.to_le_bytes());
        cmd[29..37].copy_from_slice(&y_h.to_le_bytes());
        let rc = sys::gpu_submit(&cmd);
        if rc < 0 {
            return Err(());
        }
        Ok((rc as u64) >> 32 != 0)
    }
}

impl GpuDispatch for SysGpu {
    fn available(&self) -> bool {
        true
    }

    fn matvec_f32(
        &mut self,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, ()> {
        let w = view.f32().ok_or(())?;
        if w.len() != rows * cols {
            return Err(());
        }
        let w_bytes = (rows * cols * 4) as u64;
        let x_bytes = (cols * 4) as u64;
        let y_bytes = (rows * 4) as u64;
        Self::ensure_buffer(&mut self.w_handle, w_bytes, &mut self.vram_free)?;
        Self::ensure_buffer(&mut self.x_handle, x_bytes.max(4096), &mut self.vram_free)?;
        Self::ensure_buffer(&mut self.y_handle, y_bytes.max(4096), &mut self.vram_free)?;
        self.write_f32(self.w_handle, w)?;
        self.write_f32(self.x_handle, x)?;
        self.write_f32(self.y_handle, out)?;
        let gpu = self.submit_matvf(
            self.w_handle,
            rows as u32,
            cols as u32,
            self.x_handle,
            self.y_handle,
        )?;
        let y = self.read_f32(self.y_handle, rows)?;
        out.copy_from_slice(&y);
        Ok(gpu)
    }
}
