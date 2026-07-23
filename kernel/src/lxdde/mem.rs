//! kmalloc/kfree — pool con devolución real (G2).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ffi::c_void;
use spin::Mutex;

struct Pool {
    blocks: Vec<(usize, usize)>,
    live: BTreeMap<usize, usize>,
}

static POOL: Mutex<Pool> = Mutex::new(Pool {
    blocks: Vec::new(),
    live: BTreeMap::new(),
});

const ALIGN: usize = 8;

fn alloc_block(user_size: usize, zero: bool) -> *mut c_void {
    let size = (user_size + ALIGN - 1) & !(ALIGN - 1);
    let mut p = POOL.lock();
    if let Some(i) = p.blocks.iter().position(|&(_, s)| s >= size) {
        let (addr, cap) = p.blocks.swap_remove(i);
        if zero {
            unsafe { core::ptr::write_bytes(addr as *mut u8, 0, size) };
        }
        p.live.insert(addr, size.min(cap));
        return addr as *mut c_void;
    }
    drop(p);
    use alloc::alloc::{alloc, Layout};
    let layout = Layout::from_size_align(size.max(ALIGN), ALIGN).unwrap();
    let ptr = unsafe { alloc(layout) };
    if ptr.is_null() {
        return core::ptr::null_mut();
    }
    if zero {
        unsafe { core::ptr::write_bytes(ptr, 0, size) };
    }
    POOL.lock().live.insert(ptr as usize, size);
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
    let n = lx_kmalloc(size, 0);
    if !n.is_null() {
        unsafe {
            core::ptr::copy_nonoverlapping(ptr as *const u8, n as *mut u8, size);
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
    if let Some(size) = p.live.remove(&addr) {
        p.blocks.push((addr, size));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_vmalloc(size: u32) -> *mut c_void {
    lx_kmalloc(size as usize, 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_vfree(ptr: *mut c_void) {
    lx_kfree(ptr);
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_register_initcall(_func: extern "C" fn() -> i32, _level: i32) {}
