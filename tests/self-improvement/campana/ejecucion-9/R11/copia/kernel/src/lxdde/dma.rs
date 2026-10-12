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

/// Memoria DMA con el mapeo de CPU cacheado (write-back).
///
/// En x86 el DMA de PCIe es coherente —el dispositivo fisga la caché—, así que
/// `dma_alloc_coherent` de Linux devuelve memoria normal WB; nuestro shim la da
/// UC porque el primer usuario fueron anillos donde el dispositivo escribe. Para
/// el búfer de rebote de las subidas a VRAM eso es carísimo: la CPU escribe
/// megabytes y a UC eso son ~100 MB/s. Quien use esto tiene que poner una
/// barrera antes de picar el timbre del dispositivo (el `mfence` de la subida).
#[unsafe(no_mangle)]
pub extern "C" fn lx_dma_alloc_wb(
    _dev: *mut super::pci::LxPciDev,
    size: usize,
    dma_handle: *mut u64,
    _gfp: u32,
) -> *mut c_void {
    let pages = (size + 4095) / 4096;
    let (phys, ptr) = dma::alloc_zeroed(pages);
    if !dma_handle.is_null() {
        unsafe { *dma_handle = phys as u64; }
    }
    ptr.as_ptr() as *mut c_void
}

/// Baja a memoria las líneas de caché de `[ptr, ptr+len)` y ordena la escritura.
///
/// Es lo que hace segura la memoria WB de arriba. En x86 el DMA de PCIe fisga la
/// caché, pero una GPU NVIDIA puede emitir transacciones **no-snoop** (el bit de
/// PCIe Device Control), y entonces leería la RAM y no nuestra línea sucia: el
/// búfer subiría bytes viejos sin que nada fallara, que es la peor forma de
/// fallar. `clflush` deja el rango en RAM cueste lo que cueste, y sigue siendo
/// mucho más barato que escribirlo todo por memoria UC.
#[unsafe(no_mangle)]
pub extern "C" fn lx_dma_flush_range(ptr: *const c_void, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    const LINE: usize = 64;
    let start = (ptr as usize) & !(LINE - 1);
    let end = (ptr as usize) + len;
    let mut p = start;
    while p < end {
        unsafe { core::arch::x86_64::_mm_clflush(p as *const u8) };
        p += LINE;
    }
    unsafe { core::arch::x86_64::_mm_sfence() };
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
