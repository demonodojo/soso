//! Futex de soso: `SYS_FUTEX` con `FUTEX_WAIT` / `FUTEX_WAKE`.
//!
//! El plazo se pasa **relativo en ms**, con `0` = sin plazo, que es la
//! convención del resto del kernel (`read_timeout`, `ping`, `tcp_connect`,
//! `tcp_accept`). Al vencer, el kernel despierta con `-ETIMEDOUT`, y esa es la
//! única forma de distinguir «expiró» de «me despertaron» (T73).

use crate::sync::atomic::Atomic;
use crate::time::Duration;

/// Un átomo para usar como futex, de al menos 32 bits.
pub type Futex = Atomic<Primitive>;
/// Tiene que ser el tipo subyacente de `Futex`.
pub type Primitive = u32;

/// Un átomo para usar como futex, de al menos 8 bits.
pub type SmallFutex = Atomic<SmallPrimitive>;
/// Tiene que ser el tipo subyacente de `SmallFutex`.
pub type SmallPrimitive = u32;

/// Convierte el plazo de `std` al de soso.
///
/// **Un plazo menor de 1 ms se sube a 1, no se redondea a 0.** En soso `0`
/// significa «sin plazo», así que redondear hacia abajo convertiría un
/// `wait_timeout(100 µs)` en una espera **eterna** — el defecto de T55 otra
/// vez, esta vez introducido por una conversión de unidades.
fn plazo_ms(timeout: Option<Duration>) -> u64 {
    match timeout {
        None => 0,
        Some(d) => {
            let ms = d.as_millis();
            if ms == 0 {
                1
            } else {
                // Un plazo enorme se satura en vez de dar la vuelta, que lo
                // dejaría en el pasado y haría que expirase al instante.
                u64::try_from(ms).unwrap_or(u64::MAX)
            }
        }
    }
}

/// Espera mientras `*futex == expected`. Devuelve `false` **sólo** si venció
/// el plazo.
pub fn futex_wait(futex: &Atomic<u32>, expected: u32, timeout: Option<Duration>) -> bool {
    // SAFETY: `futex` apunta a un `u32` vivo y alineado; el kernel valida
    // además que la dirección esté mapeada en el proceso.
    let r = unsafe {
        soso_rt::syscall4(
            soso_rt::SYS_FUTEX,
            soso_rt::FUTEX_WAIT,
            futex.as_ptr().expose_provenance() as u64,
            expected as u64,
            plazo_ms(timeout),
        )
    };
    r != -soso_rt::ETIMEDOUT
}

/// Despierta a uno. Devuelve si despertó a alguien.
#[inline]
pub fn futex_wake(futex: &Atomic<u32>) -> bool {
    // SAFETY: como arriba.
    unsafe {
        soso_rt::syscall4(
            soso_rt::SYS_FUTEX,
            soso_rt::FUTEX_WAKE,
            futex.as_ptr().expose_provenance() as u64,
            0,
            1,
        ) > 0
    }
}

#[inline]
pub fn futex_wake_all(futex: &Atomic<u32>) {
    // SAFETY: como arriba.
    unsafe {
        soso_rt::syscall4(
            soso_rt::SYS_FUTEX,
            soso_rt::FUTEX_WAKE,
            futex.as_ptr().expose_provenance() as u64,
            0,
            u64::MAX,
        );
    }
}
