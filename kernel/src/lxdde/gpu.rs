//! Puente Rust ↔ nouveau-lx (GSP + compute G4/G5).

use core::ffi::c_void;

unsafe extern "C" {
    fn lx_nouveau_gsp_is_ready() -> i32;
    fn lx_nouveau_gsp_phase() -> *const u8;
    fn lx_nouveau_compute_saxpy(a: f32, x: *const f32, y: *mut f32, n: u32) -> i32;
    fn lx_nouveau_compute_matvec_f32(
        w: *const f32,
        rows: u32,
        cols: u32,
        x: *const f32,
        y: *mut f32,
    ) -> i32;
    fn lx_nouveau_vram_total() -> u64;
    fn lx_nouveau_set_boot0(boot0: u32, device_id: u32);
    fn lx_nouveau_gsp_fini() -> i32;
}

pub fn notify_boot0(boot0: u32, device_id: u16) {
    unsafe {
        lx_nouveau_set_boot0(boot0, device_id as u32);
    }
}

pub fn gsp_ready() -> bool {
    unsafe { lx_nouveau_gsp_is_ready() != 0 }
}

pub fn gsp_phase() -> &'static str {
    unsafe {
        let p = lx_nouveau_gsp_phase();
        if p.is_null() {
            return "?";
        }
        let mut len = 0usize;
        while *p.add(len) != 0 && len < 32 {
            len += 1;
        }
        core::str::from_utf8(core::slice::from_raw_parts(p, len)).unwrap_or("?")
    }
}

pub fn vram_total() -> u64 {
    unsafe { lx_nouveau_vram_total() }
}

/// Apaga GSP-RM y deja la GPU sin DMA. **Sin vuelta atrás** en este arranque:
/// tras esto `gsp_ready()` es falso y el cómputo cae a CPU.
///
/// Hay que llamarlo antes de que el host suelte la tarjeta. Un reset de
/// vfio-pci sobre un GSP vivo colgó el host el 2026-07-25 (ver `gsp_fini.h`).
pub fn gsp_fini() -> bool {
    unsafe { lx_nouveau_gsp_fini() == 0 }
}

pub fn submit_saxpy(a: f32, x: &[f32], y: &mut [f32]) -> Result<bool, ()> {
    if x.len() != y.len() || x.is_empty() {
        return Err(());
    }
    let rc = unsafe {
        lx_nouveau_compute_saxpy(a, x.as_ptr(), y.as_mut_ptr(), x.len() as u32)
    };
    if rc < 0 {
        Err(())
    } else {
        Ok(rc > 0)
    }
}

pub fn submit_matvec_f32(w: &[f32], rows: usize, cols: usize, x: &[f32], y: &mut [f32]) -> Result<bool, ()> {
    if w.len() != rows * cols || x.len() != cols || y.len() != rows {
        return Err(());
    }
    let rc = unsafe {
        lx_nouveau_compute_matvec_f32(
            w.as_ptr(),
            rows as u32,
            cols as u32,
            x.as_ptr(),
            y.as_mut_ptr(),
        )
    };
    if rc < 0 {
        Err(())
    } else {
        Ok(rc > 0)
    }
}

#[allow(dead_code)]
pub fn init_module() -> i32 {
    unsafe extern "C" {
        fn lx_nouveau_init_module() -> i32;
    }
    unsafe { lx_nouveau_init_module() }
}
