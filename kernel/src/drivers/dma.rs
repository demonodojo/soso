//! Asignación de páginas físicamente contiguas para DMA de dispositivos.
//!
//! Free-list compartida entre virtio, NVMe y e1000e.

use alloc::vec::Vec;
use core::ptr::NonNull;
use spin::Mutex;

pub const PAGE_SIZE: usize = 4096;

/// Dirección física (igual que virtio-drivers::PhysAddr).
pub type PhysAddr = usize;

static DMA_FREE: Mutex<Vec<(PhysAddr, usize)>> = Mutex::new(Vec::new());

/// Reserva `pages` páginas contiguas (reutiliza free-list si hay).
pub fn alloc_pages(pages: usize) -> PhysAddr {
    let pages = pages.max(1);
    let mut free = DMA_FREE.lock();
    if let Some(i) = free.iter().position(|&(_, p)| p == pages) {
        return free.swap_remove(i).0;
    }
    drop(free);
    let frame = crate::mm::FRAME_ALLOC
        .get()
        .unwrap()
        .lock()
        .allocate_contiguous(pages)
        .expect("sin memoria contigua para DMA");
    frame.start_address().as_u64() as PhysAddr
}

/// Devuelve páginas a la free-list (no al frame allocator).
pub fn free_pages(paddr: PhysAddr, pages: usize) {
    DMA_FREE.lock().push((paddr, pages.max(1)));
}

/// Vista virtual de una dirección física de RAM (mapeo del bootloader).
pub fn virt(paddr: PhysAddr) -> NonNull<u8> {
    NonNull::new(crate::mm::phys_to_virt(paddr as u64).as_mut_ptr()).unwrap()
}

/// Reserva y pone a cero (caché write-back del mapeo físico).
pub fn alloc_zeroed(pages: usize) -> (PhysAddr, NonNull<u8>) {
    let paddr = alloc_pages(pages);
    let v = virt(paddr);
    unsafe { core::ptr::write_bytes(v.as_ptr(), 0, pages * PAGE_SIZE) };
    (paddr, v)
}

/// Reserva, pone a cero y mapea en VA dedicado **sin caché**.
/// Obligatorio para colas NVMe (el dispositivo escribe CQEs).
pub fn alloc_zeroed_uc(pages: usize) -> (PhysAddr, NonNull<u8>) {
    let paddr = alloc_pages(pages);
    // Cero vía mapeo WB primero.
    unsafe {
        core::ptr::write_bytes(virt(paddr).as_ptr(), 0, pages * PAGE_SIZE);
    }
    let va = crate::mm::map_dma_uc(paddr as u64, (pages * PAGE_SIZE) as u64);
    let v = NonNull::new(va.as_mut_ptr::<u8>()).unwrap();
    (paddr, v)
}
