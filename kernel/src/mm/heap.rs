//! Heap del kernel: páginas mapeadas bajo demanda en el arranque y
//! gestionadas por `talc` como asignador global.

use alloc::boxed::Box;
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicBool, Ordering};
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

static HEAP_MAPEADO: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

static TALC: Talck<Mutex<()>, ClaimOnOom> =
    Talc::new(unsafe { ClaimOnOom::new(Span::empty()) }).lock();

/// Ocho bytes detrás de cada reserva de Rust (`Vec`, `Box`, smoltcp). El
/// centinela de `lx_kmalloc` no cubre estas: un desbordamiento lineal pisa
/// esto antes que la etiqueta de talc, y el `malloc` siguiente lo dice con
/// tamaño y dirección de retorno en vez de petar dentro del asignador.
const CENTINELA_RUST: u64 = 0xA5A5_5A5A_A5A5_5A5A;
const NOTAS_MAX: usize = 4096;

#[derive(Clone, Copy)]
struct Nota {
    ptr: usize,
    size: usize,
    ra: u64,
}

struct Tabla {
    n: usize,
    v: [Nota; NOTAS_MAX],
}

static TABLA: Mutex<Tabla> = Mutex::new(Tabla {
    n: 0,
    v: [Nota {
        ptr: 0,
        size: 0,
        ra: 0,
    }; NOTAS_MAX],
});

static EN_REVISION: AtomicBool = AtomicBool::new(false);

const ACCESOS_HIST: usize = 8;

#[derive(Clone, Copy, Default)]
struct Acceso {
    ptr: usize,
    size: usize,
    ra: u64,
}

struct RingAccesos {
    i: usize,
    v: [Acceso; ACCESOS_HIST],
}

impl RingAccesos {
    const fn new() -> Self {
        Self {
            i: 0,
            v: [Acceso {
                ptr: 0,
                size: 0,
                ra: 0,
            }; ACCESOS_HIST],
        }
    }

    fn push(&mut self, ptr: usize, size: usize, ra: u64) {
        self.v[self.i] = Acceso { ptr, size, ra };
        self.i = (self.i + 1) % ACCESOS_HIST;
    }
}

static ULTIMOS_ALLOC: Mutex<RingAccesos> = Mutex::new(RingAccesos::new());
static ULTIMOS_FREE: Mutex<RingAccesos> = Mutex::new(RingAccesos::new());

pub fn imprimir_ultimos_accesos() {
    crate::println!("heap: últimos {ACCESOS_HIST} alloc (ptr size ra):");
    let a = ULTIMOS_ALLOC.lock();
    for n in 0..ACCESOS_HIST {
        let e = a.v[(a.i + n) % ACCESOS_HIST];
        if e.ptr != 0 {
            crate::println!("heap:   alloc {:#x} {} B ra={:#x}", e.ptr, e.size, e.ra);
        }
    }
    crate::println!("heap: últimos {ACCESOS_HIST} free (ptr size ra):");
    let f = ULTIMOS_FREE.lock();
    for n in 0..ACCESOS_HIST {
        let e = f.v[(f.i + n) % ACCESOS_HIST];
        if e.ptr != 0 {
            crate::println!("heap:   free {:#x} {} B ra={:#x}", e.ptr, e.size, e.ra);
        }
    }
}

fn registrar_alloc(ptr: usize, size: usize, ra: u64) {
    ULTIMOS_ALLOC.lock().push(ptr, size, ra);
}

fn registrar_free(ptr: usize, size: usize, ra: u64) {
    ULTIMOS_FREE.lock().push(ptr, size, ra);
}

/// Bytes del heap ya mapeados (para auditoría de talc).
pub fn heap_mapeado_bytes() -> u64 {
    HEAP_MAPEADO.load(Ordering::Acquire)
}

/// `true` si alguien tiene tomado el candado de talc (desde el handler de
/// `#DB`: talc escribe `bins` siempre con él tomado; nadie más debería).
pub fn talc_bloqueado() -> bool {
    match TALC.try_lock() {
        Some(guard) => {
            drop(guard);
            false
        }
        None => true,
    }
}

/// Un `next`/cabecera de bin válido: `None` o un puntero dentro del heap.
pub fn puntero_de_heap_valido(v: u64) -> bool {
    v == 0 || (v >= HEAP_START && v < HEAP_START + heap_mapeado_bytes())
}

/// Dirección de `bins` de talc: `claim` deja la tabla justo detrás de la
/// etiqueta base del primer heap (`HEAP_START + TAG_SIZE`).
pub const TALC_BINS: u64 = HEAP_START + 8;

pub(super) fn with_talc_audit(f: impl FnOnce(&Talc<ClaimOnOom>)) {
    if let Some(guard) = TALC.try_lock() {
        f(&*guard);
    } else {
        crate::println!("heap: auditoría talc sin candado");
    }
}

struct ConCentinela;

#[global_allocator]
static ALLOCATOR: ConCentinela = ConCentinela;

#[inline(never)]
fn ra_de_quien_reserva() -> u64 {
    // `__rust_alloc` es un envoltorio. En la pila hay varias direcciones del
    // kernel; la tercera suele ser el `Vec`/`Box` que pidió la memoria.
    let rsp: u64;
    unsafe {
        core::arch::asm!(
            "mov {}, rsp",
            out(reg) rsp,
            options(nostack, preserves_flags, readonly),
        );
        let mut vistos = 0u32;
        for i in 0..48 {
            let v = core::ptr::read_volatile(rsp.wrapping_add(i * 8) as *const u64);
            if (0x1000_0000_0000..0x1000_0060_0000).contains(&v) {
                vistos += 1;
                if vistos == 3 {
                    return v;
                }
            }
        }
    }
    0
}

fn con_cola(layout: Layout) -> Option<Layout> {
    let total = layout.size().checked_add(8)?;
    Layout::from_size_align(total, layout.align()).ok()
}

fn anotar(ptr: usize, size: usize, ra: u64) {
    let mut t = TABLA.lock();
    if let Some(slot) = t.v.iter_mut().find(|n| n.ptr == 0) {
        *slot = Nota { ptr, size, ra };
        t.n += 1;
        return;
    }
    drop(t);
    panic!("heap: tabla de centinelas llena ({NOTAS_MAX})");
}

fn quitar(ptr: usize) {
    let mut t = TABLA.lock();
    if let Some(slot) = t.v.iter_mut().find(|n| n.ptr == ptr) {
        *slot = Nota {
            ptr: 0,
            size: 0,
            ra: 0,
        };
        t.n = t.n.saturating_sub(1);
    }
}

/// Recorre las reservas vivas. Si un centinela cambió, el bloque de `size`
/// bytes que empieza en `ptr` se escribió de más; `ra` es quien lo reservó.
fn revisar() {
    if EN_REVISION.swap(true, Ordering::Relaxed) {
        return;
    }
    let roto = {
        let t = TABLA.lock();
        let mut hallado = None;
        for n in t.v.iter() {
            if n.ptr == 0 {
                continue;
            }
            let visto = unsafe { core::ptr::read_unaligned((n.ptr + n.size) as *const u64) };
            if visto != CENTINELA_RUST {
                hallado = Some((*n, visto));
                break;
            }
        }
        hallado
    };
    if let Some((n, visto)) = roto {
        crate::println!(
            "heap: BLOQUE DESBORDADO en {:#x} ({} B) ra={:#x} centinela={:#x}",
            n.ptr,
            n.size,
            n.ra,
            visto
        );
        panic!(
            "heap escribió más allá de un bloque de {} B en {:#x} (ra {:#x})",
            n.size, n.ptr, n.ra
        );
    }
    EN_REVISION.store(false, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for ConCentinela {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        revisar();
        let Some(grande) = con_cola(layout) else {
            return core::ptr::null_mut();
        };
        let p = unsafe { TALC.alloc(grande) };
        if !p.is_null() {
            unsafe {
                core::ptr::write_unaligned(p.add(layout.size()) as *mut u64, CENTINELA_RUST);
            }
            let ra = ra_de_quien_reserva();
            anotar(p as usize, layout.size(), ra);
            registrar_alloc(p as usize, layout.size(), ra);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        revisar();
        let Some(grande) = con_cola(layout) else {
            return;
        };
        let ra = ra_de_quien_reserva();
        registrar_free(ptr as usize, layout.size(), ra);
        quitar(ptr as usize);
        unsafe { TALC.dealloc(ptr, grande) };
    }
}

/// Recorre los `next` de los huecos libres de talc (véase `heap_talc_audit`).
pub fn vigilar_huecos(contexto: &str) {
    if vigilando() {
        super::heap_talc_audit::vigilar_huecos(contexto);
    }
}

/// Fuerza a Talc a recorrer todas sus listas libres y deja un marcador antes
/// y después. En `dev`, `malloc` llama a `scan_for_errors()` antes de tocar el
/// heap: si una liberación anterior dejó un enlace inválido, el segundo
/// marcador no llega a imprimirse y el contexto del primero identifica la
/// operación culpable.
///
/// La sonda se conserva deliberadamente. Liberarla aquí convertiría su propio
/// `free` en la última mutación sin validar y volvería ambiguo el diagnóstico.
/// Son ocho bytes por punto de comprobación, sólo usado por la traza temporal
/// del DNS que reproduce el panic en hardware.
/// Activo si el kernel se compiló con `SOSO_HEAP_DEBUG=1` (xtask/build).
pub const fn debug_enabled() -> bool {
    cfg!(soso_heap_debug)
}

/// Sonda del heap del kernel (`talc`). Sin el cfg es no-op.
pub fn audit(contexto: &str) {
    if debug_enabled() {
        comprobar_listas(contexto);
    }
}

pub fn comprobar_listas(contexto: &str) {
    crate::println!("heap: comprobando listas {contexto}");
    let sonda = Box::new(0x534f_534f_4845_4150u64);
    crate::println!("heap: listas OK {contexto} sonda={:p}", sonda.as_ref());
}

/// Se enciende en el DNS y en el `connect` del HTTPS. Mientras está puesta,
/// `net::poll` y la reserva del búfer TCP comprueban las listas libres.
static VIGILAR: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

static ULTIMO_CTX: spin::Mutex<[u8; 24]> = spin::Mutex::new([0; 24]);

pub fn vigilar() {
    VIGILAR.store(true, core::sync::atomic::Ordering::Relaxed);
}

pub fn vigilando() -> bool {
    VIGILAR.load(core::sync::atomic::Ordering::Relaxed)
}

/// Reserva una sonda y la suelta. El nombre se guarda **después** de
/// reservar: si este `malloc` peta, el pánico sigue mostrando el punto
/// anterior. No se filtra: cada `poll` la dejaba viva y llenaba la tabla
/// de centinelas a mitad de la descarga.
pub fn punto(contexto: &str) {
    let sonda = Box::new(0x534f_534f_4845_4150u64);
    drop(sonda);
    vigilar_huecos(contexto);
    let bytes = contexto.as_bytes();
    let n = bytes.len().min(24);
    let mut buf = [0u8; 24];
    buf[..n].copy_from_slice(&bytes[..n]);
    *ULTIMO_CTX.lock() = buf;
}

pub fn ultimo_punto() -> [u8; 24] {
    *ULTIMO_CTX.lock()
}

/// Localiza el valor que provocó el #PF en la RAM del heap. Permite distinguir
/// un enlace almacenado de un puntero estropeado sólo en registros/pila.
/// No reserva, no toma el candado de talc y no desreferencia el valor buscado.
/// Se limita al rango realmente mapeado y a ocho coincidencias. Una coincidencia
/// no prueba quién escribió el dato; una ausencia tampoco excluye una carrera
/// con otro core ni una dirección calculada (CR2 = puntero + desplazamiento).
pub fn localizar_en_pf(valor: u64) {
    let bytes = HEAP_MAPEADO.load(Ordering::Acquire);
    if bytes == 0 || valor == 0 {
        return;
    }
    let fin = HEAP_START + bytes;
    let mut cursor = HEAP_START;
    let mut encontrados = 0;
    crate::println!("heap: buscando CR2={valor:#x} en {} MiB (qwords alineados)", bytes >> 20);
    while cursor < fin && encontrados < 8 {
        let indice = unsafe {
            super::heap_scan::buscar(cursor as *const u64, ((fin - cursor) / 8) as usize, valor)
        };
        let Some(indice) = indice else { break };
        let sitio = cursor + indice as u64 * 8;
        encontrados += 1;
        crate::println!("heap: coincidencia {encontrados} en {sitio:#x}");
        // La foto pierde dígitos: decir en claro si el qword es una cabecera
        // de bin de talc (tabla de 128 en `TALC_BINS`), que es lo que apuntan
        // las coincidencias en `0x44444444…` de las fotos de las 17:11 y 19:33.
        if sitio >= TALC_BINS && sitio < TALC_BINS + 128 * 8 {
            crate::println!("heap: la coincidencia es bins[{}] de talc", (sitio - TALC_BINS) / 8);
        }
        let desde = sitio.saturating_sub(16).max(HEAP_START);
        let hasta = (sitio + 32).min(fin);
        let mut p = desde;
        while p < hasta {
            let dato = unsafe { core::ptr::read_volatile(p as *const u64) };
            crate::println!("heap: [{p:#x}]={dato:#018x}");
            p += 8;
        }
        cursor = sitio + 8;
    }
    crate::println!("heap: búsqueda terminada ({encontrados} coincidencias, máximo 8)");
}

/// ¿El puntero encaja en RAM DMA (iwlwifi, free-list, cuarentena)?
pub fn informar_dma_en_pf(valor: u64) {
    #[cfg(feature = "lxdde")]
    crate::lxdde::informar_dma_en_pf(valor);
    #[cfg(not(feature = "lxdde"))]
    crate::drivers::dma::informar_valor_dma(valor);
}

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
    HEAP_MAPEADO.store(mapeado, Ordering::Release);
    unsafe {
        TALC
            .lock()
            .claim(Span::from_base_size(HEAP_START as *mut u8, mapeado as usize))
            .expect("talc rechazó el span del heap");
    }
    // Las fotos del #PF de red sitúan el valor malo en `bins[0]` (y una vez
    // en `bins[1]`): watchpoint de escritura sobre ambas cabeceras.
    crate::arch::hwbp::vigilar_escritura(0, TALC_BINS);
    crate::arch::hwbp::vigilar_escritura(1, TALC_BINS + 8);
    Ok(mapeado)
}
