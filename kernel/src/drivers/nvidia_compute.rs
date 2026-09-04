//! G4/G5: canal de cómputo NVIDIA — gpfifo, QMD y kernels SASS precompilados.

use crate::drivers::nvidia_probe;
use spin::Mutex;

const SAXPY_SASS: &[u8] = include_bytes!("../../../lxdde/ports/nouveau/saxpy.sass.bin");
/// El kernel de G5. Se incluye aquí solo para poder decir en el arranque que
/// existe: quien lo lanza es el port en C, que lo lleva embebido por su lado. Un
/// blob a cero es el único fallo de `l6-g4f-build-sass.sh` que no da error.
const MATVEC_SASS: &[u8] = include_bytes!("../../../lxdde/ports/nouveau/matvec.sass.bin");

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
        "nvidia-compute: GSP={} blobs SASS saxpy {} B / matvec {} B",
        gsp_status(),
        SAXPY_SASS.len(),
        MATVEC_SASS.len()
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

#[cfg_attr(not(feature = "lxdde"), allow(unused_variables))]
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

#[cfg_attr(not(feature = "lxdde"), allow(unused_variables))]
pub fn submit_matvec_resident(
    w_va: u64,
    rows: usize,
    cols: usize,
    x: &[f32],
    y: &mut [f32],
) -> Result<bool, ()> {
    let mut st = COMPUTE.lock();
    let Some(s) = st.as_mut() else {
        return Err(());
    };
    #[cfg(feature = "lxdde")]
    {
        if let Ok(on_gpu) = crate::lxdde::submit_matvec_resident(w_va, rows, cols, x, y) {
            s.gpu_path = on_gpu;
            s.channel_ready = crate::lxdde::gsp_ready();
            return Ok(on_gpu);
        }
    }
    Err(())
}

/// Como `submit_matvec_resident` pero con la matriz **cuantizada** en VRAM: el
/// kernel SASS decodifica los bloques al multiplicar.
///
/// No hay camino de CPU aquí, y es deliberado: sin `lxdde` (o sin canal) esto
/// devuelve `Err` y el driver contesta `Ok(0)` para que userspace lo calcule con su
/// matvec fusionado AVX2 sobre el shard ya mapeado, que es más rápido que cualquier
/// bucle escalar del kernel.
#[cfg_attr(not(feature = "lxdde"), allow(unused_variables))]
pub fn submit_matvec_q_resident(
    w_va: u64,
    dtype: u8,
    rows: usize,
    cols: usize,
    x: &[f32],
    y: &mut [f32],
) -> Result<bool, ()> {
    let mut st = COMPUTE.lock();
    let Some(s) = st.as_mut() else {
        return Err(());
    };
    #[cfg(feature = "lxdde")]
    {
        if let Ok(on_gpu) =
            crate::lxdde::submit_matvec_q_resident(w_va, dtype, rows, cols, x, y)
        {
            s.gpu_path = on_gpu;
            s.channel_ready = crate::lxdde::gsp_ready();
            return Ok(on_gpu);
        }
    }
    Err(())
}

#[cfg_attr(not(feature = "lxdde"), allow(unused_variables))]
pub fn submit_matmul_resident(
    w_va: u64,
    rows: usize,
    cols: usize,
    n: usize,
    x: &[f32],
    y: &mut [f32],
) -> Result<bool, ()> {
    let mut st = COMPUTE.lock();
    let Some(s) = st.as_mut() else {
        return Err(());
    };
    #[cfg(feature = "lxdde")]
    {
        if let Ok(on_gpu) =
            crate::lxdde::submit_matmul_resident(w_va, rows, cols, n, x, y)
        {
            s.gpu_path = on_gpu;
            s.channel_ready = crate::lxdde::gsp_ready();
            return Ok(on_gpu);
        }
    }
    Err(())
}

#[cfg_attr(not(feature = "lxdde"), allow(unused_variables))]
pub fn submit_softmax_rows(x: &mut [f32], rows: usize, cols: usize) -> Result<bool, ()> {
    let mut st = COMPUTE.lock();
    let Some(s) = st.as_mut() else {
        return Err(());
    };
    #[cfg(feature = "lxdde")]
    {
        if let Ok(on_gpu) = crate::lxdde::submit_softmax_rows(x, rows, cols) {
            s.gpu_path = on_gpu;
            s.channel_ready = crate::lxdde::gsp_ready();
            return Ok(on_gpu);
        }
    }
    Ok(false)
}

#[cfg_attr(not(feature = "lxdde"), allow(unused_variables))]
pub fn submit_layernorm_rows(
    x: &mut [f32],
    weight: &[f32],
    bias: &[f32],
    rows: usize,
    cols: usize,
    eps: f32,
) -> Result<bool, ()> {
    let mut st = COMPUTE.lock();
    let Some(s) = st.as_mut() else {
        return Err(());
    };
    #[cfg(feature = "lxdde")]
    {
        if let Ok(on_gpu) =
            crate::lxdde::submit_layernorm_rows(x, weight, bias, rows, cols, eps)
        {
            s.gpu_path = on_gpu;
            s.channel_ready = crate::lxdde::gsp_ready();
            return Ok(on_gpu);
        }
    }
    Ok(false)
}

#[cfg_attr(not(feature = "lxdde"), allow(unused_variables))]
pub fn wait_fence(sem_slot: u32) -> Result<(), ()> {
    #[cfg(feature = "lxdde")]
    {
        if crate::lxdde::wait_fence(sem_slot).is_ok() {
            return Ok(());
        }
    }
    Err(())
}

#[allow(dead_code)]
pub fn channel_ready() -> bool {
    COMPUTE.lock().as_ref().is_some_and(|s| s.channel_ready)
}

#[allow(dead_code)]
pub fn last_gpu_path() -> bool {
    COMPUTE.lock().as_ref().is_some_and(|s| s.gpu_path)
}
