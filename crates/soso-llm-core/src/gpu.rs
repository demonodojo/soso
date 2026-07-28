//! Offload GPU híbrido (G5): matvec vía syscall cuando hay VRAM.

use crate::layer::TensorView;
use sosomodel::layout::DTYPE_F32;

/// Despacho opcional hacia la GPU del kernel (implementado en userspace).
pub trait GpuDispatch {
    fn available(&self) -> bool;
    /// matvec F32. `key` identifica el tensor (`"L03.ffn_up"`) y es **estable**:
    /// el despacho lo usa para saber si esos pesos ya están en el dispositivo y no
    /// resubirlos en cada token. Identificarlos por la dirección de `view` sería
    /// más cómodo y estaría mal: los shards se mapean y se pueden desmapear, y una
    /// dirección reutilizada por otro tensor daría pesos ajenos sin ningún error.
    ///
    /// `Ok(true)` = **el resultado ya está en `out`** (lo haya calculado la GPU o
    /// el kernel); `Ok(false)` = calcúlalo tú.
    fn matvec_f32(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, ()>;
}

/// Sin GPU: siempre CPU.
pub struct NoGpu;

impl GpuDispatch for NoGpu {
    fn available(&self) -> bool {
        false
    }

    fn matvec_f32(
        &mut self,
        _key: &str,
        _view: &TensorView<'_>,
        _rows: usize,
        _cols: usize,
        _x: &[f32],
        _out: &mut [f32],
    ) -> Result<bool, ()> {
        Ok(false)
    }
}

/// Intenta offload GPU para F32; devuelve true si la GPU lo ejecutó.
pub fn try_gpu_matvec(
    gpu: &mut dyn GpuDispatch,
    key: &str,
    view: &TensorView<'_>,
    rows: usize,
    cols: usize,
    x: &[f32],
    out: &mut [f32],
) -> Result<bool, ()> {
    if !gpu.available() || view.dtype != DTYPE_F32 {
        return Ok(false);
    }
    if view.f32().is_some() {
        return gpu.matvec_f32(key, view, rows, cols, x, out);
    }
    Ok(false)
}
