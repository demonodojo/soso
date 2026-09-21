//! kmalloc/kfree — pool con devolución real (G2).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ffi::c_void;
use spin::Mutex;

struct Pool {
    /// Bloques libres: `(dirección, capacidad real reservada)`.
    blocks: Vec<(usize, usize)>,
    /// Vivos: `(dirección) -> (bytes del usuario, capacidad real)`.
    live: BTreeMap<usize, (usize, usize)>,
}

/// Centinela detrás de cada bloque. Los drivers son C portado y el síntoma de
/// que uno se pase escribiendo aparece **lejos** de la causa: el asignador del
/// kernel se encuentra sus listas rotas y revienta en otra cosa, minutos
/// después. Ocho bytes por bloque valen ese diagnóstico.
const CENTINELA: u64 = 0x5A50_4F53_4F5A_4F53; // "ZSOSOZOS"

static POOL: Mutex<Pool> = Mutex::new(Pool {
    blocks: Vec::new(),
    live: BTreeMap::new(),
});

const ALIGN: usize = 8;

fn poner_centinela(addr: usize, size: usize) {
    unsafe { core::ptr::write_unaligned((addr + size) as *mut u64, CENTINELA) };
}

fn alloc_block(user_size: usize, zero: bool) -> *mut c_void {
    let size = (user_size + ALIGN - 1) & !(ALIGN - 1);
    // Lo que se reserva de verdad: lo pedido más el centinela.
    let total = size + 8;
    let mut p = POOL.lock();
    if let Some(i) = p.blocks.iter().position(|&(_, cap)| cap >= total) {
        let (addr, cap) = p.blocks.swap_remove(i);
        if zero {
            unsafe { core::ptr::write_bytes(addr as *mut u8, 0, size) };
        }
        poner_centinela(addr, size);
        p.live.insert(addr, (size, cap));
        return addr as *mut c_void;
    }
    drop(p);
    use alloc::alloc::{alloc, Layout};
    let layout = Layout::from_size_align(total.max(ALIGN), ALIGN).unwrap();
    let ptr = unsafe { alloc(layout) };
    if ptr.is_null() {
        return core::ptr::null_mut();
    }
    if zero {
        unsafe { core::ptr::write_bytes(ptr, 0, size) };
    }
    poner_centinela(ptr as usize, size);
    POOL.lock().live.insert(ptr as usize, (size, layout.size()));
    ptr as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_kmalloc(size: usize, flags: u32) -> *mut c_void {
    alloc_block(size, flags & 0x8000 != 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_kzalloc(size: usize, flags: u32) -> *mut c_void {
    lx_kmalloc(size, flags | 0x8000)
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_krealloc(ptr: *mut c_void, size: usize, _flags: u32) -> *mut c_void {
    if ptr.is_null() {
        return lx_kmalloc(size, 0);
    }
    // Lo que se copia es lo que **había**, no lo que se pide: al crecer, el
    // bloque viejo es más pequeño y copiar `size` bytes lee fuera de él.
    let viejo = POOL.lock().live.get(&(ptr as usize)).map(|&(s, _)| s).unwrap_or(0);
    let n = lx_kmalloc(size, 0);
    if !n.is_null() {
        unsafe {
            core::ptr::copy_nonoverlapping(ptr as *const u8, n as *mut u8, size.min(viejo));
        }
    }
    lx_kfree(ptr);
    n
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_kfree(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    let addr = ptr as usize;
    let mut p = POOL.lock();
    if let Some((size, cap)) = p.live.remove(&addr) {
        // Si el centinela no está, alguien escribió más allá de su bloque. Se
        // dice **aquí**, con el tamaño y la dirección, en vez de dejar que el
        // asignador del kernel se estrelle después siguiendo una lista rota.
        let visto = unsafe { core::ptr::read_unaligned((addr + size) as *const u64) };
        if visto != CENTINELA {
            crate::println!(
                "lxdde: BLOQUE DESBORDADO en {addr:#x} ({size} B): centinela {visto:#x}"
            );
            panic!("lxdde escribió más allá de un bloque de {size} B en {addr:#x}");
        }
        p.blocks.push((addr, cap));
    }
}

/// Comprueba el centinela de **todos** los bloques vivos.
///
/// El de `lx_kfree` sólo mira el bloque que se libera, y un driver puede
/// desbordar un buffer que no suelta en horas: para entonces la lista del
/// asignador ya está rota y el pánico sale en `talc`, lejísimos de la causa.
/// Esto recorre los vivos y dice **qué** bloque se pasó, con dirección y
/// tamaño, poco después de que ocurra.
///
/// Devuelve el número de bloques revisados, o `None` si el candado estaba
/// ocupado (se prueba en la vuelta siguiente; esto nunca debe esperar).
pub fn barrer_centinelas() -> Option<usize> {
    let p = POOL.try_lock()?;
    let mut n = 0usize;
    for (&addr, &(size, _)) in p.live.iter() {
        let visto = unsafe { core::ptr::read_unaligned((addr + size) as *const u64) };
        if visto != CENTINELA {
            crate::println!(
                "lxdde: BLOQUE DESBORDADO (barrido) en {addr:#x} ({size} B): centinela {visto:#x}"
            );
            panic!("lxdde escribió más allá de un bloque de {size} B en {addr:#x}");
        }
        n += 1;
    }
    Some(n)
}

/// Igual que `barrer_centinelas`, pero **no panica**: se llama desde el
/// manejador de un #PF en ring 0, cuando `talc` ya ha petado siguiendo una
/// lista rota. Si el smash fue un `lx_kmalloc`, aquí sale el bloque y el
/// tamaño. Si no sale nada, el overflow no fue de este pool (Vec/smoltcp).
pub fn avisar_desbordados_en_pf() {
    let Some(p) = POOL.try_lock() else {
        crate::println!("lxdde: pool ocupado, no se pudieron mirar centinelas en el #PF");
        return;
    };
    let mut rotos = 0usize;
    for (&addr, &(size, _)) in p.live.iter() {
        let visto = unsafe { core::ptr::read_unaligned((addr + size) as *const u64) };
        if visto != CENTINELA {
            rotos += 1;
            crate::println!(
                "lxdde: BLOQUE DESBORDADO (en #PF) en {addr:#x} ({size} B): centinela {visto:#x}"
            );
        }
    }
    crate::println!(
        "lxdde: centinelas en #PF: {} bloques vivos, {rotos} desbordados",
        p.live.len()
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_vmalloc(size: u32) -> *mut c_void {
    lx_kmalloc(size as usize, 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_vfree(ptr: *mut c_void) {
    lx_kfree(ptr);
}

/// Buffer alineado a página (4 KiB), fuera del pool de `lx_kmalloc` (que solo
/// alinea a 8). Lo necesita la tabla radix3 del GSP: describe la imagen GSP-RM
/// página a página, así que tiene que empezar justo en un límite de página.
/// **No** exige contigüidad física — cada página se traduce por separado con
/// `lx_virt_to_phys`, que es precisamente para lo que existe radix3.
#[unsafe(no_mangle)]
pub extern "C" fn lx_alloc_pages_exact(size: usize) -> *mut c_void {
    use alloc::alloc::{alloc_zeroed, Layout};
    let size = (size + 4095) & !4095;
    let Ok(layout) = Layout::from_size_align(size.max(4096), 4096) else {
        return core::ptr::null_mut();
    };
    unsafe { alloc_zeroed(layout) as *mut c_void }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_free_pages_exact(ptr: *mut c_void, size: usize) {
    if ptr.is_null() {
        return;
    }
    use alloc::alloc::{dealloc, Layout};
    let size = (size + 4095) & !4095;
    let Ok(layout) = Layout::from_size_align(size.max(4096), 4096) else {
        return;
    };
    unsafe { dealloc(ptr as *mut u8, layout) };
}

/// Dirección física de una virtual del kernel. 0 si no está mapeada.
#[unsafe(no_mangle)]
pub extern "C" fn lx_virt_to_phys(ptr: *const c_void) -> u64 {
    if ptr.is_null() {
        return 0;
    }
    crate::mm::virt_to_phys(ptr as u64).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_register_initcall(_func: extern "C" fn() -> i32, _level: i32) {}
