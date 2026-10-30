//! Copias seguras desde/hacia memoria de usuario (sin desreferenciar VA de ring 3).

use super::addrspace::USER_MAX;
use super::{with_current, PROCS};
use soso_abi as abi;

pub fn copy_from_user(ptr: u64, out: &mut [u8]) -> Result<(), i64> {
    if out.is_empty() {
        return Ok(());
    }
    if ptr == 0 || ptr.checked_add(out.len() as u64).is_none_or(|e| e > USER_MAX) {
        return Err(-abi::EFAULT);
    }
    with_current(|p| {
        let space = p.space.as_ref().ok_or(-abi::EFAULT)?;
        if !space.range_ok(ptr, out.len() as u64, false) {
            return Err(-abi::EFAULT);
        }
        space.read(ptr, out).ok_or(-abi::EFAULT)
    })
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
        if !space.range_ok(ptr, data.len() as u64, true) {
            return Err(-abi::EFAULT);
        }
        space.write(ptr, data).ok_or(-abi::EFAULT)
    })
}

pub fn copy_to_user_of_pid(pid: u64, ptr: u64, data: &[u8]) -> Result<(), i64> {
    if data.is_empty() {
        return Ok(());
    }
    let mut procs = PROCS.lock();
    let p = procs.iter().find(|p| p.pid == pid).ok_or(-abi::ESRCH)?;
    let space = p.space.as_ref().ok_or(-abi::EFAULT)?;
    if !space.range_ok(ptr, data.len() as u64, true) {
        return Err(-abi::EFAULT);
    }
    space.write(ptr, data).ok_or(-abi::EFAULT)
}

pub fn read_u32_user(ptr: u64) -> Result<u32, i64> {
    let mut b = [0u8; 4];
    copy_from_user(ptr, &mut b)?;
    Ok(u32::from_le_bytes(b))
}
