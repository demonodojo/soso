//! Copias seguras desde/hacia memoria de usuario (sin desreferenciar VA de ring 3).

use super::addrspace::{AddrSpace, USER_MAX};
use super::with_current;
use soso_abi as abi;

/// Copia desde un espacio **ya conocido**, sin pedir `PROCS`.
///
/// Todo lo que corre en el scheduler con `PROCS` cogido (despertar lectores y
/// escritores bloqueados) tiene que copiar por aquí. `copy_from_user` lo
/// volvería a pedir vía `with_current` y, al ser un `spin::Mutex` no
/// reentrante, ese mismo núcleo se quedaría girando para siempre dentro del
/// candado que ya tiene: la máquina entera acaba en `PROCS.lock()` y ni el
/// teclado ni SSH vuelven (0.3.27, primera tecla tras el prompt).
pub fn copy_from_space(space: &AddrSpace, ptr: u64, out: &mut [u8]) -> Result<(), i64> {
    if out.is_empty() {
        return Ok(());
    }
    if ptr == 0 || ptr.checked_add(out.len() as u64).is_none_or(|e| e > USER_MAX) {
        return Err(-abi::EFAULT);
    }
    if !space.range_ok(ptr, out.len() as u64, false) {
        return Err(-abi::EFAULT);
    }
    space.read(ptr, out).ok_or(-abi::EFAULT)
}

pub fn copy_from_user(ptr: u64, out: &mut [u8]) -> Result<(), i64> {
    if out.is_empty() {
        return Ok(());
    }
    if ptr == 0 || ptr.checked_add(out.len() as u64).is_none_or(|e| e > USER_MAX) {
        return Err(-abi::EFAULT);
    }
    with_current(|p| {
        let space = p.space.as_ref().ok_or(-abi::EFAULT)?;
        copy_from_space(space, ptr, out)
    })
}

/// Copia a un espacio **ya conocido**, sin pedir `PROCS` (ver
/// [`copy_from_space`]).
pub fn copy_to_space(space: &AddrSpace, ptr: u64, data: &[u8]) -> Result<(), i64> {
    if data.is_empty() {
        return Ok(());
    }
    if ptr == 0 || ptr.checked_add(data.len() as u64).is_none_or(|e| e > USER_MAX) {
        return Err(-abi::EFAULT);
    }
    if !space.range_ok(ptr, data.len() as u64, true) {
        return Err(-abi::EFAULT);
    }
    space.write(ptr, data).ok_or(-abi::EFAULT)
}

pub fn copy_to_user(ptr: u64, data: &[u8]) -> Result<(), i64> {
    if data.is_empty() {
        return Ok(());
    }
    if ptr == 0 || ptr.checked_add(data.len() as u64).is_none_or(|e| e > USER_MAX) {
        return Err(-abi::EFAULT);
    }
    with_current(|p| {
        let space = p.space.as_ref().ok_or(-abi::EFAULT)?;
        copy_to_space(space, ptr, data)
    })
}

