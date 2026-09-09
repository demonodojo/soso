//! Gestión de memoria. Tras `init()`, el mapper, el frame allocator y el
//! offset del mapeo físico quedan como globales: los drivers (p. ej. el Hal
//! de virtio, que es todo métodos estáticos) los necesitan sin threading de
//! referencias.

pub mod frame;
pub mod heap;
pub mod memtest;
pub mod paging;
pub mod reclaim;

use bootloader_api::BootInfo;
use frame::BootInfoFrameAllocator;
use spin::{Mutex, Once};
use x86_64::structures::paging::{
    Mapper, OffsetPageTable, Page, PageTableFlags, PhysFrame, Size4KiB, Translate,
};
use x86_64::{PhysAddr, VirtAddr};

static PHYS_OFFSET: Once<VirtAddr> = Once::new();
pub static MAPPER: Once<Mutex<OffsetPageTable<'static>>> = Once::new();
pub static FRAME_ALLOC: Once<Mutex<BootInfoFrameAllocator>> = Once::new();
static KERNEL_PML4: Once<x86_64::structures::paging::PhysFrame> = Once::new();

/// El PML4 del kernel (el de arranque): base para los de cada proceso.
pub fn kernel_pml4() -> x86_64::structures::paging::PhysFrame {
    *KERNEL_PML4.get().expect("mm sin inicializar")
}

pub fn init(boot_info: &'static mut BootInfo) {
    KERNEL_PML4.call_once(|| x86_64::registers::control::Cr3::read().0);
    let boot_info: &'static BootInfo = boot_info;
    let phys_offset = VirtAddr::new(
        boot_info
            .physical_memory_offset
            .into_option()
            .expect("el bootloader no mapeó la memoria física"),
    );
    PHYS_OFFSET.call_once(|| phys_offset);
    MAPPER.call_once(|| Mutex::new(unsafe { paging::init(phys_offset) }));
    FRAME_ALLOC.call_once(|| {
        Mutex::new(unsafe { BootInfoFrameAllocator::new(&boot_info.memory_regions) })
    });

    // La RAM utilizable se lee ANTES de tomar el candado para el mapeo: este
    // Mutex no es reentrante y pedirlo dos veces aquí colgaría el arranque en
    // silencio, que es peor que cualquier fallo de memoria.
    let usable = {
        let fa = FRAME_ALLOC.get().unwrap().lock();
        fa.total_usable_frames() as u64 * 4096
    };
    let heap = heap::init(
        &mut *MAPPER.get().unwrap().lock(),
        &mut *FRAME_ALLOC.get().unwrap().lock(),
        usable,
    )
    .expect("fallo inicializando el heap");
    crate::println!(
        "mm: heap {} MiB de {} MiB utilizables",
        heap / (1024 * 1024),
        usable / (1024 * 1024)
    );
}

/// Dirección virtual de una física, vía el mapeo completo del bootloader.
pub fn phys_to_virt(phys: u64) -> VirtAddr {
    *PHYS_OFFSET.get().expect("mm sin inicializar") + phys
}

/// Traduce VA → PA si está mapeada (RAM vía offset o identidad/MMIO).
#[allow(dead_code)]
pub fn virt_to_phys(virt: u64) -> Option<u64> {
    let va = VirtAddr::new(virt);
    let mapper = MAPPER.get()?.lock();
    mapper.translate_addr(va).map(|a| a.as_u64())
}

/// Garantiza que una región MMIO (ECAM, BARs...) es accesible vía
/// `phys_to_virt`. El mapeo del bootloader solo cubre la RAM, así que las
/// regiones de dispositivos por encima hay que mapearlas aquí, sin caché.
pub fn ensure_mmio_mapped(phys: u64, size: u64) {
    let mut mapper = MAPPER.get().unwrap().lock();
    let mut fa = FRAME_ALLOC.get().unwrap().lock();
    let start = phys & !0xfff;
    let end = (phys + size).next_multiple_of(4096);
    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::NO_CACHE
        | PageTableFlags::WRITE_THROUGH;
    for p in (start..end).step_by(4096) {
        let virt = phys_to_virt(p);
        if mapper.translate_addr(virt).is_some() {
            continue;
        }
        let page = Page::<Size4KiB>::containing_address(virt);
        let frame = PhysFrame::containing_address(PhysAddr::new(p));
        unsafe {
            mapper
                .map_to(page, frame, flags, &mut *fa)
                .expect("fallo mapeando MMIO")
                .flush();
        }
    }
}

/// Mapea frames físicos en un VA dedicado sin caché (no toca el mapeo
/// phys_to_virt del bootloader, que puede ser de 2 MiB).
/// Devuelve el VA del primer byte.
pub fn map_dma_uc(phys: u64, size: u64) -> VirtAddr {
    use core::sync::atomic::{AtomicU64, Ordering};
    use x86_64::structures::paging::PageTableFlags as F;
    /// Ventana VA alta para buffers DMA UC (fuera del identity phys map).
    static DMA_UC_NEXT: AtomicU64 = AtomicU64::new(0xFFFF_FE00_0000_0000);

    let size = size.next_multiple_of(4096);
    let virt_base = DMA_UC_NEXT.fetch_add(size, Ordering::SeqCst);
    let mut mapper = MAPPER.get().unwrap().lock();
    let mut fa = FRAME_ALLOC.get().unwrap().lock();
    let flags = F::PRESENT | F::WRITABLE | F::NO_CACHE | F::WRITE_THROUGH;
    for off in (0..size).step_by(4096) {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt_base + off));
        let frame = PhysFrame::containing_address(PhysAddr::new((phys & !0xfff) + off));
        unsafe {
            mapper
                .map_to(page, frame, flags, &mut *fa)
                .expect("map_dma_uc")
                .flush();
        }
    }
    VirtAddr::new(virt_base + (phys & 0xfff))
}

/// ¿Cabe `phys` en las direcciones físicas que esta CPU traduce? Un PTE con
/// bits por encima de MAXPHYADDR está reservado: el acceso es #PF, no un
/// error de `map_to`.
fn phys_cabe(phys: u64) -> bool {
    let w = (core::arch::x86_64::__cpuid(0x8000_0008).eax & 0xff) as u32;
    let bits = if (32..53).contains(&w) { w } else { 46 };
    let max = 1u64.checked_shl(bits).unwrap_or(u64::MAX);
    phys < max
}

pub fn map_dma_wc(phys: u64, size: u64) -> VirtAddr {
    use core::sync::atomic::{AtomicU64, Ordering};
    use x86_64::structures::paging::PageTableFlags as F;
    static DMA_WC_NEXT: AtomicU64 = AtomicU64::new(0xFFFF_FD00_0000_0000);

    if size == 0 || !phys_cabe(phys) {
        return VirtAddr::new(0);
    }
    let size = size.next_multiple_of(4096);
    let virt_base = DMA_WC_NEXT.fetch_add(size, Ordering::SeqCst);
    let mut mapper = MAPPER.get().unwrap().lock();
    let mut fa = FRAME_ALLOC.get().unwrap().lock();
    let flags = F::PRESENT | F::WRITABLE | F::NO_CACHE;
    for off in (0..size).step_by(4096) {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt_base + off));
        let frame = PhysFrame::containing_address(PhysAddr::new((phys & !0xfff) + off));
        unsafe {
            mapper
                .map_to(page, frame, flags, &mut *fa)
                .expect("map_dma_wc")
                .flush();
        }
    }
    VirtAddr::new(virt_base + (phys & 0xfff))
}

/// Mapea identidad (VA == PA) una región de RAM baja. La usa el trampolín de
/// arranque de APs (`arch::smp`): el AP activa CR3+paginación mientras el
/// puntero de instrucción sigue en la página física baja donde se copió el
/// trampolín, y el mapeo del bootloader solo cubre esa RAM vía
/// `phys_to_virt` (con offset) — sin una entrada VA==PA aquí, el primer
/// fetch tras `mov cr0` (activar PG) hace page fault y el AP triple-faultea
/// (se observa como "AB" en el puerto serie sin la "C" del modo largo).
pub fn ensure_identity_mapped(phys: u64, size: u64) {
    let mut mapper = MAPPER.get().unwrap().lock();
    let mut fa = FRAME_ALLOC.get().unwrap().lock();
    let start = phys & !0xfff;
    let end = (phys + size).next_multiple_of(4096);
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
    for p in (start..end).step_by(4096) {
        let virt = VirtAddr::new(p);
        if mapper.translate_addr(virt).is_some() {
            continue;
        }
        let page = Page::<Size4KiB>::containing_address(virt);
        let frame = PhysFrame::containing_address(PhysAddr::new(p));
        unsafe {
            mapper
                .map_to(page, frame, flags, &mut *fa)
                .expect("fallo mapeando identidad")
                .flush();
        }
    }
}
