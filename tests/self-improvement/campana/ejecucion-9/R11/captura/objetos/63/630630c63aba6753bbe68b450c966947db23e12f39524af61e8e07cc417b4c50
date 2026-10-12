//! Hilos de la PAL soso — delega en la capa `sys` del crt0 (libsoso).

pub struct Thread {
    pub tid: u64,
}

pub struct JoinHandle {
    tid: u64,
}

impl JoinHandle {
    pub fn join(self) -> Result<(), i64> {
        crate::sys::thread_join(self.tid)
    }
}

pub fn spawn<F>(f: F) -> Result<JoinHandle, i64>
where
    F: FnOnce() + Send + 'static,
{
    let tid = crate::sys::thread_spawn(f)?;
    Ok(JoinHandle { tid })
}

pub fn yield_now() {
    let _ = crate::sys::sched_yield();
}
