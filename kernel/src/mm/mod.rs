//! Gestión de memoria. Tras `init()`, el mapper, el frame allocator y el
//! offset del mapeo físico quedan como globales: los drivers (p. ej. el Hal
//! de virtio, que es todo métodos estáticos) los necesitan sin threading de
//! referencias.

pub mod frame;
pub mod heap;
pub mod paging;

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

    heap::init(
        &mut *MAPPER.get().unwrap().lock(),
        &mut *FRAME_ALLOC.get().unwrap().lock(),
    )
    .expect("fallo inicializando el heap");
}

/// Dirección virtual de una física, vía el mapeo completo del bootloader.
pub fn phys_to_virt(phys: u64) -> VirtAddr {
    *PHYS_OFFSET.get().expect("mm sin inicializar") + phys
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
