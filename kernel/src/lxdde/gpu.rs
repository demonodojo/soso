//! Puente Rust ↔ nouveau-lx (GSP + compute G4/G5/G6).

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
    fn lx_nouveau_compute_matvec_resident(
        w_va: u64,
        rows: u32,
        cols: u32,
        x: *const f32,
        y: *mut f32,
    ) -> i32;
    fn lx_nouveau_vram_total() -> u64;
    fn lx_nouveau_device_buf_alloc(size: u64) -> u64;
    fn lx_nouveau_device_bufs_ready() -> i32;
    // `lx_nouveau_device_buf_upload` (sin offset) existe en la capa C y la usa el
    // hostcheck; desde aquí siempre se sube con offset, aunque sea 0.
    fn lx_nouveau_device_buf_upload_at(
        va: u64,
        offset: u64,
        src: *const c_void,
        size: u64,
    ) -> i32;
    fn lx_nouveau_device_buf_upload_dma(
        va: u64,
        offset: u64,
        phys: *const u64,
        npages: u32,
        size: u64,
    ) -> i32;
    fn lx_nouveau_device_buf_free(va: u64) -> i32;
    fn lx_nouveau_device_vram_free() -> u64;
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

pub fn device_vram_free() -> u64 {
    unsafe { lx_nouveau_device_vram_free() }
}

/// ¿El pool de búferes en VRAM existe? Más estricto que `gsp_ready()`, que es
/// cierto con el GSP arrancado aunque no haya llegado a haber canal ni CE.
pub fn device_bufs_ready() -> bool {
    unsafe { lx_nouveau_device_bufs_ready() != 0 }
}

pub fn device_buf_alloc(size: u64) -> Result<u64, ()> {
    let va = unsafe { lx_nouveau_device_buf_alloc(size) };
    if va == 0 {
        Err(())
    } else {
        Ok(va)
    }
}

/// Sube `data` a `offset` bytes del principio del búfer. `offset` múltiplo de
/// página; el último trozo puede no serlo.
pub fn device_buf_upload_at(va: u64, offset: u64, data: &[u8]) -> Result<(), ()> {
    if data.is_empty() {
        return Ok(());
    }
    let rc = unsafe {
        lx_nouveau_device_buf_upload_at(va, offset, data.as_ptr().cast(), data.len() as u64)
    };
    if rc < 0 { Err(()) } else { Ok(()) }
}

/// Sube `size` bytes a `offset` **sin copia de CPU**: `phys` son las páginas del
/// origen, que la capa C mapea en el espacio de la GPU para que el CE lea de
/// ellas. `Err` significa «no se cumplía algo y no se ha tocado el CE», así que
/// el llamante puede rebotar por `device_buf_upload_at`.
pub fn device_buf_upload_dma(va: u64, offset: u64, phys: &[u64], size: u64) -> Result<(), ()> {
    if phys.is_empty() || size == 0 {
        return Err(());
    }
    let rc = unsafe {
        lx_nouveau_device_buf_upload_dma(va, offset, phys.as_ptr(), phys.len() as u32, size)
    };
    if rc < 0 { Err(()) } else { Ok(()) }
}

pub fn device_buf_free(va: u64) -> Result<(), ()> {
    if unsafe { lx_nouveau_device_buf_free(va) } < 0 {
        Err(())
    } else {
        Ok(())
    }
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

pub fn submit_matvec_resident(
    w_va: u64,
    rows: usize,
    cols: usize,
    x: &[f32],
    y: &mut [f32],
) -> Result<bool, ()> {
    if w_va == 0 || x.len() != cols || y.len() != rows {
        return Err(());
    }
    let rc = unsafe {
        lx_nouveau_compute_matvec_resident(
            w_va,
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
