//! Offload GPU híbrido (G5): matvec vía syscall cuando hay VRAM.
//!
//! **Los pesos que importan están cuantizados.** Este despacho sólo admitía F32,
//! así que con un modelo Q4_K o Q8_0 —los que caben en una tarjeta de 12 GiB— el
//! offload no se usaba nunca y no lo decía: `try_gpu_matvec` se rendía por el
//! dtype y el matvec se iba a CPU en silencio. Ahora pasan también Q8_0 y Q4_K, y
//! el que implementa el rasgo decide cómo: el camino de soso es **descuantizar una
//! sola vez al subirlos** (el dispositivo calcula en f32 y los pesos son
//! residentes, así que se paga una vez por tensor, no por token). El precio es
//! VRAM —Q4_K a F32 son ~8×—, y ése es justo el criterio del offload híbrido: lo
//! que no cabe se queda en CPU.

use crate::layer::TensorView;
use crate::sched::{CostModel, N_TRAMOS};
use sosomodel::layout::{DTYPE_F32, DTYPE_MXFP4, DTYPE_Q4_K, DTYPE_Q8_0};

/// Estadísticas acumuladas del despacho GPU.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuStats {
    pub launches: u64,
    pub total_ns: u64,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub ewma_ns: [f64; N_TRAMOS],
    pub samples: [u64; N_TRAMOS],
}

/// Una operación matvec dentro de un lote.
pub struct MatvecOp<'a> {
    pub key: &'a str,
    pub view: &'a TensorView<'a>,
    pub rows: usize,
    pub cols: usize,
    pub x: &'a [f32],
    pub out: &'a mut [f32],
}

/// Despacho opcional hacia la GPU del kernel (implementado en userspace).
pub trait GpuDispatch {
    fn available(&self) -> bool;

    fn stats(&self) -> GpuStats {
        GpuStats::default()
    }

    fn cost_model(&self) -> CostModel {
        CostModel::default()
    }

    /// matvec de una fila por elemento de `out`.
    fn matvec(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, ()>;

    /// Y[n×rows] = X[n×cols] · W[rows×cols]^T (W row-major).
    fn matmul(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
        n: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, ()> {
        let _ = (key, view, rows, cols, n, x, out);
        Ok(false)
    }

    /// Varios matvec encolados; espera única al final.
    fn matvec_batch(&mut self, ops: &mut [MatvecOp<'_>]) -> Result<bool, ()> {
        let mut any = false;
        for op in ops.iter_mut() {
            if self.matvec(op.key, op.view, op.rows, op.cols, op.x, op.out)? {
                any = true;
            }
        }
        Ok(any)
    }

    fn softmax_rows(&mut self, x: &mut [f32], rows: usize, cols: usize) -> Result<bool, ()> {
        let _ = (x, rows, cols);
        Ok(false)
    }

    fn layernorm_rows(
        &mut self,
        x: &mut [f32],
        weight: &[f32],
        bias: &[f32],
        rows: usize,
        cols: usize,
        eps: f32,
    ) -> Result<bool, ()> {
        let _ = (x, weight, bias, rows, cols, eps);
        Ok(false)
    }
}

/// Sin GPU: siempre CPU.
pub struct NoGpu;

impl GpuDispatch for NoGpu {
    fn available(&self) -> bool {
        false
    }

    fn matvec(
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

/// Dtypes que el despacho sabe presentar.
pub fn dtype_ofrecible(dtype: u8) -> bool {
    matches!(dtype, DTYPE_F32 | DTYPE_Q8_0 | DTYPE_Q4_K | DTYPE_MXFP4)
}

/// Intenta el offload; `true` si el resultado ya está en `out`.
pub fn try_gpu_matvec(
    gpu: &mut dyn GpuDispatch,
    key: &str,
    view: &TensorView<'_>,
    rows: usize,
    cols: usize,
    x: &[f32],
    out: &mut [f32],
) -> Result<bool, ()> {
    if !gpu.available() || !dtype_ofrecible(view.dtype) {
        return Ok(false);
    }
    if view.dtype == DTYPE_F32 && view.f32().is_none() {
        return Ok(false);
    }
    if view.elems != rows * cols {
        return Ok(false);
    }
    gpu.matvec(key, view, rows, cols, x, out)
}

/// Intenta matmul batched; `true` si el resultado ya está en `out`.
pub fn try_gpu_matmul(
    gpu: &mut dyn GpuDispatch,
    key: &str,
    view: &TensorView<'_>,
    rows: usize,
    cols: usize,
    n: usize,
    x: &[f32],
    out: &mut [f32],
) -> Result<bool, ()> {
    if !gpu.available() || !dtype_ofrecible(view.dtype) {
        return Ok(false);
    }
    if view.dtype == DTYPE_F32 && view.f32().is_none() {
        return Ok(false);
    }
    if view.elems != rows * cols || x.len() != cols * n || out.len() != rows * n {
        return Ok(false);
    }
    gpu.matmul(key, view, rows, cols, n, x, out)
}
