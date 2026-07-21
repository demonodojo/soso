//! Heap del kernel: páginas mapeadas bajo demanda en el arranque y
//! gestionadas por `talc` como asignador global.

use spin::Mutex;
use talc::{ClaimOnOom, Span, Talc, Talck};
use x86_64::VirtAddr;
use x86_64::structures::paging::mapper::MapToError;
use x86_64::structures::paging::{
    FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB,
};

pub const HEAP_START: u64 = 0x_4444_4444_0000;
// La caché de sosomfs puede crecer hasta 8 MiB (2048 bloques) y la
// verificación de segmento reserva hasta 8 MiB más; 8 MiB de heap se
// agotaban con la inferencia LLM.
pub const HEAP_SIZE: u64 = 32 * 1024 * 1024;

#[global_allocator]
static ALLOCATOR: Talck<Mutex<()>, ClaimOnOom> =
    Talc::new(unsafe { ClaimOnOom::new(Span::empty()) }).lock();

pub fn init(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), MapToError<Size4KiB>> {
    let range = Page::range_inclusive(
        Page::containing_address(VirtAddr::new(HEAP_START)),
        Page::containing_address(VirtAddr::new(HEAP_START + HEAP_SIZE - 1)),
    );
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
    for page in range {
        let frame = frame_allocator
            .allocate_frame()
            .ok_or(MapToError::FrameAllocationFailed)?;
        unsafe { mapper.map_to(page, frame, flags, frame_allocator)?.flush() };
    }
    unsafe {
        ALLOCATOR
            .lock()
            .claim(Span::from_base_size(HEAP_START as *mut u8, HEAP_SIZE as usize))
            .expect("talc rechazó el span del heap");
    }
    Ok(())
}
