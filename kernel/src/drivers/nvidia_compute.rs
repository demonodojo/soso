//! G4: canal de cómputo NVIDIA — gpfifo, QMD, saxpy SASS precompilado.

use crate::drivers::nvidia_probe;
use spin::Mutex;

const SAXPY_SASS: &[u8] = include_bytes!("../../../lxdde/ports/nouveau/saxpy.sass.bin");

struct ComputeState {
    channel_ready: bool,
    gpu_path: bool,
    last_result: f32,
}

static COMPUTE: Mutex<Option<ComputeState>> = Mutex::new(None);

pub fn init() {
    if !nvidia_probe::present() {
        return;
    }
    let channel_ready = gsp_ready();
    *COMPUTE.lock() = Some(ComputeState {
        channel_ready,
        gpu_path: false,
        last_result: 0.0,
    });
    crate::println!(
        "nvidia-compute: GSP={} saxpy blob {} bytes",
        gsp_status(),
        SAXPY_SASS.len()
    );
}

fn gsp_ready() -> bool {
    #[cfg(feature = "lxdde")]
    {
        if crate::lxdde::gsp_ready() {
            return true;
        }
    }
    false
}

fn gsp_status() -> &'static str {
    #[cfg(feature = "lxdde")]
    {
        return crate::lxdde::gsp_phase();
    }
    #[cfg(not(feature = "lxdde"))]
    "off"
}

pub fn submit_saxpy(a: f32, x: &[f32], y: &mut [f32]) -> Result<bool, ()> {
    let mut st = COMPUTE.lock();
    let Some(s) = st.as_mut() else {
        return Err(());
    };
    #[cfg(feature = "lxdde")]
    {
        if let Ok(on_gpu) = crate::lxdde::submit_saxpy(a, x, y) {
            s.channel_ready = crate::lxdde::gsp_ready();
            s.gpu_path = on_gpu;
            s.last_result = y.first().copied().unwrap_or(0.0);
            return Ok(on_gpu);
        }
    }
    for i in 0..x.len().min(y.len()) {
        y[i] = a * x[i] + y[i];
    }
    s.last_result = y.first().copied().unwrap_or(0.0);
    Ok(false)
}

pub fn submit_matvec_f32(w: &[f32], rows: usize, cols: usize, x: &[f32], y: &mut [f32]) -> Result<bool, ()> {
    let mut st = COMPUTE.lock();
    let Some(s) = st.as_mut() else {
        return Err(());
    };
    #[cfg(feature = "lxdde")]
    {
        if let Ok(on_gpu) = crate::lxdde::submit_matvec_f32(w, rows, cols, x, y) {
            s.gpu_path = on_gpu;
            s.channel_ready = crate::lxdde::gsp_ready();
            return Ok(on_gpu);
        }
    }
    for r in 0..rows {
        let mut sum = 0.0f32;
        for c in 0..cols {
            sum += w[r * cols + c] * x[c];
        }
        y[r] = sum;
    }
    Ok(false)
}

pub fn channel_ready() -> bool {
    COMPUTE.lock().as_ref().is_some_and(|s| s.channel_ready)
}

pub fn last_gpu_path() -> bool {
    COMPUTE.lock().as_ref().is_some_and(|s| s.gpu_path)
}
