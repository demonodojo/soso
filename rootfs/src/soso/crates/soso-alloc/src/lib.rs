//! Alocador de userspace compartido por `libsoso` y por `libstd` (vía `soso-rt`).
//!
//! La política es la que ya medía libsoso: por debajo de 1 MiB, primera
//! coincidencia en una lista enlazada alimentada de una vez por `sbrk`; si no
//! cabe o la alineación pasa de una página, un arena que pide el break en
//! trozos de 256 KiB; desde 1 MiB, `mmap` anónimo. Un solo estado por
//! ejecutable: estas estáticas. Quien enlaza el crate no registra otro.

#![no_std]

#[cfg(soso_heap_debug)]
mod heap_debug;

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use linked_list_allocator::Heap;
use soso_abi as abi;

/// Reserva inicial del heap de lista. Idempotente: la segunda llamada no pide
/// otro `sbrk`.
const HEAP_INIT_BYTES: usize = 1024 * 1024;
/// Lo que crece el montón de lista cada vez que se queda sin hueco.
const HEAP_GROW_BYTES: usize = 4 * 1024 * 1024;

/// Umbral a partir del cual una reserva va a mmap anónimo.
pub const MMAP_ALLOC_MIN: usize = 1024 * 1024;

/// Lo que se le pide al kernel de una vez en el camino del arena.
const ARENA_CHUNK: usize = 256 * 1024;

struct Ops {
    sbrk: fn(i64) -> i64,
    mmap: fn(u64, u64, u64, u64) -> i64,
    munmap: fn(u64, u64) -> i64,
}

static OPS: AtomicPtr<Ops> = AtomicPtr::new(core::ptr::null_mut());

static SOSO_OPS: Ops = Ops {
    sbrk: sbrk_soso,
    mmap: mmap_soso,
    munmap: munmap_soso,
};

fn ops() -> &'static Ops {
    let p = OPS.load(Ordering::Acquire);
    if p.is_null() {
        &SOSO_OPS
    } else {
        unsafe { &*p }
    }
}

fn instalar(ops: &'static Ops) {
    OPS.store(core::ptr::from_ref(ops) as *mut Ops, Ordering::Release);
}

fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    let ret: i64;
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr => ret,
            inlateout("rdi") a1 => _,
            inlateout("rsi") a2 => _,
            inlateout("rdx") a3 => _,
            inlateout("r10") a4 => _,
            lateout("rcx") _,
            lateout("r11") _,
            lateout("r8") _,
            lateout("r9") _,
            // El kernel usa SSE: los registros vectoriales no sobreviven.
            clobber_abi("C"),
        );
    }
    ret
}

fn sbrk_soso(delta: i64) -> i64 {
    syscall4(abi::SYS_SBRK, delta as u64, 0, 0, 0)
}

fn mmap_soso(addr: u64, len: u64, fd: u64, offset: u64) -> i64 {
    syscall4(abi::SYS_MMAP, addr, len, fd, offset)
}

fn munmap_soso(addr: u64, len: u64) -> i64 {
    syscall4(abi::SYS_MUNMAP, addr, len, 0, 0)
}

struct Candado<T> {
    lock: AtomicBool,
    valor: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for Candado<T> {}

impl<T> Candado<T> {
    const fn new(valor: T) -> Self {
        Self {
            lock: AtomicBool::new(false),
            valor: UnsafeCell::new(valor),
        }
    }

    fn lock(&self) {
        while self
            .lock
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    fn unlock(&self) {
        self.lock.store(false, Ordering::Release);
    }
}

static HEAP: Candado<Heap> = Candado::new(Heap::empty());

/// Caché de bloques pequeños por clase de tamaño (T80).
///
/// `linked_list_allocator` es *first-fit* sobre una lista de huecos: cada
/// `alloc` recorre los huecos desde el principio, y rustc deja miles de huecos
/// pequeños por delante. La compilación de un crate mediano en el guest era ~100
/// veces más lenta que en el host. Los bloques de hasta `CLASE_MAX` bytes con
/// alineación ≤ 16 se piden redondeados a múltiplo de 16 y, al liberarlos, se
/// apilan por clase y se reutilizan en O(1) sin pasar por la lista. No se
/// devuelven nunca a la lista (no se unen con sus vecinos).
const CLASE_MAX: usize = 1024;
const N_CLASES: usize = CLASE_MAX / 16;

struct Clases {
    cabeza: [*mut u8; N_CLASES],
}

unsafe impl Send for Clases {}

static CLASES: Candado<Clases> = Candado::new(Clases { cabeza: [core::ptr::null_mut(); N_CLASES] });

#[inline]
fn clase_de(layout: &Layout) -> Option<usize> {
    #[cfg(soso_heap_debug)]
    {
        let _ = layout;
        return None;
    }
    #[cfg(not(soso_heap_debug))]
    if layout.size() <= CLASE_MAX && layout.align() <= 16 {
        Some(layout.size().max(1).div_ceil(16) - 1)
    } else {
        None
    }
}
static HEAP_READY: AtomicBool = AtomicBool::new(false);
/// El montón de lista nació de `sbrk` (puede crecer con `sbrk`); si nació de
/// `mmap` (`heap_init_mmap`, objetos compartidos) no crece por ahí.
static HEAP_BRK: AtomicBool = AtomicBool::new(false);

struct ArenaState {
    cur: usize,
    end: usize,
    last_start: usize,
    last_end: usize,
}

static ARENA: Candado<ArenaState> = Candado::new(ArenaState {
    cur: 0,
    end: 0,
    last_start: 0,
    last_end: 0,
});

/// Comprueba invariantes del arena (no hace nada sin `SOSO_HEAP_DEBUG=1`).
pub fn heap_audit() {
    #[cfg(soso_heap_debug)]
    heap_debug::audit();
}

#[cfg(soso_heap_debug)]
pub(crate) fn heap_debug_audit_impl() {
    ARENA.lock();
    let st = unsafe { &*ARENA.valor.get() };
    heap_debug::audit_arena(st.cur, st.end, st.last_start, st.last_end);
    ARENA.unlock();
}

#[cfg(not(soso_heap_debug))]
#[allow(dead_code)]
pub(crate) fn heap_debug_audit_impl() {}

/// Pide el primer mebibyte del montón de lista. Se puede llamar más de una vez.
pub fn heap_init() {
    if OPS.load(Ordering::Acquire).is_null() {
        instalar(&SOSO_OPS);
    }
    if HEAP_READY.swap(true, Ordering::AcqRel) {
        return;
    }
    let base = (ops().sbrk)(HEAP_INIT_BYTES as i64);
    if base > 0 {
        HEAP_BRK.store(true, Ordering::Release);
        HEAP.lock();
        unsafe {
            (*HEAP.valor.get()).init(base as *mut u8, HEAP_INIT_BYTES);
        }
        HEAP.unlock();
    }
}

/// Como [`heap_init`] pero el montón de lista sale de un `mmap` y no usa `sbrk`
/// jamás. Es lo que usa un objeto compartido cargado dentro de un proceso (T80):
/// su copia de este crate y la del ejecutable se disputarían el `brk`, y el
/// montón de lista sólo crece si lo que se añade es contiguo a su tope.
pub fn heap_init_mmap() {
    if OPS.load(Ordering::Acquire).is_null() {
        instalar(&SOSO_OPS);
    }
    if HEAP_READY.swap(true, Ordering::AcqRel) {
        return;
    }
    let base = (ops().mmap)(0, HEAP_INIT_BYTES as u64, u64::MAX, 0);
    if base > 0 {
        HEAP.lock();
        unsafe {
            (*HEAP.valor.get()).init(base as *mut u8, HEAP_INIT_BYTES);
        }
        HEAP.unlock();
    }
}

/// Bytes ocupados en la lista de bloques pequeños. Un `mmap` no los mueve:
/// sirve para ver que una liberación pequeña devolvió el bloque.
pub fn bytes_en_lista() -> usize {
    HEAP.lock();
    let n = unsafe { (*HEAP.valor.get()).used() };
    HEAP.unlock();
    n
}

/// `(ocupados, tamaño usable)` de la lista. El tamaño es 0 si `heap_init` no
/// llegó a pedir el primer mebibyte.
pub fn estado_lista() -> (usize, usize) {
    HEAP.lock();
    let heap = unsafe { &*HEAP.valor.get() };
    let par = (heap.used(), heap.size());
    HEAP.unlock();
    par
}

/// Cierto si `ptr` salió de la lista y no del arena ni de `mmap`.
pub fn en_lista(ptr: *mut u8) -> bool {
    ptr_in_linked_heap(ptr)
}

/// Tipo vacío: el estado vive en las estáticas de este crate. Registrarlo como
/// alocador global no crea un segundo montón.
pub struct Allocator;

impl Allocator {
    pub const fn new() -> Self {
        Self
    }
}

struct SbrkAllocator;

impl SbrkAllocator {
    unsafe fn bump(st: &mut ArenaState, size: usize, align: usize) -> *mut u8 {
        let mut start = (st.cur + align - 1) & !(align - 1);
        if start + size > st.end {
            let want = if size + align > ARENA_CHUNK {
                size + align
            } else {
                ARENA_CHUNK
            };
            // `mmap` y no `sbrk`: el `brk` es del montón de lista, que sólo crece
            // si lo que se añade es contiguo a su tope.
            let base = (ops().mmap)(0, want.next_multiple_of(4096) as u64, u64::MAX, 0);
            if base <= 0 {
                return core::ptr::null_mut();
            }
            st.cur = base as usize;
            st.end = base as usize + want;
            start = (st.cur + align - 1) & !(align - 1);
            if start + size > st.end {
                return core::ptr::null_mut();
            }
        }
        st.cur = start + size;
        st.last_start = start;
        st.last_end = st.cur;
        start as *mut u8
    }
}

unsafe impl core::alloc::GlobalAlloc for SbrkAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe {
            #[cfg(soso_heap_debug)]
            let inner = heap_debug::layout_extra(layout);
            #[cfg(not(soso_heap_debug))]
            let inner = layout;
            let size = inner.size();
            let align = inner.align().max(16);
            ARENA.lock();
            let ptr = Self::bump(&mut *ARENA.valor.get(), size, align);
            ARENA.unlock();
            if !ptr.is_null() {
                #[cfg(soso_heap_debug)]
                heap_debug::stamp(ptr, layout.size());
            }
            ptr
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            #[cfg(soso_heap_debug)]
            if !ptr.is_null() {
                heap_debug::verify(ptr, layout.size(), "dealloc arena");
            }
            let p = ptr as usize;
            ARENA.lock();
            let st = &mut *ARENA.valor.get();
            #[cfg(soso_heap_debug)]
            let reserved = heap_debug::stored_size(layout);
            #[cfg(not(soso_heap_debug))]
            let reserved = layout.size();
            if p == st.last_start && p + reserved == st.last_end && st.cur == st.last_end {
                st.cur = st.last_start;
                st.last_end = st.last_start;
            }
            ARENA.unlock();
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe {
            let p = ptr as usize;
            ARENA.lock();
            let st = &mut *ARENA.valor.get();
            #[cfg(soso_heap_debug)]
            let old_reserved = heap_debug::stored_size(layout);
            #[cfg(not(soso_heap_debug))]
            let old_reserved = layout.size();
            let es_ultimo =
                p == st.last_start && p + old_reserved == st.last_end && st.cur == st.last_end;
            #[cfg(soso_heap_debug)]
            let new_reserved = {
                let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
                heap_debug::stored_size(new_layout)
            };
            #[cfg(not(soso_heap_debug))]
            let new_reserved = new_size;
            if es_ultimo && p + new_reserved <= st.end {
                st.cur = p + new_reserved;
                st.last_end = st.cur;
                ARENA.unlock();
                #[cfg(soso_heap_debug)]
                heap_debug::stamp(ptr, new_size);
                return ptr;
            }
            ARENA.unlock();
            let nuevo = Layout::from_size_align_unchecked(new_size, layout.align());
            let dst = self.alloc(nuevo);
            if !dst.is_null() {
                core::ptr::copy_nonoverlapping(ptr, dst, layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
            dst
        }
    }
}

fn inner_layout(layout: Layout) -> Layout {
    #[cfg(soso_heap_debug)]
    {
        heap_debug::layout_extra(layout)
    }
    #[cfg(not(soso_heap_debug))]
    {
        layout
    }
}

/// El arena vive por encima del heap enlazado; liberar un puntero del arena
/// con `deallocate` de la lista corrompe la lista.
fn ptr_in_linked_heap(ptr: *mut u8) -> bool {
    if ptr.is_null() {
        return false;
    }
    HEAP.lock();
    let heap = unsafe { &*HEAP.valor.get() };
    let bottom = heap.bottom() as usize;
    let top = heap.top() as usize;
    HEAP.unlock();
    let p = ptr as usize;
    p >= bottom && p < top
}

impl Allocator {
    unsafe fn alloc_base(&self, layout: Layout) -> *mut u8 {
        unsafe {
            let size = layout.size();
            if size >= MMAP_ALLOC_MIN && layout.align() <= 4096 {
                let p = (ops().mmap)(0, size as u64, u64::MAX, 0);
                if p > 0 {
                    return p as *mut u8;
                }
            }
            #[cfg(soso_heap_debug)]
            let inner = heap_debug::layout_extra(layout);
            #[cfg(not(soso_heap_debug))]
            let inner = layout;
            if layout.align() <= 4096 {
                HEAP.lock();
                let heap = &mut *HEAP.valor.get();
                let mut hecho = heap.allocate_first_fit(inner);
                if hecho.is_err() && HEAP_BRK.load(Ordering::Acquire) {
                    // El montón de lista era de 1 MiB fijos y lo demás caía en
                    // el arena de bump, que no reutiliza memoria: rustc agotaba
                    // el `brk` (~1,3 GiB) compilando `quote` («memory allocation
                    // of 448 bytes failed»). Se extiende con `sbrk` mientras el
                    // nuevo trozo sea contiguo al tope del montón.
                    let paso = (inner.size() + inner.align()).next_multiple_of(4096).max(HEAP_GROW_BYTES);
                    let tope = heap.top() as usize;
                    let old = (ops().sbrk)(paso as i64);
                    if old > 0 {
                        if old as usize == tope {
                            heap.extend(paso);
                            hecho = heap.allocate_first_fit(inner);
                        } else {
                            // No contiguo (otro `sbrk` por medio): se deshace.
                            let _ = (ops().sbrk)(-(paso as i64));
                        }
                    }
                }
                HEAP.unlock();
                if let Ok(ptr) = hecho {
                    let p = ptr.as_ptr();
                    #[cfg(soso_heap_debug)]
                    heap_debug::stamp(p, layout.size());
                    return p;
                }
            }
            let align = inner.align().max(16);
            ARENA.lock();
            let ptr = SbrkAllocator::bump(&mut *ARENA.valor.get(), inner.size(), align);
            ARENA.unlock();
            if !ptr.is_null() {
                #[cfg(soso_heap_debug)]
                heap_debug::stamp(ptr, layout.size());
            }
            ptr
        }
    }

    unsafe fn dealloc_base(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            if layout.size() >= MMAP_ALLOC_MIN && layout.align() <= 4096 {
                let _ = (ops().munmap)(ptr as u64, (layout.size() as u64).next_multiple_of(4096));
                return;
            }
            #[cfg(soso_heap_debug)]
            if !ptr.is_null() {
                heap_debug::verify(ptr, layout.size(), "dealloc");
            }
            if layout.align() <= 4096 && ptr_in_linked_heap(ptr) {
                let inner = inner_layout(layout);
                HEAP.lock();
                let _ = (*HEAP.valor.get()).deallocate(core::ptr::NonNull::new_unchecked(ptr), inner);
                HEAP.unlock();
                return;
            }
            SbrkAllocator.dealloc(ptr, layout);
        }
    }

    unsafe fn realloc_base(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe {
            if layout.size() >= MMAP_ALLOC_MIN && layout.align() <= 4096 {
                let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
                let dst = self.alloc_base(new_layout);
                if !dst.is_null() && !ptr.is_null() {
                    core::ptr::copy_nonoverlapping(ptr, dst, layout.size().min(new_size));
                    self.dealloc_base(ptr, layout);
                }
                return dst;
            }
            #[cfg(soso_heap_debug)]
            if !ptr.is_null() {
                heap_debug::verify(ptr, layout.size(), "realloc");
            }
            if layout.align() <= 4096 && ptr_in_linked_heap(ptr) {
                let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
                let dst = self.alloc_base(new_layout);
                if !dst.is_null() && !ptr.is_null() {
                    core::ptr::copy_nonoverlapping(ptr, dst, layout.size().min(new_size));
                    self.dealloc_base(ptr, layout);
                }
                return dst;
            }
            SbrkAllocator.realloc(ptr, layout, new_size)
        }
    }
}

unsafe impl core::alloc::GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe {
            let Some(c) = clase_de(&layout) else {
                return self.alloc_base(layout);
            };
            CLASES.lock();
            let cab = &mut (*CLASES.valor.get()).cabeza[c];
            let p = *cab;
            if !p.is_null() {
                *cab = *(p as *mut *mut u8);
                CLASES.unlock();
                return p;
            }
            CLASES.unlock();
            self.alloc_base(Layout::from_size_align_unchecked((c + 1) * 16, 16))
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe {
            let ptr = self.alloc(layout);
            if !ptr.is_null() {
                core::ptr::write_bytes(ptr, 0, layout.size());
            }
            ptr
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            let Some(c) = clase_de(&layout) else {
                return self.dealloc_base(ptr, layout);
            };
            CLASES.lock();
            let cab = &mut (*CLASES.valor.get()).cabeza[c];
            *(ptr as *mut *mut u8) = *cab;
            *cab = ptr;
            CLASES.unlock();
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe {
            let nuevo = Layout::from_size_align_unchecked(new_size, layout.align());
            match (clase_de(&layout), clase_de(&nuevo)) {
                (Some(a), Some(b)) if a == b => ptr,
                (None, None) => self.realloc_base(ptr, layout, new_size),
                _ => {
                    let dst = self.alloc(nuevo);
                    if !dst.is_null() && !ptr.is_null() {
                        core::ptr::copy_nonoverlapping(ptr, dst, layout.size().min(new_size));
                        self.dealloc(ptr, layout);
                    }
                    dst
                }
            }
        }
    }
}

/// Las mismas operaciones que `GlobalAlloc`, para la rama `sys/alloc` de std.
pub unsafe fn alloc(layout: Layout) -> *mut u8 {
    unsafe { Allocator.alloc(layout) }
}

pub unsafe fn alloc_zeroed(layout: Layout) -> *mut u8 {
    unsafe { Allocator.alloc_zeroed(layout) }
}

pub unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
    unsafe { Allocator.dealloc(ptr, layout) }
}

pub unsafe fn realloc(ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    unsafe { Allocator.realloc(ptr, layout, new_size) }
}

#[cfg(test)]
mod pruebas {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    struct Banco {
        base: usize,
        len: usize,
        cursor: usize,
        sbrk_falla: bool,
        mmap_falla: bool,
        munmaps: usize,
        maps: Vec<(usize, usize)>,
    }

    static BANCO: std::sync::Mutex<Banco> = std::sync::Mutex::new(Banco {
        base: 0,
        len: 0,
        cursor: 0,
        sbrk_falla: false,
        mmap_falla: false,
        munmaps: 0,
        maps: Vec::new(),
    });

    static PRUEBA: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static FALSA: Ops = Ops {
        sbrk: sbrk_falso,
        mmap: mmap_falso,
        munmap: munmap_falso,
    };

    fn sbrk_falso(delta: i64) -> i64 {
        let mut b = BANCO.lock().unwrap();
        if b.sbrk_falla || delta < 0 {
            return -1;
        }
        let old = b.cursor;
        let nuevo = old + delta as usize;
        if nuevo > b.len {
            return -1;
        }
        b.cursor = nuevo;
        (b.base + old) as i64
    }

    fn mmap_falso(_addr: u64, len: u64, _fd: u64, _off: u64) -> i64 {
        let mut b = BANCO.lock().unwrap();
        if b.mmap_falla || len == 0 {
            return -1;
        }
        let alineado = (len as usize).next_multiple_of(4096);
        let layout = Layout::from_size_align(alineado, 4096).unwrap();
        let p = unsafe { std::alloc::alloc(layout) };
        if p.is_null() {
            return -1;
        }
        unsafe { core::ptr::write_bytes(p, 0xA5, alineado) };
        b.maps.push((p as usize, alineado));
        p as i64
    }

    fn munmap_falso(addr: u64, len: u64) -> i64 {
        let mut b = BANCO.lock().unwrap();
        let n = len as usize;
        let Some(i) = b.maps.iter().position(|&(a, l)| a == addr as usize && l == n) else {
            return -1;
        };
        let (a, l) = b.maps.swap_remove(i);
        unsafe {
            std::alloc::dealloc(a as *mut u8, Layout::from_size_align(l, 4096).unwrap());
        }
        b.munmaps += 1;
        0
    }

    fn reiniciar() {
        HEAP_READY.store(false, Ordering::Release);
        OPS.store(core::ptr::null_mut(), Ordering::Release);
        ARENA.lock();
        unsafe {
            *ARENA.valor.get() = ArenaState {
                cur: 0,
                end: 0,
                last_start: 0,
                last_end: 0,
            };
        }
        ARENA.unlock();
        HEAP.lock();
        unsafe {
            *HEAP.valor.get() = Heap::empty();
        }
        HEAP.unlock();
        let mut b = BANCO.lock().unwrap();
        if b.base == 0 {
            let layout = Layout::from_size_align(8 * 1024 * 1024, 4096).unwrap();
            let p = unsafe { std::alloc::alloc(layout) };
            assert!(!p.is_null());
            b.base = p as usize;
            b.len = layout.size();
        }
        b.cursor = 0;
        b.sbrk_falla = false;
        b.mmap_falla = false;
        b.munmaps = 0;
        // La caché de clases apunta a memoria de la prueba anterior.
        unsafe { (*CLASES.valor.get()).cabeza = [core::ptr::null_mut(); N_CLASES] };
        for (a, l) in b.maps.drain(..) {
            unsafe {
                std::alloc::dealloc(a as *mut u8, Layout::from_size_align(l, 4096).unwrap());
            }
        }
    }

    fn con_prueba(f: impl FnOnce()) {
        let _g = PRUEBA.lock().unwrap();
        reiniciar();
        instalar(&FALSA);
        heap_init();
        f();
    }

    #[test]
    fn reserva_fallida_devuelve_nulo() {
        con_prueba(|| {
            BANCO.lock().unwrap().sbrk_falla = true;
            BANCO.lock().unwrap().mmap_falla = true;
            // El mebibyte inicial ya se pidió. Un bloque que no cabe en él y
            // no puede ir a mmap tiene que fallar, no inventar memoria.
            let grande = Layout::from_size_align(MMAP_ALLOC_MIN * 2, 16).unwrap();
            let p = unsafe { alloc(grande) };
            assert!(p.is_null());
            let pequeno = Layout::from_size_align(32, 16).unwrap();
            let q = unsafe { alloc(pequeno) };
            assert!(!q.is_null(), "la lista ya reservada sigue sirviendo");
            unsafe { dealloc(q, pequeno) };
        });
    }

    #[test]
    fn alineacion_de_pequeno_y_de_arena() {
        con_prueba(|| {
            for align in [16usize, 64, 256, 4096] {
                let layout = Layout::from_size_align(32, align).unwrap();
                let p = unsafe { alloc(layout) };
                assert!(!p.is_null(), "align {align}");
                assert_eq!(p as usize % align, 0, "align {align}");
                unsafe { dealloc(p, layout) };
            }
            let ancho = Layout::from_size_align(64, 8192).unwrap();
            let p = unsafe { alloc(ancho) };
            assert!(!p.is_null());
            assert_eq!(p as usize % 8192, 0);
            unsafe { dealloc(p, ancho) };
        });
    }

    #[test]
    fn alloc_zeroed_y_realloc_conserva_el_prefijo() {
        con_prueba(|| {
            let layout = Layout::from_size_align(32, 16).unwrap();
            let p = unsafe { alloc_zeroed(layout) };
            assert!(!p.is_null());
            assert!(unsafe { core::slice::from_raw_parts(p, 32) }.iter().all(|&b| b == 0));
            for i in 0..32 {
                unsafe { p.add(i).write(i as u8) };
            }
            let q = unsafe { realloc(p, layout, 200) };
            assert!(!q.is_null());
            for i in 0..32 {
                assert_eq!(unsafe { q.add(i).read() }, i as u8);
            }
            let nuevo = Layout::from_size_align(200, 16).unwrap();
            unsafe { dealloc(q, nuevo) };
        });
    }

    #[test]
    fn tras_las_alineaciones_el_bloque_pequeno_sigue_en_la_lista() {
        con_prueba(|| {
            for align in [16usize, 64, 256, 4096] {
                let layout = Layout::from_size_align(32, align).unwrap();
                let p = unsafe { alloc(layout) };
                assert!(!p.is_null() && en_lista(p), "align {align} no salió de la lista");
                unsafe { dealloc(p, layout) };
            }
            let z = Layout::from_size_align(32, 16).unwrap();
            let p = unsafe { alloc_zeroed(z) };
            assert!(en_lista(p));
            let q = unsafe { realloc(p, z, 200) };
            assert!(en_lista(q));
            unsafe { dealloc(q, Layout::from_size_align(200, 16).unwrap()) };

            let (ocupados, tam) = estado_lista();
            assert!(tam > 512 * 1024, "lista={ocupados}/{tam}");
            let a_lay = Layout::from_size_align(128, 16).unwrap();
            let b_lay = Layout::from_size_align(64, 16).unwrap();
            let antes = bytes_en_lista();
            let a = unsafe { alloc(a_lay) };
            let b = unsafe { alloc(b_lay) };
            assert!(en_lista(a) && en_lista(b), "a={a:?} b={b:?} lista={antes}");
            assert!(bytes_en_lista() > antes);
            unsafe { dealloc(a, a_lay) };
            let c = unsafe { alloc(a_lay) };
            assert_eq!(c, a, "el hueco del primero se reutiliza");
            unsafe { dealloc(c, a_lay) };
            unsafe { dealloc(b, b_lay) };
            // Los bloques pequeños se quedan en la caché de clases: la lista no
            // los recupera, se reutilizan (comprobado arriba).
            assert!(bytes_en_lista() >= antes);
        });
    }

    #[test]
    fn liberar_pequeno_devuelve_el_bloque_y_grande_hace_munmap() {
        con_prueba(|| {
            let layout = Layout::from_size_align(128, 16).unwrap();
            let antes = bytes_en_lista();
            let p = unsafe { alloc(layout) };
            assert!(!p.is_null());
            assert!(bytes_en_lista() > antes);
            unsafe { dealloc(p, layout) };
            let otra = unsafe { alloc(layout) };
            assert_eq!(otra, p, "el hueco liberado se reutiliza");
            unsafe { dealloc(otra, layout) };

            let grande = Layout::from_size_align(MMAP_ALLOC_MIN, 16).unwrap();
            let lista = bytes_en_lista();
            let g = unsafe { alloc(grande) };
            assert!(!g.is_null());
            assert_eq!(bytes_en_lista(), lista, "mmap no entra en la lista");
            unsafe { g.write(0x11) };
            unsafe { g.add(MMAP_ALLOC_MIN - 1).write(0x22) };
            unsafe { dealloc(g, grande) };
            assert_eq!(BANCO.lock().unwrap().munmaps, 1);
            assert_eq!(bytes_en_lista(), lista);
        });
    }
}
