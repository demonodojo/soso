//! DMA shim.

use crate::drivers::dma;
use core::ffi::c_void;

#[unsafe(no_mangle)]
pub extern "C" fn lx_dma_alloc_coherent(
    _dev: *mut super::pci::LxPciDev,
    size: usize,
    dma_handle: *mut u64,
    _gfp: u32,
) -> *mut c_void {
    let pages = (size + 4095) / 4096;
    let (phys, ptr) = dma::alloc_zeroed_uc(pages);
    if !dma_handle.is_null() {
        unsafe { *dma_handle = phys as u64; }
    }
    ptr.as_ptr() as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_dma_free_coherent(
    _dev: *mut super::pci::LxPciDev,
    size: usize,
    _cpu_addr: *mut c_void,
    dma_handle: u64,
) {
    let pages = (size + 4095) / 4096;
    dma::free_pages(dma_handle as dma::PhysAddr, pages);
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_dma_map_single(
    _dev: *mut super::pci::LxPciDev,
    ptr: *mut c_void,
    _size: usize,
    _dir: i32,
) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    crate::mm::virt_to_phys(ptr as u64).unwrap_or(ptr as u64)
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_dma_unmap_single(
    _dev: *mut super::pci::LxPciDev,
    _dma_addr: u64,
    _size: usize,
    _dir: i32,
) {
}
