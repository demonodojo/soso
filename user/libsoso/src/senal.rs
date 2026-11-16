//! Señales: capturar, ignorar, bloquear y esperar.
//!
//! Lo recomendado es [`capturar`] + [`tomar`]: el manejador interno sólo
//! enciende un bit atómico, y el programa lo mira cuando le viene bien (tras
//! cada syscall que vuelva con `EINTR`, en su bucle principal…). Un manejador
//! propio ([`Accion::Manejar`]) corre **en mitad de cualquier cosa**, incluso con
//! el candado del montón cogido: no debe reservar memoria, imprimir ni tomar
//! candados. Sólo atómicos y syscalls.
//!
//! Una syscall bloqueada (`read`, `sleep_ms`, `wait`…) que recibe una señal
//! con manejador vuelve con `-EINTR` (no hay `SA_RESTART`). Al lanzar un
//! programa (`spawn`) todo vuelve a su acción por defecto y nada queda
//! bloqueado.

use core::arch::naked_asm;
use core::sync::atomic::{AtomicU64, Ordering};
use soso_abi as abi;

pub use abi::{
    SIGALRM, SIGCHLD, SIGCONT, SIGHUP, SIGINT, SIGKILL, SIGPIPE, SIGQUIT, SIGSTOP, SIGTERM,
    SIGTSTP, SIGUSR1, SIGUSR2, SIGWINCH, sig_bit,
};

/// Qué hacer al recibir una señal.
#[derive(Clone, Copy, Debug)]
pub enum Accion {
    PorDefecto,
    Ignorar,
    /// `extern "C" fn(sig)`. Ver la advertencia del módulo.
    Manejar(extern "C" fn(u64)),
}

/// Vuelta de todos los manejadores: el kernel deja su dirección como dirección
/// de retorno del marco. `rsp` queda 8 bytes por encima del marco, que es
/// donde `SYS_SIGRETURN` lo busca.
#[unsafe(naked)]
extern "C" fn restaurador() -> ! {
    naked_asm!(
        "mov eax, {nr}",
        "syscall",
        "ud2",
        nr = const abi::SYS_SIGRETURN,
    )
}

/// Cambia la acción de `sig`. Devuelve la acción anterior en crudo
/// ([`abi::SIG_DFL`], [`abi::SIG_IGN`] o una dirección) o `-errno`.
pub fn accion(sig: u64, accion: Accion) -> Result<u64, i64> {
    let manejador = match accion {
        Accion::PorDefecto => abi::SIG_DFL,
        Accion::Ignorar => abi::SIG_IGN,
        Accion::Manejar(f) => f as *const () as u64,
    };
    let r = crate::sys::raw4(
        abi::SYS_SIGACTION,
        sig,
        manejador,
        0,
        restaurador as *const () as u64,
    );
    if r < 0 { Err(r) } else { Ok(r as u64) }
}

pub fn ignorar(sig: u64) -> Result<(), i64> {
    accion(sig, Accion::Ignorar).map(|_| ())
}

pub fn por_defecto(sig: u64) -> Result<(), i64> {
    accion(sig, Accion::PorDefecto).map(|_| ())
}

/// Bits que ha encendido el manejador de [`capturar`]. Uno por proceso: las
/// acciones son del proceso, no del hilo.
static LLEGADAS: AtomicU64 = AtomicU64::new(0);

extern "C" fn anotar(sig: u64) {
    LLEGADAS.fetch_or(abi::sig_bit(sig), Ordering::SeqCst);
}

/// Instala el manejador seguro: cada llegada de `sig` enciende su bit, que se
/// consulta con [`tomar`] o [`llegadas`].
pub fn capturar(sig: u64) -> Result<(), i64> {
    accion(sig, Accion::Manejar(anotar)).map(|_| ())
}

/// ¿Llegó `sig` desde la última vez? La apaga al mirarla.
pub fn tomar(sig: u64) -> bool {
    LLEGADAS.fetch_and(!abi::sig_bit(sig), Ordering::SeqCst) & abi::sig_bit(sig) != 0
}

/// Señales capturadas que han llegado y nadie ha tomado (sin apagarlas).
pub fn llegadas() -> u64 {
    LLEGADAS.load(Ordering::SeqCst)
}

fn mascara(how: u64, set: u64) -> u64 {
    crate::sys::raw4(abi::SYS_SIGPROCMASK, how, set, 0, 0) as u64
}

/// Añade `set` (bits [`sig_bit`]) a la máscara. Devuelve la anterior.
pub fn bloquear(set: u64) -> u64 {
    mascara(abi::SIG_BLOCK, set)
}

/// Quita `set` de la máscara. Lo pendiente se entrega antes de volver.
pub fn desbloquear(set: u64) -> u64 {
    mascara(abi::SIG_UNBLOCK, set)
}

/// Fija la máscara entera. Devuelve la anterior.
pub fn fijar_mascara(set: u64) -> u64 {
    mascara(abi::SIG_SETMASK, set)
}

/// Señales pendientes de este hilo (normalmente, las bloqueadas).
pub fn pendientes() -> u64 {
    crate::sys::raw4(abi::SYS_SIGPENDING, 0, 0, 0, 0) as u64
}

/// SIGALRM dentro de `ms` (0 la cancela). Devuelve los ms que le quedaban a la
/// anterior.
pub fn alarma(ms: u64) -> u64 {
    crate::sys::raw4(abi::SYS_ALARM, ms, 0, 0, 0) as u64
}

/// Duerme hasta que llegue una señal con manejador (vuelve tras ejecutarlo).
pub fn pausa() {
    while crate::sys::sleep_ms(u64::MAX / 2) != -abi::EINTR {}
}
