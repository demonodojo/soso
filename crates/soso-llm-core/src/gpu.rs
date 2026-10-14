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

/// Una capa de Gemma entera para un despacho que la ejecute **en la GPU** (G9): del `hidden` de entrada al de
/// salida, con su KV residente en el dispositivo. Los pesos van por clave (el despacho ya los tiene subidos:
/// ver [`GpuDispatch::gemma_subir`]); las normas, que son pequeñas, viajan en cada llamada.
///
/// **Posición y KV.** `pos0` es la posición absoluta del primer token del bloque. El despacho guarda cuántas
/// posiciones tiene de cada capa: si `pos0` es menor que las que guarda, **rebobina** (reutilización de prefijo
/// o secuencia nueva); si es mayor, no puede seguir y devuelve `Ok(false)`.
pub struct GemmaCapa<'a> {
    pub layer: u32,
    pub pos0: usize,
    pub n: usize,
    /// `n` vectores de `h` floats; entra el estado antes de la capa y sale el de después.
    pub hidden: &'a mut [f32],
    pub h: usize,
    pub heads: usize,
    pub kv_heads: usize,
    pub head_dim: usize,
    pub ffn: usize,
    /// Ventana deslizante de la capa (0 = global).
    pub ventana: usize,
    /// Filas máximas del KV de una capa global.
    pub ctx_max: usize,
    pub eps: f32,
    pub theta: f32,
    pub escala: f32,
    pub v_de_k: bool,
    pub v_norm: bool,
    pub qk_norm: bool,
    pub geglu: bool,
    pub layer_scale: f32,
    pub clave_q: &'a str,
    pub clave_k: &'a str,
    pub clave_v: &'a str,
    pub clave_o: &'a str,
    pub clave_up: &'a str,
    pub clave_gate: &'a str,
    pub clave_down: &'a str,
    pub attn_norm: &'a [f32],
    pub q_norm: &'a [f32],
    pub k_norm: &'a [f32],
    pub post_attn_norm: &'a [f32],
    pub ffn_norm: &'a [f32],
    pub post_ffn_norm: &'a [f32],
    pub rope_freqs: Option<&'a [f32]>,
    /// Salida: filas expulsadas por delante del KV de esta capa.
    pub expulsadas: usize,
}

/// Despacho opcional hacia la GPU del kernel (implementado en userspace).
pub trait GpuDispatch {
    fn available(&self) -> bool;

    /// ¿Sabe este despacho presentar tensores de ese dtype? Por defecto, los de [`dtype_ofrecible`]; un
    /// backend con más núcleos (el CUDA del host acepta Q4_0) lo amplía sin tocar a los demás.
    fn dtype_soportado(&self, dtype: u8) -> bool {
        dtype_ofrecible(dtype)
    }

    fn stats(&self) -> GpuStats {
        GpuStats::default()
    }

    fn cost_model(&self) -> CostModel {
        CostModel::default()
    }

    /// ¿Puede este despacho intentar la capa `layer` de Gemma entera? `false` si ya la rechazó.
    fn gemma_puede(&self, layer: u32) -> bool {
        let _ = layer;
        false
    }

    /// ¿Está ya subido ese tensor para [`Self::gemma_capa`]?
    fn gemma_residente(&self, key: &str) -> bool {
        let _ = key;
        false
    }

    /// Sube un tensor para [`Self::gemma_capa`]; `Ok(false)` si no lo acepta (dtype, VRAM).
    fn gemma_subir(&mut self, key: &str, view: &TensorView<'_>, rows: usize, cols: usize) -> Result<bool, ()> {
        let _ = (key, view, rows, cols);
        Ok(false)
    }

    /// Ejecuta una capa de Gemma entera en el dispositivo; `Ok(false)` si no puede (la CPU la hace).
    fn gemma_capa(&mut self, capa: &mut GemmaCapa<'_>) -> Result<bool, ()> {
        let _ = capa;
        Ok(false)
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

    /// RMSNorm in-place (1 fila): x *= rsqrt(mean(x²)+eps) * weight.
    fn rmsnorm(&mut self, x: &mut [f32], weight: &[f32], eps: f32) -> Result<bool, ()> {
        let _ = (x, weight, eps);
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
    if !gpu.available() || !gpu.dtype_soportado(view.dtype) {
        return Ok(false);
    }
    if view.dtype == DTYPE_F32 && view.f32().is_none() {
        return Ok(false);
    }
    if view.elems != rows * cols {
        return Ok(false);
    }
    match gpu.matvec(key, view, rows, cols, x, out) {
        Ok(v) => Ok(v),
        Err(()) => Ok(false),
    }
}

/// Intenta varios matvec en un solo submit BATCH; `true` si todos en GPU.
pub fn try_gpu_matvec_batch(
    gpu: &mut dyn GpuDispatch,
    ops: &mut [MatvecOp<'_>],
) -> Result<bool, ()> {
    if !gpu.available() || ops.is_empty() {
        return Ok(false);
    }
    for op in ops.iter() {
        if !gpu.dtype_soportado(op.view.dtype) {
            return Ok(false);
        }
        if op.view.dtype == DTYPE_F32 && op.view.f32().is_none() {
            return Ok(false);
        }
    }
    match gpu.matvec_batch(ops) {
        Ok(v) => Ok(v),
        Err(()) => Ok(false),
    }
}

/// Intenta RMSNorm en GPU; `true` si `x` ya está normalizado.
pub fn try_gpu_rmsnorm(
    gpu: &mut dyn GpuDispatch,
    x: &mut [f32],
    weight: &[f32],
    eps: f32,
) -> Result<bool, ()> {
    if !gpu.available() || x.len() != weight.len() || x.is_empty() {
        return Ok(false);
    }
    match gpu.rmsnorm(x, weight, eps) {
        Ok(v) => Ok(v),
        Err(()) => Ok(false),
    }
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
    if !gpu.available() || !gpu.dtype_soportado(view.dtype) {
        return Ok(false);
    }
    if view.dtype == DTYPE_F32 && view.f32().is_none() {
        return Ok(false);
    }
    if view.elems != rows * cols || x.len() != cols * n || out.len() != rows * n {
        return Ok(false);
    }
    match gpu.matmul(key, view, rows, cols, n, x, out) {
        Ok(v) => Ok(v),
        Err(()) => Ok(false),
    }
}
