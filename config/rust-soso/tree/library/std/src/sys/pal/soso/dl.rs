//! Cargador de objetos compartidos de soso (T80: proc macros).
//!
//! No es `dlopen`: soso no tiene enlazador dinámico. `wild-soso -shared`
//! produce un `ET_DYN` ya enlazado contra todo lo que necesita (libstd incluida,
//! con su propio montón), a la base nominal `0x6100_0000`, más tres secciones:
//!
//! - `.rustc`: los metadatos que rustc lee del propio fichero;
//! - `.soso.exp`: `u32 n` y `n × { u64 desplazamiento, u16 largo, nombre }`;
//! - `.soso.rel`: `u64`s `(desplazamiento << 2) | tipo` (0 = `u64`, 1 = `u32`),
//!   una por cada dirección absoluta que hay que sumar la diferencia de base.
//!
//! Cargar es mapear un trozo anónimo en la ventana baja (que cabe en los 31
//! bits de `R_X86_64_32S`), copiar los segmentos, aplicar `.soso.rel`, anotar la
//! plantilla TLS y llamar a `__soso_dl_init` y a los `init_array`.
//!
//! TLS: el código del objeto usa desplazamientos fijos respecto al TP, con su
//! bloque terminando `PM_TLS_BIAS` bytes por debajo. Cada hilo reserva
//! `TLS_RESERVA` bytes bajo el TCB (`instalar_tls`) y copia aquí las plantillas
//! de los objetos ya cargados. Dos objetos comparten ese hueco: sólo es
//! seguro si sus TLS tienen la misma forma (los de libstd la tienen; el de un
//! crate propio con `thread_local!` choca con el de otro). Es un límite.

// Un cargador convierte enteros en punteros por definición.
#![allow(implicit_provenance_casts)]

use crate::collections::BTreeMap;
use crate::ffi::OsStr;
use crate::path::Path;
use crate::string::String;
use crate::sync::atomic::{AtomicUsize, Ordering};
use crate::sync::Mutex;
use crate::vec::Vec;

/// Debe coincidir con `PM_TLS_BIAS` de `user/coreutils/src/bin/wild-soso.rs`.
pub const PM_TLS_BIAS: usize = 0x8000;
const PM_TLS_MAX: usize = 0x8000;
/// Bytes que cada hilo reserva por debajo del TCB para los objetos cargados.
pub const TLS_RESERVA: usize = PM_TLS_BIAS + PM_TLS_MAX;

const PAGE: usize = 4096;
const PT_LOAD: u32 = 1;
const PT_TLS: u32 = 7;
const SHT_INIT_ARRAY: u32 = 14;

/// Plantillas TLS de los objetos cargados, para los hilos que nazcan después.
/// Arrays de átomos y no un `Vec`: `instalar_tls` corre antes del montón.
const MAX_TLS: usize = 64;
static TLS_N: AtomicUsize = AtomicUsize::new(0);
static TLS_SRC: [AtomicUsize; MAX_TLS] = [const { AtomicUsize::new(0) }; MAX_TLS];
static TLS_FILESZ: [AtomicUsize; MAX_TLS] = [const { AtomicUsize::new(0) }; MAX_TLS];
static TLS_MEMSZ: [AtomicUsize; MAX_TLS] = [const { AtomicUsize::new(0) }; MAX_TLS];
static TLS_ALIGN: [AtomicUsize; MAX_TLS] = [const { AtomicUsize::new(0) }; MAX_TLS];
/// Cuánto más abajo del hueco nominal va el bloque de cada objeto: cada uno se
/// enlazó creyendo que era el único con TLS (todos terminan en `TP - PM_TLS_BIAS`),
/// así que el segundo pisaba al primero. El enlazador anota las reubicaciones
/// TPOFF (`.soso.rel`, tipos 2 y 3) y el cargador les resta este desplazamiento.
static TLS_DESPL: [AtomicUsize; MAX_TLS] = [const { AtomicUsize::new(0) }; MAX_TLS];
static TLS_USADO: AtomicUsize = AtomicUsize::new(0);
/// Tamaño del bloque TLS de este ejecutable (lo fija `instalar_tls`).
pub static TLS_PROPIO: AtomicUsize = AtomicUsize::new(0);

struct Cargado {
    exportados: BTreeMap<String, usize>,
}

static CARGADOS: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());

fn tabla() -> &'static Mutex<BTreeMap<usize, &'static Cargado>> {
    static T: Mutex<BTreeMap<usize, &'static Cargado>> = Mutex::new(BTreeMap::new());
    &T
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Copia las plantillas TLS de los objetos cargados bajo el TP `tp`.
///
/// # Safety
/// `tp` es el TP de un hilo con `TLS_RESERVA` bytes mapeados por debajo.
pub unsafe fn tls_instalar(tp: usize) {
    let n = TLS_N.load(Ordering::Acquire);
    for i in 0..n.min(MAX_TLS) {
        unsafe { tls_instalar_uno(tp, i) };
    }
}

/// Copia la plantilla del objeto `i` bajo `tp` (sin tocar la de los demás: un
/// objeto que se carga no puede reiniciar el TLS vivo de los ya cargados).
///
/// # Safety
/// Como `tls_instalar`.
unsafe fn tls_instalar_uno(tp: usize, i: usize) {
    let memsz = TLS_MEMSZ[i].load(Ordering::Relaxed);
    let align = TLS_ALIGN[i].load(Ordering::Relaxed).max(1);
    let tam = (memsz + align - 1) & !(align - 1);
    let dst = (tp - PM_TLS_BIAS - TLS_DESPL[i].load(Ordering::Relaxed) - tam) as *mut u8;
    let src = TLS_SRC[i].load(Ordering::Relaxed) as *const u8;
    let filesz = TLS_FILESZ[i].load(Ordering::Relaxed);
    unsafe {
        crate::ptr::copy_nonoverlapping(src, dst, filesz);
        crate::ptr::write_bytes(dst.add(filesz), 0, memsz - filesz);
    }
}

fn tp_actual() -> usize {
    let tp: usize;
    // SAFETY: `%fs:0` es el puntero propio del TCB (`Tcb::propio`).
    unsafe { core::arch::asm!("mov {}, fs:0", out(reg) tp, options(nostack, readonly)) };
    tp
}

/// Carga `ruta` (o la devuelve si ya estaba) y devuelve su identificador.
pub fn cargar(ruta: &Path) -> Result<usize, String> {
    let clave = ruta.to_string_lossy().into_owned();
    if let Some(&id) = CARGADOS.lock().unwrap().get(&clave) {
        return Ok(id);
    }
    let datos = crate::fs::read(ruta).map_err(|e| crate::format!("{e}"))?;
    let b = &datos[..];
    if b.get(..4) != Some(b"\x7fELF") || u16_at(b, 16) != Some(3) {
        return Err(String::from("no es un objeto compartido de soso (ET_DYN)"));
    }
    let phoff = u64_at(b, 32).ok_or("ELF truncado")? as usize;
    let phentsize = u16_at(b, 54).ok_or("ELF truncado")? as usize;
    let phnum = u16_at(b, 56).ok_or("ELF truncado")? as usize;
    let mut cargas: Vec<(usize, usize, usize, usize)> = Vec::new(); // vaddr, off, filesz, memsz
    let mut tls = None;
    for i in 0..phnum {
        let o = phoff + i * phentsize;
        let ty = u32_at(b, o).ok_or("ELF truncado")?;
        let off = u64_at(b, o + 8).ok_or("ELF truncado")? as usize;
        let va = u64_at(b, o + 16).ok_or("ELF truncado")? as usize;
        let filesz = u64_at(b, o + 32).ok_or("ELF truncado")? as usize;
        let memsz = u64_at(b, o + 40).ok_or("ELF truncado")? as usize;
        let align = u64_at(b, o + 48).ok_or("ELF truncado")? as usize;
        if ty == PT_LOAD {
            cargas.push((va, off, filesz, memsz));
        } else if ty == PT_TLS {
            tls = Some((va, off, filesz, memsz, align));
        }
    }
    if cargas.is_empty() {
        return Err(String::from("sin segmentos PT_LOAD"));
    }
    let nominal = cargas.iter().map(|c| c.0).min().unwrap() & !(PAGE - 1);
    let fin = cargas.iter().map(|c| c.0 + c.3).max().unwrap();
    let largo = (fin - nominal + PAGE - 1) & !(PAGE - 1);

    // Ventana baja: `mmap` con pista busca el primer hueco desde ahí.
    // SAFETY: mmap anónimo (`u64::MAX` es el fd de anónimo).
    let r = unsafe { soso_rt::syscall4(soso_rt::SYS_MMAP, nominal as u64, largo as u64, u64::MAX, 0) };
    if r <= 0 {
        return Err(crate::format!("mmap de {largo} bytes falló ({r})"));
    }
    let base = r as usize;
    if base + largo > 0x8000_0000 {
        return Err(String::from("la imagen cae por encima de 2 GiB: las relocaciones de 32 bits no caben"));
    }
    for &(va, off, filesz, _) in &cargas {
        let src = b.get(off..off + filesz).ok_or("segmento fuera del fichero")?;
        // SAFETY: dentro de lo recién mapeado.
        unsafe { crate::ptr::copy_nonoverlapping(src.as_ptr(), (base + va - nominal) as *mut u8, filesz) };
    }
    let delta = base.wrapping_sub(nominal);

    // Secciones por nombre.
    let shoff = u64_at(b, 40).ok_or("ELF truncado")? as usize;
    let shentsize = u16_at(b, 58).ok_or("ELF truncado")? as usize;
    let shnum = u16_at(b, 60).ok_or("ELF truncado")? as usize;
    let shstrndx = u16_at(b, 62).ok_or("ELF truncado")? as usize;
    let sh = |i: usize| shoff + i * shentsize;
    let str_off = u64_at(b, sh(shstrndx) + 24).ok_or("ELF truncado")? as usize;
    let nombre_de = |o: usize| -> &[u8] {
        let s = &b[str_off + o..];
        &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
    };
    let mut rel: &[u8] = &[];
    let mut exp: &[u8] = &[];
    let mut inits: Vec<(usize, usize)> = Vec::new();
    for i in 0..shnum {
        let o = sh(i);
        let nombre = nombre_de(u32_at(b, o).ok_or("ELF truncado")? as usize);
        let ty = u32_at(b, o + 4).ok_or("ELF truncado")?;
        let addr = u64_at(b, o + 16).ok_or("ELF truncado")? as usize;
        let off = u64_at(b, o + 24).ok_or("ELF truncado")? as usize;
        let size = u64_at(b, o + 32).ok_or("ELF truncado")? as usize;
        if nombre == b".soso.rel" {
            rel = b.get(off..off + size).ok_or("`.soso.rel` fuera del fichero")?;
        } else if nombre == b".soso.exp" {
            exp = b.get(off..off + size).ok_or("`.soso.exp` fuera del fichero")?;
        } else if ty == SHT_INIT_ARRAY {
            inits.push((addr - nominal + base, size / 8));
        }
    }
    let despl = TLS_USADO.load(Ordering::Acquire);
    for e in rel.chunks_exact(8) {
        let v = u64::from_le_bytes(e.try_into().unwrap()) as usize;
        let at = base + (v >> 2);
        // SAFETY: lo emitió el enlazador dentro de la imagen.
        unsafe {
            match v & 3 {
                0 => {
                    let p = at as *mut u64;
                    p.write_unaligned(p.read_unaligned().wrapping_add(delta as u64));
                }
                1 => {
                    let p = at as *mut u32;
                    p.write_unaligned(p.read_unaligned().wrapping_add(delta as u32));
                }
                // TPOFF32 / ranura GOTTPOFF: el bloque TLS baja `despl` bytes.
                2 => {
                    let p = at as *mut u32;
                    p.write_unaligned(p.read_unaligned().wrapping_sub(despl as u32));
                }
                _ => {
                    let p = at as *mut u64;
                    p.write_unaligned(p.read_unaligned().wrapping_sub(despl as u64));
                }
            }
        }
    }

    let mut exportados = BTreeMap::new();
    if exp.len() >= 4 {
        let n = u32_at(exp, 0).unwrap() as usize;
        let mut o = 4;
        for _ in 0..n {
            let off = u64_at(exp, o).ok_or("`.soso.exp` truncada")? as usize;
            let largo = u16_at(exp, o + 8).ok_or("`.soso.exp` truncada")? as usize;
            let nombre = exp.get(o + 10..o + 10 + largo).ok_or("`.soso.exp` truncada")?;
            exportados.insert(String::from_utf8_lossy(nombre).into_owned(), base + off);
            o += 10 + largo;
        }
    }

    if let Some((va, off, filesz, memsz, align)) = tls {
        let propio = TLS_PROPIO.load(Ordering::Relaxed);
        if propio > PM_TLS_BIAS {
            return Err(crate::format!("el TLS de este ejecutable ({propio} B) no cabe en la reserva ({PM_TLS_BIAS} B)"));
        }
        let tam = (memsz + align.max(1) - 1) & !(align.max(1) - 1);
        if despl + tam > PM_TLS_MAX {
            return Err(crate::format!(
                "el TLS de los objetos cargados ({} B) no cabe en la reserva ({PM_TLS_MAX} B)",
                despl + tam
            ));
        }
        let i = TLS_N.load(Ordering::Acquire);
        if i >= MAX_TLS {
            return Err(String::from("demasiados objetos con TLS"));
        }
        let _ = off;
        TLS_DESPL[i].store(despl, Ordering::Relaxed);
        TLS_USADO.store(despl + tam, Ordering::Release);
        TLS_SRC[i].store(base + va - nominal, Ordering::Relaxed);
        TLS_FILESZ[i].store(filesz, Ordering::Relaxed);
        TLS_MEMSZ[i].store(memsz, Ordering::Relaxed);
        TLS_ALIGN[i].store(align, Ordering::Relaxed);
        TLS_N.store(i + 1, Ordering::Release);
        // SAFETY: el hilo actual reservó `TLS_RESERVA` bajo su TCB.
        unsafe { tls_instalar_uno(tp_actual(), i) };
    }

    // Montón propio del objeto (cada copia de `soso-alloc` tiene su estado).
    if let Some(&f) = exportados.get("__soso_dl_init") {
        // SAFETY: función `extern "C" fn()` exportada por el objeto.
        unsafe { crate::mem::transmute::<usize, extern "C" fn()>(f)() };
    }
    for (a, n) in inits {
        for k in 0..n {
            // SAFETY: entradas de `init_array` ya relocadas.
            unsafe {
                let f = *((a + k * 8) as *const usize);
                if f != 0 && f != usize::MAX {
                    crate::mem::transmute::<usize, extern "C" fn()>(f)();
                }
            }
        }
    }

    let id = base;
    let c: &'static Cargado = crate::boxed::Box::leak(crate::boxed::Box::new(Cargado { exportados }));
    tabla().lock().unwrap().insert(id, c);
    CARGADOS.lock().unwrap().insert(clave, id);
    Ok(id)
}

/// Dirección del símbolo exportado `nombre` del objeto `id`.
pub fn simbolo(id: usize, nombre: &str) -> Result<usize, String> {
    let t = tabla().lock().unwrap();
    let c = t.get(&id).ok_or_else(|| String::from("objeto no cargado"))?;
    c.exportados.get(nombre).copied().ok_or_else(|| crate::format!("símbolo `{nombre}` no exportado"))
}

/// Llamada por `__soso_dl_init`: inicializa el montón de esta copia de libstd.
#[unsafe(no_mangle)]
pub extern "C" fn __soso_dl_init() {
    soso_rt::heap_init_mmap();
}

#[allow(dead_code)]
fn _usa(_: &OsStr) {}
