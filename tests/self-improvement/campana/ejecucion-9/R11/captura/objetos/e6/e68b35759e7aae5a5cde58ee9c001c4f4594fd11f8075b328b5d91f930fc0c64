//! Implementación del trait `Hal` de virtio-drivers.
//!
//! - DMA: vía `drivers::dma` (frames contiguos).
//! - share/unshare: buffers de rebote (bounce).

use core::ptr::NonNull;
use virtio_drivers::{BufferDirection, Hal, PAGE_SIZE, PhysAddr};

use crate::drivers::dma;

pub struct HalImpl;

unsafe impl Hal for HalImpl {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let (paddr, vptr) = dma::alloc_zeroed(pages);
        (paddr as PhysAddr, vptr)
    }

    unsafe fn dma_dealloc(paddr: PhysAddr, _vaddr: NonNull<u8>, pages: usize) -> i32 {
        dma::free_pages(paddr as dma::PhysAddr, pages);
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, size: usize) -> NonNull<u8> {
        crate::mm::ensure_mmio_mapped(paddr as u64, size as u64);
        dma::virt(paddr as dma::PhysAddr)
    }

    unsafe fn share(buffer: NonNull<[u8]>, direction: BufferDirection) -> PhysAddr {
        let len = buffer.len();
        let pages = len.div_ceil(PAGE_SIZE).max(1);
        let paddr = dma::alloc_pages(pages);
        if direction != BufferDirection::DeviceToDriver {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    buffer.as_ptr().cast::<u8>(),
                    dma::virt(paddr).as_ptr(),
                    len,
                );
            }
        }
        paddr as PhysAddr
    }

    unsafe fn unshare(paddr: PhysAddr, buffer: NonNull<[u8]>, direction: BufferDirection) {
        let len = buffer.len();
        if direction != BufferDirection::DriverToDevice {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    dma::virt(paddr as dma::PhysAddr).as_ptr(),
                    buffer.as_ptr().cast::<u8>(),
                    len,
                );
            }
        }
        dma::free_pages(paddr as dma::PhysAddr, len.div_ceil(PAGE_SIZE).max(1));
    }
}
