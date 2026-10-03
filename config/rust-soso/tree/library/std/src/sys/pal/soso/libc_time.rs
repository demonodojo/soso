//! `clock_gettime` y `gettimeofday` sobre `SYS_CLOCK_GETTIME` de soso.
//!
//! libgit2 siembra su generador con `gettimeofday`; el de glibc usa el vDSO o
//! un syscall de Linux y devuelve ENOSYS ("could get time for random seed").

#[repr(C)]
pub struct Timeval {
    tv_sec: i64,
    tv_usec: i64,
}

fn leer(id: u64) -> Option<soso_rt::Timespec> {
    let mut ts = soso_rt::Timespec { tv_sec: 0, tv_nsec: 0 };
    let rc = unsafe {
        soso_rt::syscall3(
            soso_rt::SYS_CLOCK_GETTIME,
            id,
            (&raw mut ts).expose_provenance() as u64,
            0,
        )
    };
    (rc >= 0).then_some(ts)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn clock_gettime(id: i32, out: *mut soso_rt::Timespec) -> i32 {
    // CLOCK_REALTIME es 0 en Linux; el resto se trata como monotónico.
    let soso_id = if id == 0 { soso_rt::CLOCK_REALTIME } else { soso_rt::CLOCK_MONOTONIC };
    match leer(soso_id) {
        Some(ts) => {
            unsafe { out.write(ts) };
            0
        }
        None => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gettimeofday(tv: *mut Timeval, _tz: *mut u8) -> i32 {
    if tv.is_null() {
        return 0;
    }
    match leer(soso_rt::CLOCK_REALTIME) {
        Some(ts) => {
            unsafe { tv.write(Timeval { tv_sec: ts.tv_sec, tv_usec: ts.tv_nsec / 1000 }) };
            0
        }
        None => -1,
    }
}
