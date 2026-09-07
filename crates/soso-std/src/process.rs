pub fn exit(code: u8) -> ! {
    libsoso::sys::exit(code)
}

pub fn spawn(path: &str, args: &str) -> Result<u64, i64> {
    let pid = libsoso::sys::spawn(path, args);
    if pid < 0 {
        Err(pid)
    } else {
        Ok(pid as u64)
    }
}

pub fn wait() -> Result<(u64, u8), i64> {
    libsoso::sys::wait()
}
