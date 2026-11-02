//! Controles extra de heap con `SOSO_HEAP_DEBUG=1` al compilar (xtask/build).

pub const REDZONE_SIZE: usize = core::mem::size_of::<u64>();
const MAGIC: u64 = 0x534F_534F_4445_4155; // "SOSODEAU"

pub fn layout_extra(layout: core::alloc::Layout) -> core::alloc::Layout {
    core::alloc::Layout::from_size_align(layout.size() + REDZONE_SIZE, layout.align())
        .unwrap_or(layout)
}

/// Tamaño real reservado (usuario + redzone).
pub fn stored_size(layout: core::alloc::Layout) -> usize {
    layout_extra(layout).size()
}

pub unsafe fn stamp(ptr: *mut u8, user_size: usize) {
    unsafe {
        (ptr.add(user_size) as *mut u64).write(MAGIC);
    }
}

pub unsafe fn verify(ptr: *mut u8, user_size: usize, op: &str) {
    unsafe {
        let got = (ptr.add(user_size) as *mut u64).read();
        if got != MAGIC {
            panic!(
                "heap-debug: redzone corrupto en {op} ptr={ptr:p} size={user_size} got={got:#x}"
            );
        }
    }
}

pub fn audit_arena(cur: usize, end: usize, last_start: usize, last_end: usize) {
    if cur > end {
        panic!("heap-debug: arena cur ({cur}) > end ({end})");
    }
    if last_start > last_end || last_end > cur {
        panic!(
            "heap-debug: arena last [{last_start},{last_end}) cur={cur} end={end}"
        );
    }
}

pub fn audit() {
    super::heap_debug_audit_impl();
}
