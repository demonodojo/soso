//! Asignador de frames físicos sobre el mapa de memoria del bootloader.
//!
//! Cursor O(1) por región (el `nth()` lineal anterior era cuadrático con
//! decenas de GB) + lista de libres para los frames devueltos. Los huecos
//! que deja la asignación contigua alineada (2 MiB) se reciclan en la lista.
//!
//! Invariante: `free_frames = total_usable - cursor_consumed + free.len()`.
//! El cursor solo avanza (consume); todo lo devuelto o saltado va a `free`.

use alloc::vec::Vec;
use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use x86_64::PhysAddr;
use x86_64::structures::paging::{FrameAllocator, FrameDeallocator, PhysFrame, Size4KiB};

pub struct BootInfoFrameAllocator {
    regions: &'static MemoryRegions,
    /// Índice de la región `Usable` actual.
    region_idx: usize,
    /// Siguiente dirección física libre dentro de la región actual.
    next_addr: u64,
    /// Frames devueltos con deallocate y huecos de alineación reciclados.
    free: Vec<PhysFrame>,
    total_usable: usize,
    /// Frames que el cursor ha dejado atrás (entregados o movidos a `free`).
    cursor_consumed: usize,
    /// `free` está ordenada por dirección. Se mantiene sola mientras sólo
    /// empuje el cursor (que va en orden); un `deallocate` suelto la rompe.
    /// Sin esta marca habría que reordenar cientos de miles de frames en cada
    /// petición contigua, y eso arrastra la máquina más que el propio fallo.
    free_ordenada: bool,
}

impl BootInfoFrameAllocator {
    /// # Safety
    /// Las regiones `Usable` del mapa deben estar realmente libres y el
    /// allocator debe ser único (cada frame se entrega una sola vez).
    pub unsafe fn new(regions: &'static MemoryRegions) -> Self {
        let total_usable = regions
            .iter()
            .filter(|r| r.kind == MemoryRegionKind::Usable)
            .map(|r| ((r.end - r.start) / 4096) as usize)
            .sum();
        Self {
            regions,
            region_idx: 0,
            next_addr: 0,
            free: Vec::new(),
            total_usable,
            cursor_consumed: 0,
            free_ordenada: true,
        }
    }

    /// Devuelve un frame a la lista de libres manteniendo la marca de orden.
    fn push_free(&mut self, frame: PhysFrame) {
        if let Some(last) = self.free.last()
            && frame.start_address() < last.start_address()
        {
            self.free_ordenada = false;
        }
        self.free.push(frame);
    }

    pub fn free_frames(&self) -> usize {
        self.total_usable - self.cursor_consumed + self.free.len()
    }

    pub fn total_usable_frames(&self) -> usize {
        self.total_usable
    }

    /// Avanza el cursor hasta `count` frames contiguos alineados a
    /// `align_frames`. Los frames saltados por la alineación (y los restos
    /// de región que no caben) se reciclan en la lista de libres.
    fn take_contiguous(&mut self, count: u64, align_frames: u64) -> Option<PhysFrame> {
        let align_bytes = align_frames * 4096;
        let bytes = count * 4096;
        while self.region_idx < self.regions.len() {
            let r = &self.regions[self.region_idx];
            if r.kind != MemoryRegionKind::Usable {
                self.region_idx += 1;
                self.next_addr = 0;
                continue;
            }
            let base = self.next_addr.max(r.start).next_multiple_of(4096);
            let aligned = base.next_multiple_of(align_bytes);
            // Página reservada para el trampolín SMP (<1 MiB): saltarla.
            let tramp = crate::arch::smp::TRAMP_PHYS;
            if aligned <= tramp && tramp < aligned + bytes {
                let mut gap = base;
                while gap < tramp {
                    self.push_free(PhysFrame::containing_address(PhysAddr::new(gap)));
                    self.cursor_consumed += 1;
                    gap += 4096;
                }
                self.cursor_consumed += 1; // la propia página del trampolín
                self.next_addr = tramp + 4096;
                continue;
            }
            if aligned + bytes <= r.end {
                // reciclar el hueco [base, aligned)
                let mut gap = base;
                while gap < aligned {
                    self.push_free(PhysFrame::containing_address(PhysAddr::new(gap)));
                    self.cursor_consumed += 1;
                    gap += 4096;
                }
                self.next_addr = aligned + bytes;
                self.cursor_consumed += count as usize;
                return Some(PhysFrame::containing_address(PhysAddr::new(aligned)));
            }
            // el resto de la región no sirve para esta petición: reciclarlo
            let mut rest = base;
            while rest + 4096 <= r.end {
                self.push_free(PhysFrame::containing_address(PhysAddr::new(rest)));
                self.cursor_consumed += 1;
                rest += 4096;
            }
            self.region_idx += 1;
            self.next_addr = 0;
        }
        // Regiones vírgenes agotadas. Lo liberado sigue ahí: buscarlo.
        self.take_contiguous_from_free(count, align_frames)
    }

    /// Busca `count` frames consecutivos y alineados dentro de `free`.
    ///
    /// El cursor sólo avanza, así que sin esto, en cuanto se agotan las
    /// regiones vírgenes **ninguna petición contigua vuelve a salir adelante**
    /// por mucha memoria que se libere. Era lo que tumbaba la máquina con «sin
    /// memoria contigua para DMA» al cargar un modelo grande después de otros:
    /// el reclamo devolvía cientos de frames a `free` y `allocate_contiguous`
    /// ni los miraba. También es lo que hace reutilizable un bloque devuelto
    /// por `deallocate_2m`, que son 512 frames seguidos.
    fn take_contiguous_from_free(&mut self, count: u64, align_frames: u64) -> Option<PhysFrame> {
        let count = count as usize;
        if count == 0 || self.free.len() < count {
            return None;
        }
        if !self.free_ordenada {
            self.free
                .sort_unstable_by_key(|f| f.start_address().as_u64());
            self.free_ordenada = true;
        }
        let align = align_frames * 4096;
        let mut i = 0usize;
        while i + count <= self.free.len() {
            let base = self.free[i].start_address().as_u64();
            if align > 4096 && !base.is_multiple_of(align) {
                i += 1;
                continue;
            }
            let mut n = 1usize;
            while n < count
                && self.free[i + n].start_address().as_u64() == base + (n as u64) * 4096
            {
                n += 1;
            }
            if n == count {
                self.free.drain(i..i + count);
                return Some(PhysFrame::containing_address(PhysAddr::new(base)));
            }
            // El tramo se corta en i+n: antes de ahí no hay nada que probar.
            i += n;
        }
        None
    }

    /// `count` frames físicamente consecutivos (DMA de virtio).
    pub fn allocate_contiguous(&mut self, count: usize) -> Option<PhysFrame> {
        self.take_contiguous(count as u64, 1)
    }

    /// Bloque de 2 MiB alineado a 2 MiB (mapeos de página grande).
    pub fn allocate_2m(&mut self) -> Option<PhysFrame> {
        self.take_contiguous(512, 512)
    }

    /// Devuelve un bloque de 2 MiB: sus 512 frames van a la lista de libres.
    ///
    /// # Safety
    /// El bloque debe proceder de `allocate_2m` y no estar mapeado.
    pub unsafe fn deallocate_2m(&mut self, frame: PhysFrame) {
        let start = frame.start_address().as_u64();
        for i in 0..512u64 {
            self.push_free(PhysFrame::containing_address(PhysAddr::new(start + i * 4096)));
        }
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        if let Some(frame) = self.free.pop() {
            return Some(frame);
        }
        self.take_contiguous(1, 1)
    }
}

impl FrameDeallocator<Size4KiB> for BootInfoFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame) {
        self.push_free(frame);
    }
}
