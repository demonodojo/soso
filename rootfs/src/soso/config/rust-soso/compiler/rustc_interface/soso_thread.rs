#[cfg(target_os = "soso")]
fn soso_syscall4(n: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    let ret: i64;
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") n => ret,
            inlateout("rdi") a1 => _,
            inlateout("rsi") a2 => _,
            inlateout("rdx") a3 => _,
            inlateout("r10") a4 => _,
            lateout("rcx") _,
            lateout("r11") _,
            clobber_abi("C"),
        );
    }
    ret
}

#[cfg(target_os = "soso")]
fn soso_run_in_thread<F, R>(
    stack: usize,
    edition: Edition,
    sm_inputs: SourceMapInputs,
    extra_symbols: &[&'static str],
    f: F,
) -> R
where
    F: FnOnce(CurrentGcx) -> R + Send,
    R: Send,
{
    struct Job<F, R> {
        edition: Edition,
        extra_symbols: *const &'static str,
        extra_len: usize,
        sm_inputs: Option<SourceMapInputs>,
        f: Option<F>,
        result: Option<R>,
    }

    extern "C" fn start<F, R>(arg: u64) -> !
    where
        F: FnOnce(CurrentGcx) -> R + Send,
        R: Send,
    {
        let job = unsafe { &mut *(arg as *mut Job<F, R>) };
        let edition = job.edition;
        let symbols = unsafe { core::slice::from_raw_parts(job.extra_symbols, job.extra_len) };
        let sm = job.sm_inputs.take().unwrap();
        let f = job.f.take().unwrap();
        let result = rustc_span::create_session_globals_then(edition, symbols, Some(sm), || {
            f(CurrentGcx::new())
        });
        job.result = Some(result);
        let _ = soso_syscall4(0, 0, 0, 0, 0);
        loop {
            unsafe { core::arch::asm!("pause", options(nomem, nostack)) }
        }
    }

    let mut job = Job {
        edition,
        extra_symbols: extra_symbols.as_ptr(),
        extra_len: extra_symbols.len(),
        sm_inputs: Some(sm_inputs),
        f: Some(f),
        result: None,
    };
    let len = (stack.max(256 * 1024) as u64).next_multiple_of(4096);
    let base = soso_syscall4(15, 0, len, u64::MAX, 0);
    if base < 0 {
        panic!("soso: no pude reservar la pila del hilo de rustc ({base})");
    }
    let top = (base as u64).saturating_add(len) & !0xF;
    let join = std::sync::atomic::AtomicU32::new(0);
    let entry = start::<F, R> as *const ();
    let tid = soso_syscall4(
        25,
        entry.expose_provenance() as u64,
        (&raw mut job).expose_provenance() as u64,
        top,
        (&raw const join).expose_provenance() as u64,
    );
    if tid < 0 {
        panic!("soso: SYS_THREAD_SPAWN falló ({tid})");
    }
    while join.load(Ordering::Acquire) == 0 {
        let _ = soso_syscall4(26, 0, (&raw const join).expose_provenance() as u64, 0, 0);
    }
    job.result.take().expect("el hilo de rustc no dejó resultado")
}

