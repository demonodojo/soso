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
use sosomodel::layout::{DTYPE_F32, DTYPE_MXFP4, DTYPE_Q4_K, DTYPE_Q8_0};

/// Despacho opcional hacia la GPU del kernel (implementado en userspace).
pub trait GpuDispatch {
    fn available(&self) -> bool;

    /// matvec de una fila por elemento de `out`.
    ///
    /// `key` identifica el tensor (`"L03.ffn_up"`) y es **estable**: el despacho lo
    /// usa para saber si esos pesos ya están en el dispositivo y no resubirlos en
    /// cada token. Identificarlos por la dirección de `view` sería más cómodo y
    /// estaría mal: los shards se mapean y se pueden desmapear, y una dirección
    /// reutilizada por otro tensor daría pesos ajenos sin ningún error.
    ///
    /// `view` puede venir en F32, Q8_0 o Q4_K; quien implementa esto decide si lo
    /// admite y qué hace con la cuantización. Devolver `Ok(false)` es siempre
    /// legítimo: significa "no lo he hecho, hazlo tú".
    ///
    /// `Ok(true)` = **el resultado ya está en `out`** (lo haya calculado la GPU o
    /// el kernel); `Ok(false)` = calcúlalo tú.
    fn matvec(
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

/// Dtypes que el despacho sabe presentar. Un dtype nuevo NO cae aquí por defecto:
/// mejor que se vaya a CPU a que llegue al dispositivo como bytes sin interpretar.
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
    // El F32 exige además que la vista sea reinterpretable (alineación): si no lo
    // es, a CPU. Los cuantizados se leen byte a byte y no tienen ese requisito.
    if view.dtype == DTYPE_F32 && view.f32().is_none() {
        return Ok(false);
    }
    if view.elems != rows * cols {
        return Ok(false);
    }
    gpu.matvec(key, view, rows, cols, x, out)
}
