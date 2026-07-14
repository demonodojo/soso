//! Implementación del trait `Hal` de virtio-drivers.
//!
//! - DMA: frames físicamente contiguos, accedidos vía el mapeo físico.
//! - share/unshare: buffers de rebote (bounce). Los buffers del llamante
//!   viven en el heap y pueden cruzar páginas no contiguas físicamente;
//!   copiar a páginas DMA propias evita esa clase de corrupción.

use alloc::vec::Vec;
use core::ptr::NonNull;
use spin::Mutex;
use virtio_drivers::{BufferDirection, Hal, PAGE_SIZE, PhysAddr};

/// Free-list de bloques DMA (phys, nº de páginas) para reutilizar bounces.
static DMA_FREE: Mutex<Vec<(PhysAddr, usize)>> = Mutex::new(Vec::new());

fn alloc_dma_pages(pages: usize) -> PhysAddr {
    if let Some(i) = {
        let free = DMA_FREE.lock();
        free.iter().position(|&(_, p)| p == pages)
    } {
        return DMA_FREE.lock().swap_remove(i).0;
    }
    let frame = crate::mm::FRAME_ALLOC
        .get()
        .unwrap()
        .lock()
        .allocate_contiguous(pages)
        .expect("sin memoria contigua para DMA");
    frame.start_address().as_u64() as PhysAddr
}

fn dma_virt(paddr: PhysAddr) -> NonNull<u8> {
    NonNull::new(crate::mm::phys_to_virt(paddr as u64).as_mut_ptr()).unwrap()
}

pub struct HalImpl;

unsafe impl Hal for HalImpl {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let paddr = alloc_dma_pages(pages);
        let vptr = dma_virt(paddr);
        unsafe { core::ptr::write_bytes(vptr.as_ptr(), 0, pages * PAGE_SIZE) };
        (paddr, vptr)
    }

    unsafe fn dma_dealloc(paddr: PhysAddr, _vaddr: NonNull<u8>, pages: usize) -> i32 {
        DMA_FREE.lock().push((paddr, pages));
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, size: usize) -> NonNull<u8> {
        crate::mm::ensure_mmio_mapped(paddr as u64, size as u64);
        dma_virt(paddr)
    }

    unsafe fn share(buffer: NonNull<[u8]>, direction: BufferDirection) -> PhysAddr {
        let len = buffer.len();
        let paddr = alloc_dma_pages(len.div_ceil(PAGE_SIZE).max(1));
        if direction != BufferDirection::DeviceToDriver {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    buffer.as_ptr().cast::<u8>(),
                    dma_virt(paddr).as_ptr(),
                    len,
                );
            }
        }
        paddr
    }

    unsafe fn unshare(paddr: PhysAddr, buffer: NonNull<[u8]>, direction: BufferDirection) {
        let len = buffer.len();
        if direction != BufferDirection::DriverToDevice {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    dma_virt(paddr).as_ptr(),
                    buffer.as_ptr().cast::<u8>(),
                    len,
                );
            }
        }
        DMA_FREE.lock().push((paddr, len.div_ceil(PAGE_SIZE).max(1)));
    }
}
