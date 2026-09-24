//! Hilos de usuario mínimos sobre `thread_spawn` + futex.

use crate::sys;
use core::sync::atomic::{AtomicU32, Ordering};

/// Tamaño por defecto de la pila de un hilo (mmap anónimo).
///
/// **La pila de un hilo no tiene guarda**, así que este número es lo único que
/// separa una recursión profunda de corromper la memoria de al lado. No hay
/// forma de detectar el desbordamiento: soso no puede marcar una página como
/// inaccesible —`mprotect` exige `PROT_READ` y rechaza `prot == 0`— y por tanto
/// no hay página que falle al pisarla. Ver
/// [T66](../../../docs/self-improvement/T66-guarda-de-pila-fingida.md) y su
/// ampliación de ABI, [N-002](../../../docs/self-improvement/native/N-002.md).
///
/// Quien elija un tamaño menor está eligiendo también a qué profundidad se
/// corrompe el montón en silencio.
pub const DEFAULT_STACK: usize = 256 * 1024;

/// Página que se **intentaba** dejar sin permisos al pie de la pila.
///
/// Se conserva el nombre y el tamaño porque el hueco sigue reservado —la pila
/// útil empieza por encima— aunque hoy no haya nada que lo proteja.
const GUARD: u64 = 4096;

/// Si la guarda de pila se pudo instalar de verdad en el último `spawn`.
///
/// Existe para que nadie tenga que deducirlo: hoy es siempre `false`, y cuando
/// [N-002](../../../docs/self-improvement/native/N-002.md) traiga `PROT_NONE`
/// pasará a ser `true` sin que cambie nada más.
static GUARDA_INSTALADA: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// `true` si los hilos que se creen ahora tienen guarda de pila.
///
/// Hoy devuelve `false` en soso. Sirve para que un programa que dependa de
/// detectar desbordamientos sepa que **no puede**, en vez de suponerlo.
pub fn hay_guarda_de_pila() -> bool {
    GUARDA_INSTALADA.load(Ordering::Acquire)
}

/// Arranca `f(arg)` en un hilo nuevo. Devuelve el handle o errno negativo.
pub fn spawn(entry: extern "C" fn(u64) -> !, arg: u64) -> Result<JoinHandle, i64> {
    let stack_len = DEFAULT_STACK as u64;
    let base = sys::mmap(0, stack_len, u64::MAX, 0);
    if base < 0 {
        return Err(base);
    }
    let join = alloc::boxed::Box::new(AtomicU32::new(0));
    let join_ptr = alloc::boxed::Box::into_raw(join);
    let top = (base as u64 + stack_len) & !0xFu64;
    let tid = sys::thread_spawn(
        entry as *const () as u64,
        arg,
        top,
        join_ptr as u64,
    );
    if tid < 0 {
        unsafe {
            let _ = alloc::boxed::Box::from_raw(join_ptr);
        }
        let _ = sys::munmap(base as u64, stack_len);
        return Err(tid);
    }
    // La guarda de pila: se pide, y **se mira si se pudo**.
    //
    // Antes esto era `let _ = sys::mprotect(...)`, y ese `let _` escondía que
    // la llamada **siempre falla**: `sys_mprotect` rechaza `prot == 0` y exige
    // `PROT_READ`, así que no hay manera de dejar una página sin acceso. El
    // resultado era un `spawn` que decía instalar una guarda y no instalaba
    // ninguna — y un desbordamiento de pila que, en vez de fallar, escribe en
    // lo que haya debajo y revienta lejos y tarde (T66).
    //
    // No se falla el `spawn` por esto: dejaría a soso sin hilos, y la pila sin
    // guarda es lo que hay hoy. Lo que no se hace es callarlo.
    let rc = sys::mprotect(base as u64, GUARD, 0);
    GUARDA_INSTALADA.store(rc >= 0, Ordering::Release);
    Ok(JoinHandle {
        tid: tid as u64,
        join: join_ptr,
    })
}

/// Barrera de generación: el líder incrementa `gen` y hace wake; los
/// workers esperan a que `gen` cambie.
pub struct Barrier {
    pub generation: AtomicU32,
    pub arrived: AtomicU32,
    pub n: u32,
}

impl Barrier {
    pub const fn new(n: u32) -> Self {
        Self {
            generation: AtomicU32::new(0),
            arrived: AtomicU32::new(0),
            n,
        }
    }

    pub fn wait(&self) {
        let g = self.generation.load(Ordering::Acquire);
        let a = self.arrived.fetch_add(1, Ordering::AcqRel) + 1;
        if a >= self.n {
            self.arrived.store(0, Ordering::Release);
            self.generation.fetch_add(1, Ordering::Release);
            let _ = sys::futex_wake(
                &self.generation as *const AtomicU32 as *const u32,
                u64::MAX,
            );
        } else {
            while self.generation.load(Ordering::Acquire) == g {
                let _ = sys::futex_wait(
                    &self.generation as *const AtomicU32 as *const u32,
                    g,
                );
            }
        }
    }
}

/// Handle para join de hilos (futex en el kernel al salir del hilo).
pub struct JoinHandle {
    pub tid: u64,
    join: *const AtomicU32,
}

impl JoinHandle {
    pub fn join(self) -> Result<(), i64> {
        unsafe {
            while (*self.join).load(Ordering::Acquire) == 0 {
                let _ = sys::futex_wait(self.join as *const u32, 0);
            }
            let _ = alloc::boxed::Box::from_raw(self.join as *mut AtomicU32);
        }
        Ok(())
    }
}
