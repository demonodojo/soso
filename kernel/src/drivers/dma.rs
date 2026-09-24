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
/// Páginas liberadas por `lx_dma_free_coherent`: no se reutilizan (cuarentena).
static DMA_QUARANTINE: Mutex<Vec<(PhysAddr, usize)>> = Mutex::new(Vec::new());

/// Saca `pages` páginas contiguas de la free-list, si hay una entrada exacta.
fn de_la_free_list(pages: usize) -> Option<PhysAddr> {
    let mut free = DMA_FREE.lock();
    let i = free.iter().position(|&(_, p)| p == pages)?;
    Some(free.swap_remove(i).0)
}

/// Pide al asignador de frames. Suelta su candado antes de volver: quien
/// reclame después lo necesita.
fn del_frame_alloc(pages: usize) -> Option<PhysAddr> {
    let frame = crate::mm::FRAME_ALLOC
        .get()?
        .lock()
        .allocate_contiguous(pages)?;
    Some(frame.start_address().as_u64() as PhysAddr)
}

/// Reserva `pages` páginas contiguas (reutiliza free-list si hay).
///
/// Si no queda memoria contigua, **reclama y reintenta** antes de rendirse.
/// El camino de mmap ya evicta páginas respaldadas por fichero cuando aprieta;
/// éste no lo hacía y moría con un `expect` en cuanto un modelo grande llenaba
/// la RAM: cargar Whisper tras el resto de la suite tumbaba la máquina con
/// «sin memoria contigua para DMA» justo cuando virtio-blk pedía un buffer.
/// `mm::reclaim::evict_batch` existía exactamente para esto —su comentario dice
/// «antes de un bloque 2 MiB grande»— y no estaba cableada en ningún sitio.
pub fn alloc_pages(pages: usize) -> PhysAddr {
    let pages = pages.max(1);
    if let Some(p) = de_la_free_list(pages) {
        return p;
    }
    if let Some(p) = del_frame_alloc(pages) {
        return p;
    }
    // Reclamar sólo fuera de IRQ dura: `evict_batch` toma candados y manda un
    // shootdown de TLB por IPI, y eso desde un handler clava la máquina.
    if !crate::arch::irq::en_irq_dura() {
        for ronda in 1..=DMA_RECLAIM_RONDAS {
            crate::mm::reclaim::evict_batch(DMA_RECLAIM_LOTE * ronda);
            if let Some(p) = del_frame_alloc(pages) {
                return p;
            }
        }
    }
    panic!("sin memoria contigua para DMA: {pages} páginas tras reclamar");
}

/// Páginas a evictar en cada ronda. La contigüidad no se consigue liberando
/// justo lo que falta: hay que soltar un lote y confiar en que caigan juntas.
const DMA_RECLAIM_LOTE: usize = 512;
const DMA_RECLAIM_RONDAS: usize = 3;

/// Devuelve páginas a la free-list (no al frame allocator).
pub fn free_pages(paddr: PhysAddr, pages: usize) {
    DMA_FREE.lock().push((paddr, pages.max(1)));
}

/// Libera DMA a cuarentena: envenena y no vuelve a `alloc_pages`.
pub fn free_pages_cuarentena(paddr: PhysAddr, pages: usize) {
    let pages = pages.max(1);
    let bytes = pages * PAGE_SIZE;
    unsafe {
        core::ptr::write_bytes(virt(paddr).as_ptr(), 0xDE, bytes);
    }
    DMA_QUARANTINE.lock().push((paddr, pages));
}

fn en_rango_pa(valor: u64, pa: PhysAddr, pages: usize) -> bool {
    let fin = pa as u64 + pages as u64 * PAGE_SIZE as u64;
    valor >= pa as u64 && valor < fin
}

/// ¿El valor cae en RAM DMA conocida (free-list, cuarentena o offset físico)?
pub fn informar_valor_dma(valor: u64) {
    if valor == 0 {
        return;
    }
    let page = valor & !0xfff;
    for (pa, pages) in DMA_QUARANTINE.lock().iter().copied() {
        if en_rango_pa(valor, pa, pages) || en_rango_pa(page, pa, pages) {
            crate::println!(
                "dma: valor {valor:#x} cae en cuarentena pa={pa:#x} {} páginas",
                pages
            );
            return;
        }
    }
    for (pa, pages) in DMA_FREE.lock().iter().copied() {
        if en_rango_pa(valor, pa, pages) || en_rango_pa(page, pa, pages) {
            crate::println!(
                "dma: valor {valor:#x} cae en DMA_FREE pa={pa:#x} {} páginas",
                pages
            );
            return;
        }
    }
    if let Some(phys) = crate::mm::virt_to_phys(valor) {
        crate::println!("dma: valor {valor:#x} traduce a phys {phys:#x}");
    } else if valor < crate::mm::heap::HEAP_START && valor > 0x1000_0000 {
        crate::println!("dma: valor {valor:#x} no es VA del heap ni mapeado como virt");
    }
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
