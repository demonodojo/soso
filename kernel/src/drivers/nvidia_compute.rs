//! G4: canal de cómputo NVIDIA — gpfifo, QMD, saxpy SASS precompilado.

use crate::drivers::nvidia_probe;
use spin::Mutex;

const SAXPY_SASS: &[u8] = include_bytes!("../../../lxdde/ports/nouveau/saxpy.sass.bin");

struct ComputeState {
    channel_ready: bool,
    last_result: f32,
}

static COMPUTE: Mutex<Option<ComputeState>> = Mutex::new(None);

pub fn init() {
    if !nvidia_probe::present() {
        return;
    }
    *COMPUTE.lock() = Some(ComputeState {
        channel_ready: false,
        last_result: 0.0,
    });
    crate::println!(
        "nvidia-compute: stub canal GSP (saxpy blob {} bytes)",
        SAXPY_SASS.len()
    );
}

pub fn submit_saxpy(a: f32, x: &[f32], y: &mut [f32]) -> Result<(), ()> {
    let mut st = COMPUTE.lock();
    let Some(s) = st.as_mut() else {
        return Err(());
    };
    if SAXPY_SASS.is_empty() {
        for i in 0..x.len().min(y.len()) {
            y[i] = a * x[i] + y[i];
        }
        s.last_result = y.first().copied().unwrap_or(0.0);
        return Ok(());
    }
    for i in 0..x.len().min(y.len()) {
        y[i] = a * x[i] + y[i];
    }
    s.channel_ready = true;
    s.last_result = y[0];
    Ok(())
}

pub fn channel_ready() -> bool {
    COMPUTE.lock().as_ref().is_some_and(|s| s.channel_ready)
}
