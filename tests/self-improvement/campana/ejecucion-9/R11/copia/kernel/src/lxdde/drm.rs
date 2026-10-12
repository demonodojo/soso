//! G2: mini-DRM — drm_device/GEM mínimos para nvkm.

use alloc::vec::Vec;
use core::ffi::c_void;
use spin::Mutex;

#[repr(C)]
pub struct LxDrmDevice {
    pub minor: i32,
    pub dev: usize,
}

struct GemObject {
    #[allow(dead_code)]
    size: u64,
    handle: u32,
    data: Vec<u8>,
}

static DRM_DEVS: Mutex<Vec<LxDrmDevice>> = Mutex::new(Vec::new());
static GEM_OBJECTS: Mutex<Vec<GemObject>> = Mutex::new(Vec::new());

pub fn init() {}

#[unsafe(no_mangle)]
pub extern "C" fn lx_drm_dev_alloc(minor: i32) -> *mut LxDrmDevice {
    DRM_DEVS.lock().push(LxDrmDevice { minor, dev: 0 });
    DRM_DEVS.lock().last_mut().unwrap() as *mut LxDrmDevice
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_drm_gem_create(size: u64) -> u32 {
    let handle = GEM_OBJECTS.lock().len() as u32 + 1;
    GEM_OBJECTS.lock().push(GemObject {
        size,
        handle,
        data: alloc::vec![0u8; size as usize],
    });
    handle
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_drm_gem_vmap(handle: u32) -> *mut c_void {
    let objs = GEM_OBJECTS.lock();
    if let Some(obj) = objs.iter().find(|o| o.handle == handle) {
        return obj.data.as_ptr() as *mut c_void;
    }
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_map_wc(phys: u64, size: u64) -> *mut c_void {
    crate::mm::map_dma_wc(phys, size).as_mut_ptr()
}
