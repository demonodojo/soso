//! Espacio de direcciones de un proceso: un PML4 propio que comparte las
//! entradas del kernel (mitades altas) y mapea el usuario en la mitad baja.
//!
//! El kernel puede ejecutar con cualquier CR3 de proceso porque las
//! entradas L4 del kernel apuntan a las MISMAS tablas L3: cualquier cambio
//! posterior en mapeos del kernel dentro de esas tablas se ve en todos.
//! (Los mapeos del kernel no crean entradas L4 nuevas tras el arranque.)
//!
//! El PML4 se comparte entre hilos del mismo proceso vía `Arc`: el árbol
//! solo se libera cuando cae la última referencia.

use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::mm;
use spin::Mutex;
use super::mmap::{self, MmapRegion};
use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::{
    FrameAllocator, FrameDeallocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags,
    PhysFrame, Size2MiB, Size4KiB, Translate,
};
use x86_64::VirtAddr;

/// Los binarios se enlazan a esta dirección (ET_EXEC, link.ld de user/).
pub const USER_BASE: u64 = 0x40_0000;
/// Tope del código+datos+brk de un proceso.
pub const BRK_MAX: u64 = 0x6000_0000;
/// La pila del usuario: [STACK_TOP - STACK_SIZE, STACK_TOP).
pub const STACK_TOP: u64 = 0x7000_0000;
pub const STACK_SIZE: u64 = 64 * 1024;
/// Límite superior de cualquier dirección de usuario válida: toda la
/// entrada L4[0] (la ventana mmap de modelos llega hasta MMAP_LIMIT).
pub const USER_MAX: u64 = 0x80_0000_0000;

const USER_FLAGS: PageTableFlags = PageTableFlags::PRESENT
    .union(PageTableFlags::WRITABLE)
    .union(PageTableFlags::USER_ACCESSIBLE);

const USER_RDONLY: PageTableFlags = PageTableFlags::PRESENT
    .union(PageTableFlags::USER_ACCESSIBLE);

/// Regiones mmap compartidas entre hilos del mismo AddrSpace.
pub struct MmapBook {
    pub regions: Vec<MmapRegion>,
    pub next: u64,
}

struct AddrSpaceInner {
    pml4: PhysFrame,
    mmap: Mutex<MmapBook>,
}

impl Drop for AddrSpaceInner {
    fn drop(&mut self) {
        // El CR3 activo NO debe ser este espacio.
        free_user_tree(self.pml4);
    }
}

/// Espacio de direcciones clonable (hilos del mismo proceso).
#[derive(Clone)]
pub struct AddrSpace {
    inner: Arc<AddrSpaceInner>,
}

fn table_mut(frame: PhysFrame) -> &'static mut PageTable {
    unsafe { &mut *mm::phys_to_virt(frame.start_address().as_u64()).as_mut_ptr() }
}

fn free_user_tree(pml4: PhysFrame) {
    let mut fa = mm::FRAME_ALLOC.get().unwrap().lock();
    let l4 = table_mut(pml4);
    for l4e in l4.iter().take(1).filter(|e| !e.is_unused()) {
        let l3 = table_mut(PhysFrame::containing_address(l4e.addr()));
        for l3e in l3.iter().filter(|e| !e.is_unused()) {
            let l2 = table_mut(PhysFrame::containing_address(l3e.addr()));
            for l2e in l2.iter().filter(|e| !e.is_unused()) {
                if l2e.flags().contains(PageTableFlags::HUGE_PAGE) {
                    unsafe {
                        fa.deallocate_2m(PhysFrame::containing_address(l2e.addr()));
                    }
                    continue;
                }
                let l1 = table_mut(PhysFrame::containing_address(l2e.addr()));
                for l1e in l1.iter().filter(|e| !e.is_unused()) {
                    unsafe {
                        fa.deallocate_frame(PhysFrame::containing_address(l1e.addr()));
                    }
                }
                unsafe { fa.deallocate_frame(PhysFrame::containing_address(l2e.addr())) };
            }
            unsafe { fa.deallocate_frame(PhysFrame::containing_address(l3e.addr())) };
        }
        unsafe { fa.deallocate_frame(PhysFrame::containing_address(l4e.addr())) };
    }
    unsafe { fa.deallocate_frame(pml4) };
}

impl AddrSpace {
    /// PML4 nuevo con las entradas del kernel copiadas, EXCEPTO la 0: ahí
    /// vive el usuario (todo < 512 GiB).
    pub fn new() -> Option<Self> {
        let frame = mm::FRAME_ALLOC.get()?.lock().allocate_frame()?;
        let dst = table_mut(frame);
        dst.zero();
        let src = table_mut(mm::kernel_pml4());
        for i in 1..512 {
            if !src[i].is_unused() {
                dst[i] = src[i].clone();
            }
        }
        Some(Self {
            inner: Arc::new(AddrSpaceInner {
                pml4: frame,
                mmap: Mutex::new(MmapBook {
                    regions: Vec::new(),
                    next: soso_abi::MMAP_BASE,
                }),
            }),
        })
    }

    pub fn with_mmap_mut<R>(&self, f: impl FnOnce(&mut MmapBook) -> R) -> R {
        f(&mut self.inner.mmap.lock())
    }

    pub fn find_mmap_region(&self, addr: u64) -> Option<MmapRegion> {
        let book = self.inner.mmap.lock();
        mmap::find_region(&book.regions, addr).cloned()
    }

    fn pml4(&self) -> PhysFrame {
        self.inner.pml4
    }

    /// Dirección física del PML4 (clave para futex compartidos entre hilos).
    pub fn pml4_phys(&self) -> u64 {
        self.inner.pml4.start_address().as_u64()
    }

    pub fn mapper(&self) -> OffsetPageTable<'static> {
        let phys_offset = mm::phys_to_virt(0);
        unsafe { OffsetPageTable::new(table_mut(self.pml4()), phys_offset) }
    }

    /// Mapea (si no lo está ya) la página de `va` con un frame nuevo a
    /// cero. Devuelve el frame que la respalda.
    pub fn ensure_mapped(&self, va: u64) -> Option<PhysFrame> {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(va));
        let mut mapper = self.mapper();
        if let Some(frame) = mapper.translate_page(page).ok() {
            return Some(frame);
        }
        let mut fa = mm::FRAME_ALLOC.get()?.lock();
        let frame = fa.allocate_frame()?;
        unsafe {
            mm::phys_to_virt(frame.start_address().as_u64())
                .as_mut_ptr::<[u8; 4096]>()
                .write([0; 4096]);
            mapper
                .map_to_with_table_flags(page, frame, USER_FLAGS, USER_FLAGS, &mut *fa)
                .ok()?
                .ignore();
        }
        Some(frame)
    }

    /// Escribe `data` en `va` del espacio (sin necesidad de activarlo).
    pub fn write(&self, va: u64, data: &[u8]) -> Option<()> {
        let mapper = self.mapper();
        let mut off = 0usize;
        while off < data.len() {
            let dst = va + off as u64;
            let phys = mapper.translate_addr(VirtAddr::new(dst))?;
            let in_page = (4096 - (dst % 4096)) as usize;
            let n = in_page.min(data.len() - off);
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data[off..].as_ptr(),
                    mm::phys_to_virt(phys.as_u64()).as_mut_ptr(),
                    n,
                );
            }
            off += n;
        }
        Some(())
    }

    /// Mapea una página de usuario con el frame dado.
    pub fn map_page(&self, va: u64, frame: PhysFrame, writable: bool) -> Option<()> {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(va));
        let mut mapper = self.mapper();
        let flags = if writable { USER_FLAGS } else { USER_RDONLY };
        let mut fa = mm::FRAME_ALLOC.get()?.lock();
        unsafe {
            if mapper.translate_page(page).is_ok() {
                if let Ok((frame, flush)) = mapper.unmap(page) {
                    flush.flush();
                    fa.deallocate_frame(frame);
                }
            }
            mapper
                .map_to_with_table_flags(page, frame, flags, USER_FLAGS, &mut *fa)
                .ok()?
                .ignore();
        }
        Some(())
    }

    /// Mapea una página de usuario de 2 MiB con el bloque físico dado.
    pub fn map_page_2m(&self, va: u64, frame: PhysFrame, writable: bool) -> Option<()> {
        let page = Page::<Size2MiB>::containing_address(VirtAddr::new(va));
        let frame2m = PhysFrame::<Size2MiB>::containing_address(frame.start_address());
        let mut mapper = self.mapper();
        let flags = if writable { USER_FLAGS } else { USER_RDONLY };
        let mut fa = mm::FRAME_ALLOC.get()?.lock();
        unsafe {
            mapper
                .map_to_with_table_flags(page, frame2m, flags, USER_FLAGS, &mut *fa)
                .ok()?
                .ignore();
        }
        Some(())
    }

    /// Desmapea un rango de páginas (4 KiB o 2 MiB) y libera sus frames.
    pub fn unmap_range(&self, start: u64, len: u64) {
        const HUGE: u64 = 2 * 1024 * 1024;
        let mut mapper = self.mapper();
        let mut fa = mm::FRAME_ALLOC.get().unwrap().lock();
        let end = start + len;
        let mut va = start & !0xfff;
        while va < end {
            if va % HUGE == 0 && va + HUGE <= end {
                let page2m = Page::<Size2MiB>::containing_address(VirtAddr::new(va));
                if let Ok((frame, flush)) = mapper.unmap(page2m) {
                    flush.flush();
                    unsafe { fa.deallocate_2m(PhysFrame::containing_address(frame.start_address())) };
                    va += HUGE;
                    continue;
                }
            }
            let page = Page::<Size4KiB>::containing_address(VirtAddr::new(va));
            if let Ok((frame, flush)) = mapper.unmap(page) {
                flush.flush();
                unsafe { fa.deallocate_frame(frame) };
            }
            va += 4096;
        }
    }

    /// ¿Está mapeada la página que contiene `va`?
    pub fn is_mapped(&self, va: u64) -> bool {
        use x86_64::structures::paging::mapper::TranslateResult;
        matches!(
            self.mapper().translate(VirtAddr::new(va & !0xfff)),
            TranslateResult::Mapped { .. }
        )
    }

    /// Traduce una dirección de usuario a flags de la PTE.
    pub fn translate_flags(&self, va: u64) -> Option<PageTableFlags> {
        use x86_64::structures::paging::mapper::TranslateResult;
        match self.mapper().translate(VirtAddr::new(va)) {
            TranslateResult::Mapped { flags, .. } => Some(flags),
            _ => None,
        }
    }

    pub fn activate(&self) {
        unsafe { Cr3::write(self.pml4(), Cr3Flags::empty()) };
    }
}

pub fn activate_kernel() {
    unsafe { Cr3::write(mm::kernel_pml4(), Cr3Flags::empty()) };
}
