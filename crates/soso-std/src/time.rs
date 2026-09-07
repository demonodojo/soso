use libsoso::{abi, sys};

pub fn now_secs() -> i64 {
    let mut ts = abi::Timespec::default();
    if sys::clock_gettime(abi::CLOCK_REALTIME, &mut ts) == 0 {
        ts.tv_sec
    } else {
        0
    }
}
