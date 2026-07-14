//! Asignador de frames físicos sobre el mapa de memoria del bootloader.
//!
//! Lineal, con una lista de libres para los frames devueltos por los
//! procesos de usuario al morir (fase 6).

use alloc::vec::Vec;
use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use x86_64::PhysAddr;
use x86_64::structures::paging::{FrameAllocator, FrameDeallocator, PhysFrame, Size4KiB};

pub struct BootInfoFrameAllocator {
    regions: &'static MemoryRegions,
    next: usize,
    /// Frames devueltos con deallocate_frame. Vacía hasta después de
    /// inicializar el heap (Vec::new no asigna).
    free: Vec<PhysFrame>,
}

impl BootInfoFrameAllocator {
    /// # Safety
    /// Las regiones `Usable` del mapa deben estar realmente libres y el
    /// allocator debe ser único (cada frame se entrega una sola vez).
    pub unsafe fn new(regions: &'static MemoryRegions) -> Self {
        Self { regions, next: 0, free: Vec::new() }
    }

    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> + '_ {
        self.regions
            .iter()
            .filter(|r| r.kind == MemoryRegionKind::Usable)
            .flat_map(|r| (r.start..r.end).step_by(4096))
            .map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
    }

    pub fn free_frames(&self) -> usize {
        self.usable_frames().count() - self.next + self.free.len()
    }

    /// `count` frames físicamente consecutivos (para DMA de virtio).
    pub fn allocate_contiguous(&mut self, count: usize) -> Option<PhysFrame> {
        loop {
            let first = self.usable_frames().nth(self.next)?;
            let contiguos = (1..count).all(|i| {
                self.usable_frames().nth(self.next + i).map(|f| f.start_address())
                    == Some(first.start_address() + (i as u64 * 4096))
            });
            if contiguos {
                self.next += count;
                return Some(first);
            }
            self.next += 1;
        }
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        if let Some(frame) = self.free.pop() {
            return Some(frame);
        }
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}

impl FrameDeallocator<Size4KiB> for BootInfoFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame) {
        self.free.push(frame);
    }
}
