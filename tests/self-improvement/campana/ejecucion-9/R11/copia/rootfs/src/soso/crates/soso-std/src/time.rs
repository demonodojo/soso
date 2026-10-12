use libsoso::{abi, sys};

pub fn now(out: &mut abi::Timespec) -> i64 {
    sys::clock_gettime(abi::CLOCK_REALTIME, out)
}

pub fn now_secs() -> i64 {
    let mut ts = abi::Timespec::default();
    if now(&mut ts) == 0 {
        ts.tv_sec
    } else {
        0
    }
}
