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
//
// 32 MiB tampoco bastan para el bring-up GSP: el ucode `gsp-570.144.bin` son
// 60,6 MiB y el port mantiene dos copias vivas (el buffer de `g_blobs` y el
// objeto GEM del staging), ~121 MiB de pico.
//
// Con pesos residentes en `GpuBuffer` (Vec en este heap), `bench` (~128 MiB)
// + GSP (~121 MiB) rompe 256 MiB a los ~11 matvec: `alloc of 4194304 bytes
// failed` (una matriz 1024×1024). 512 MiB cubre bench y deja margen; modelos
// mayores pedirán más o dejar de espejar pesos enteros en el heap del kernel.
pub const HEAP_SIZE: u64 = 512 * 1024 * 1024;

/// Suelo por debajo del cual no merece la pena seguir arrancando: con menos que
/// esto la caché de sosomfs no cabe y el fallo llegaría más tarde y peor.
const HEAP_MIN: u64 = 6 * 1024 * 1024;

#[global_allocator]
static ALLOCATOR: Talck<Mutex<()>, ClaimOnOom> =
    Talc::new(unsafe { ClaimOnOom::new(Span::empty()) }).lock();

/// Mapea el heap y se lo entrega a talc. Devuelve los bytes que quedaron.
///
/// `HEAP_SIZE` es un TECHO, no una exigencia. Antes se mapeaban los 512 MiB de
/// golpe y el primer `allocate_frame` que fallaba tiraba el arranque entero con
/// `FrameAllocationFailed` — o sea que cada vez que este número subía (8 → 32 →
/// 256 → 512 MiB, por el ucode del GSP y por `bench`) el kernel dejaba de
/// arrancar en máquinas pequeñas sin que nadie lo dijera: el guest de 48 MiB del
/// test de reclaim llevaba haciendo panic desde entonces, tapado por un timeout
/// del arnés (2026-08-01).
///
/// Se coge la MITAD de la RAM utilizable como mucho. La otra mitad no sobra: de
/// ahí salen las tablas de páginas de este mismo mapeo (~1 página por cada 512),
/// los búferes de DMA de los drivers y las páginas de los procesos.
pub fn init(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    usable_bytes: u64,
) -> Result<u64, MapToError<Size4KiB>> {
    let objetivo = core::cmp::min(HEAP_SIZE, usable_bytes / 2) & !0xfffu64;
    if objetivo < HEAP_MIN {
        return Err(MapToError::FrameAllocationFailed);
    }
    let range = Page::range_inclusive(
        Page::containing_address(VirtAddr::new(HEAP_START)),
        Page::containing_address(VirtAddr::new(HEAP_START + objetivo - 1)),
    );
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
    let mut mapeado = 0u64;
    for page in range {
        // Sin `?`: quedarse corto no es un fallo si ya hay heap suficiente. El
        // reparto de arriba deja margen, pero las tablas de páginas se pagan de
        // la misma bolsa y la cuenta exacta depende de cómo esté troceado el
        // mapa de memoria del bootloader.
        let Some(frame) = frame_allocator.allocate_frame() else {
            break;
        };
        if unsafe { mapper.map_to(page, frame, flags, frame_allocator) }.is_err() {
            break;
        }
        mapeado += 4096;
    }
    if mapeado < HEAP_MIN {
        return Err(MapToError::FrameAllocationFailed);
    }
    x86_64::instructions::tlb::flush_all();
    unsafe {
        ALLOCATOR
            .lock()
            .claim(Span::from_base_size(HEAP_START as *mut u8, mapeado as usize))
            .expect("talc rechazó el span del heap");
    }
    Ok(mapeado)
}
