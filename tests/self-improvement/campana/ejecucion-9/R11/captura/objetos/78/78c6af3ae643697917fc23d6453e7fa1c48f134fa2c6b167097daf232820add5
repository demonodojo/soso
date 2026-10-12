//! Completions Linux sobre fibras.

#[repr(C)]
pub struct LxCompletion {
    pub done: u32,
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_init_completion(c: *mut LxCompletion) {
    if c.is_null() {
        return;
    }
    unsafe {
        (*c).done = 0;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_complete(c: *mut LxCompletion) {
    if c.is_null() {
        return;
    }
    unsafe {
        (*c).done = 1;
    }
    super::fiber::unblock_all();
}

pub fn is_complete(c: *mut LxCompletion) -> bool {
    if c.is_null() {
        return true;
    }
    unsafe { (*c).done != 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_wait_for_completion(c: *mut LxCompletion) {
    while !is_complete(c) {
        super::fiber::block_current();
    }
}
