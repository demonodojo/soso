//! Ejecutor capa a capa: atención (RoPE + GQA), FFN SwiGLU y residuales.
//!
//! Los pesos se leen como vistas zero-copy (`TensorView`) sobre los shards
//! mapeados: no hay copia de matrices por token, solo lecturas en streaming
//! durante el matvec. El KV cache es f16 o int8 (KIVI-lite) según el planner.

use crate::gemm::{
    add_assign_f32, add_f32, matvec_f32, matvec_q4_k, matvec_q4_k_batch_strided, matvec_q8_0,
    matvec_q8_0_batch_strided, rmsnorm, rope_cabezas, rope_neox_yarn, silu_inplace,
    swiglu_inplace, topk_softmax,
};
pub use crate::kv::{KvDtype, LayerKv};
use crate::parallel::{RowParallel, Sequential};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use sosomodel::index::TensorIndex;
use sosomodel::layout::{DTYPE_F32, DTYPE_MXFP4, DTYPE_Q4_0, DTYPE_Q4_K, DTYPE_Q8_0};
use sosomodel::manifest::{AttnKind, FfnKind, Manifest, FLAG_QK_NORM, FLAG_ROPE_NEOX};

/// Vista zero-copy del payload de un tensor (bytes crudos del shard mapeado,
/// alineados a 64 B en shards v2).
pub struct TensorView<'a> {
    pub bytes: &'a [u8],
    pub dtype: u8,
    /// Elementos f32 lógicos del tensor.
    pub elems: usize,
}

impl<'a> TensorView<'a> {
    /// Reinterpreta los bytes como `&[f32]` (solo dtype F32, target LE).
    pub fn f32(&self) -> Option<&'a [f32]> {
        if self.dtype != DTYPE_F32
            || self.bytes.len() != self.elems * 4
            || self.bytes.as_ptr() as usize % core::mem::align_of::<f32>() != 0
        {
            return None;
        }
        Some(unsafe {
            core::slice::from_raw_parts(self.bytes.as_ptr() as *const f32, self.elems)
        })
    }
}

pub trait TensorSource {
    fn load_f32(&mut self, name: &str, out: &mut [f32]) -> Result<(), ()>;
    /// Carga `out.len()` elementos a partir del elemento `elem_off` del tensor.
    fn load_f32_range(&mut self, name: &str, elem_off: usize, out: &mut [f32]) -> Result<(), ()>;
    /// Vista zero-copy del tensor completo.
    fn tensor_view(&mut self, name: &str) -> Result<TensorView<'_>, ()>;
    /// Prefetch layer-ahead (ScoutAttention / LayerKV): mapear shards y tocar
    /// la primera página para solapar I/O con el cómputo de la capa actual.
    fn prefetch_shards(&mut self, _shards: &[alloc::string::String]) {}
    /// Arranca prefetch sin esperar (double-buffer AirLLM).
    fn kick_prefetch_shards(&mut self, shards: &[alloc::string::String]) {
        self.prefetch_shards(shards);
    }
    /// Une prefetch en curso antes de usar los shards.
    fn wait_prefetch(&mut self) {}
    /// Prefetch MoE especulativo (slot aparte del layer-ahead).
    fn kick_moe_prefetch(&mut self, _shards: &[alloc::string::String]) {}
    fn wait_moe_prefetch(&mut self) {}
    /// Liberar shards fuera del working set (streaming FlexGen/LayerKV).
    fn release_shards_except(&mut self, _keep: &[alloc::string::String]) {}
    /// Prefetch de la fila de `embed` del token (page-fault adelantado).
    fn prefetch_embed_row(&mut self, _token: u32, _hidden: usize) {}
}

/// Última capa/tensor del camino de inferencia (el control es de un solo hilo).
struct LastInferOp {
    layer: AtomicU32,
    key_len: AtomicU32,
    key: UnsafeCell<[u8; 48]>,
}

unsafe impl Sync for LastInferOp {}

static LAST_INFER_OP: LastInferOp = LastInferOp {
    layer: AtomicU32::new(u32::MAX),
    key_len: AtomicU32::new(0),
    key: UnsafeCell::new([0; 48]),
};

/// Recuerda capa y nombre de tensor para el log si `generate` devuelve `Err`.
pub fn note_infer_op(layer: Option<u32>, key: &str) {
    LAST_INFER_OP
        .layer
        .store(layer.unwrap_or(u32::MAX), Ordering::Relaxed);
    let n = key.len().min(47);
    unsafe {
        let buf = &mut *LAST_INFER_OP.key.get();
        buf[..n].copy_from_slice(&key.as_bytes()[..n]);
    }
    LAST_INFER_OP.key_len.store(n as u32, Ordering::Release);
}

pub fn last_infer_op() -> (Option<u32>, String) {
    let layer = LAST_INFER_OP.layer.load(Ordering::Relaxed);
    let n = LAST_INFER_OP.key_len.load(Ordering::Acquire) as usize;
    let key = unsafe {
        let buf = &*LAST_INFER_OP.key.get();
        String::from_utf8_lossy(&buf[..n.min(48)]).into_owned()
    };
    (
        if layer == u32::MAX { None } else { Some(layer) },
        key,
    )
}

pub fn clear_infer_op() {
    LAST_INFER_OP.layer.store(u32::MAX, Ordering::Relaxed);
    LAST_INFER_OP.key_len.store(0, Ordering::Relaxed);
}

/// matvec despachado por dtype directamente sobre la vista (sin copiar pesos).
pub fn matvec_view(v: &TensorView, rows: usize, cols: usize, x: &[f32], out: &mut [f32]) -> Result<(), ()> {
    matvec_view_par(v, rows, cols, x, out, &Sequential)
}

/// Como `matvec_view` pero reparte filas con `par` (barrera al final del trait).
pub fn matvec_view_par(
    v: &TensorView,
    rows: usize,
    cols: usize,
    x: &[f32],
    out: &mut [f32],
    par: &dyn RowParallel,
) -> Result<(), ()> {
    if v.elems != rows * cols || x.len() != cols || out.len() != rows {
        return Err(());
    }
    let dtype = v.dtype;
    let bytes = v.bytes;
    // raw parts para compartir entre workers sin lifetimes cruzadas
    let x_ptr = x.as_ptr() as usize;
    let out_ptr = out.as_mut_ptr() as usize;
    let bytes_ptr = bytes.as_ptr() as usize;
    let bytes_len = bytes.len();
    let err = core::sync::atomic::AtomicBool::new(false);
    let err_ptr = &err as *const _ as usize;
    par.for_rows(rows, &move |r0, r1| {
        if r0 >= r1 {
            return;
        }
        let x = unsafe { core::slice::from_raw_parts(x_ptr as *const f32, cols) };
        let out = unsafe { core::slice::from_raw_parts_mut(out_ptr as *mut f32, rows) };
        let bytes = unsafe { core::slice::from_raw_parts(bytes_ptr as *const u8, bytes_len) };
        let ok = match dtype {
            DTYPE_F32 => {
                let w = unsafe {
                    core::slice::from_raw_parts(bytes.as_ptr() as *const f32, rows * cols)
                };
                let sub_rows = r1 - r0;
                matvec_f32(
                    &w[r0 * cols..r1 * cols],
                    sub_rows,
                    cols,
                    x,
                    &mut out[r0..r1],
                );
                true
            }
            DTYPE_Q8_0 => {
                use sosomodel::layout::{Q8_0_BLOCK_BYTES, Q8_0_BLOCK_ELEMS};
                let row_bytes = (cols / Q8_0_BLOCK_ELEMS) * Q8_0_BLOCK_BYTES;
                matvec_q8_0(
                    &bytes[r0 * row_bytes..r1 * row_bytes],
                    r1 - r0,
                    cols,
                    x,
                    &mut out[r0..r1],
                )
                .is_ok()
            }
            DTYPE_Q4_0 => {
                use sosomodel::layout::{Q4_0_BLOCK_BYTES, Q4_0_BLOCK_ELEMS};
                let row_bytes = (cols / Q4_0_BLOCK_ELEMS) * Q4_0_BLOCK_BYTES;
                crate::gemm::matvec_q4_0(
                    &bytes[r0 * row_bytes..r1 * row_bytes],
                    r1 - r0,
                    cols,
                    x,
                    &mut out[r0..r1],
                )
                .is_ok()
            }
            DTYPE_Q4_K => {
                use sosomodel::layout::{Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS};
                let row_bytes = (cols / Q4_K_BLOCK_ELEMS) * Q4_K_BLOCK_BYTES;
                matvec_q4_k(
                    &bytes[r0 * row_bytes..r1 * row_bytes],
                    r1 - r0,
                    cols,
                    x,
                    &mut out[r0..r1],
                )
                .is_ok()
            }
            DTYPE_MXFP4 => {
                use sosomodel::layout::{MXFP4_BLOCK_BYTES, MXFP4_BLOCK_ELEMS};
                let row_bytes = (cols / MXFP4_BLOCK_ELEMS) * MXFP4_BLOCK_BYTES;
                crate::gemm::matvec_mxfp4(
                    &bytes[r0 * row_bytes..r1 * row_bytes],
                    r1 - r0,
                    cols,
                    x,
                    &mut out[r0..r1],
                )
                .is_ok()
            }
            d => match sosomodel::dequant::row_bytes(d, cols) {
                Some(row_bytes) => crate::gemm::matvec_iq(
                    d,
                    &bytes[r0 * row_bytes..r1 * row_bytes],
                    r1 - r0,
                    cols,
                    x,
                    &mut out[r0..r1],
                )
                .is_ok(),
                None => false,
            },
        };
        if !ok {
            unsafe {
                (*(err_ptr as *const core::sync::atomic::AtomicBool))
                    .store(true, core::sync::atomic::Ordering::Relaxed);
            }
        }
    });
    if err.load(core::sync::atomic::Ordering::Relaxed) {
        Err(())
    } else {
        Ok(())
    }
}

/// Como [`matvec_view_par`] para `n` vectores a la vez (T82): `xs` son `n` vectores de
/// `cols` seguidos y `outs` `n` vectores de `rows` seguidos. Para cada vector el
/// resultado es idéntico bit a bit al de `matvec_view_par` (los kernels por lotes
/// repiten la misma secuencia de operaciones con acumuladores independientes).
pub fn matvec_view_batch_par(
    v: &TensorView,
    rows: usize,
    cols: usize,
    xs: &[f32],
    n: usize,
    outs: &mut [f32],
    par: &dyn RowParallel,
    q8k_work: Option<&mut [u8]>,
) -> Result<(), ()> {
    if v.elems != rows * cols || xs.len() != n * cols || outs.len() != n * rows {
        return Err(());
    }
    let dtype = v.dtype;
    let es_iq_q8k = sosomodel::layout::ggml_de_dtype_iq(dtype).is_some_and(crate::iq_dot::soporta) && crate::gemm::iq_via_q8k() && cols % 256 == 0;
    let q8k_ready = if (dtype == DTYPE_Q4_K && crate::gemm::q4_matvec_via_q8k()) || es_iq_q8k {
        if let Some(buf) = q8k_work {
            let need = crate::q8k::q8k_buffer_len(n, cols);
            if buf.len() < need {
                return Err(());
            }
            let row_bytes = need / n;
            for j in 0..n {
                crate::q8k::quantize_row_q8_k(
                    &xs[j * cols..(j + 1) * cols],
                    &mut buf[j * row_bytes..(j + 1) * row_bytes],
                );
            }
            Some((buf.as_ptr() as usize, need))
        } else {
            None
        }
    } else {
        None
    };
    let xs_ptr = xs.as_ptr() as usize;
    let out_ptr = outs.as_mut_ptr() as usize;
    let bytes_ptr = v.bytes.as_ptr() as usize;
    let bytes_len = v.bytes.len();
    let err = core::sync::atomic::AtomicBool::new(false);
    let err_ptr = &err as *const _ as usize;
    par.for_rows(rows, &move |r0, r1| {
        if r0 >= r1 {
            return;
        }
        let xs = unsafe { core::slice::from_raw_parts(xs_ptr as *const f32, n * cols) };
        let outs = unsafe { core::slice::from_raw_parts_mut(out_ptr as *mut f32, n * rows) };
        let bytes = unsafe { core::slice::from_raw_parts(bytes_ptr as *const u8, bytes_len) };
        let sub = r1 - r0;
        let ok = match dtype {
            DTYPE_F32 => {
                let w = unsafe {
                    core::slice::from_raw_parts(bytes.as_ptr() as *const f32, rows * cols)
                };
                for j in 0..n {
                    matvec_f32(
                        &w[r0 * cols..r1 * cols],
                        sub,
                        cols,
                        &xs[j * cols..(j + 1) * cols],
                        &mut outs[j * rows + r0..j * rows + r1],
                    );
                }
                true
            }
            DTYPE_Q8_0 => {
                use sosomodel::layout::{Q8_0_BLOCK_BYTES, Q8_0_BLOCK_ELEMS};
                let row_bytes = (cols / Q8_0_BLOCK_ELEMS) * Q8_0_BLOCK_BYTES;
                matvec_q8_0_batch_strided(
                    &bytes[r0 * row_bytes..r1 * row_bytes],
                    sub,
                    cols,
                    xs,
                    n,
                    &mut outs[r0..],
                    rows,
                )
                .is_ok()
            }
            DTYPE_Q4_0 => {
                use sosomodel::layout::{Q4_0_BLOCK_BYTES, Q4_0_BLOCK_ELEMS};
                let row_bytes = (cols / Q4_0_BLOCK_ELEMS) * Q4_0_BLOCK_BYTES;
                crate::gemm::matvec_q4_0_batch_strided(
                    &bytes[r0 * row_bytes..r1 * row_bytes],
                    sub,
                    cols,
                    xs,
                    n,
                    &mut outs[r0..],
                    rows,
                )
                .is_ok()
            }
            DTYPE_Q4_K => {
                use sosomodel::layout::{Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS};
                let row_bytes = (cols / Q4_K_BLOCK_ELEMS) * Q4_K_BLOCK_BYTES;
                if let Some((q8k_ptr, q8k_len)) = q8k_ready {
                    let q8k = unsafe { core::slice::from_raw_parts(q8k_ptr as *const u8, q8k_len) };
                    crate::q8k::matvec_q4_k_q8k_prequantized(
                        &bytes[r0 * row_bytes..r1 * row_bytes],
                        sub,
                        cols,
                        q8k,
                        n,
                        &mut outs[r0..],
                        rows,
                    )
                    .is_ok()
                } else {
                    matvec_q4_k_batch_strided(
                        &bytes[r0 * row_bytes..r1 * row_bytes],
                        sub,
                        cols,
                        xs,
                        n,
                        &mut outs[r0..],
                        rows,
                    )
                    .is_ok()
                }
            }
            DTYPE_MXFP4 => {
                use sosomodel::layout::{MXFP4_BLOCK_BYTES, MXFP4_BLOCK_ELEMS};
                let row_bytes = (cols / MXFP4_BLOCK_ELEMS) * MXFP4_BLOCK_BYTES;
                let mut ok = true;
                for j in 0..n {
                    ok &= crate::gemm::matvec_mxfp4(
                        &bytes[r0 * row_bytes..r1 * row_bytes],
                        sub,
                        cols,
                        &xs[j * cols..(j + 1) * cols],
                        &mut outs[j * rows + r0..j * rows + r1],
                    )
                    .is_ok();
                }
                ok
            }
            d if q8k_ready.is_some() && es_iq_q8k => {
                let (q8k_ptr, q8k_len) = q8k_ready.unwrap_or((0, 0));
                let q8k = unsafe { core::slice::from_raw_parts(q8k_ptr as *const u8, q8k_len) };
                match (sosomodel::dequant::row_bytes(d, cols), sosomodel::layout::ggml_de_dtype_iq(d)) {
                    (Some(row_bytes), Some(ggml)) => crate::iq_dot::matvec_q8k(ggml, &bytes[r0 * row_bytes..r1 * row_bytes], sub, cols, q8k, n, &mut outs[r0..], rows).is_ok(),
                    _ => false,
                }
            }
            d => match sosomodel::dequant::row_bytes(d, cols) {
                Some(row_bytes) => {
                    let mut ok = true;
                    for j in 0..n {
                        ok &= crate::gemm::matvec_iq(
                            d,
                            &bytes[r0 * row_bytes..r1 * row_bytes],
                            sub,
                            cols,
                            &xs[j * cols..(j + 1) * cols],
                            &mut outs[j * rows + r0..j * rows + r1],
                        )
                        .is_ok();
                    }
                    ok
                }
                None => false,
            },
        };
        if !ok {
            unsafe {
                (*(err_ptr as *const core::sync::atomic::AtomicBool))
                    .store(true, core::sync::atomic::Ordering::Relaxed);
            }
        }
    });
    if err.load(core::sync::atomic::Ordering::Relaxed) {
        Err(())
    } else {
        Ok(())
    }
}

/// Buffers reutilizados entre tokens: el heap de userspace no libera bloques
/// pequeños, así que las reservas deben hacerse una sola vez.
pub struct LayerScratch {
    pub residual: Vec<f32>,
    pub norm_w: Vec<f32>,
    pub q: Vec<f32>,
    pub k: Vec<f32>,
    pub v: Vec<f32>,
    pub up: Vec<f32>,
    pub gate: Vec<f32>,
    pub attn_out: Vec<f32>,
    pub head_out: Vec<f32>,
    /// Logits del router MoE (num_experts).
    pub router: Vec<f32>,
    /// Acumulador de salida FFN MoE ponderada.
    pub moe_acc: Vec<f32>,
    /// Masa softmax por token (H2O); reutilizada, crece con la ventana KV.
    pub mass_buf: Vec<f32>,
}

impl LayerScratch {
    pub fn new(m: &Manifest) -> Self {
        let h = m.hidden_dim as usize;
        let ffn = m.max_ffn_dim() as usize;
        let latent_max = (0..m.num_layers)
            .filter_map(|l| {
                let spec = m.layer(l)?;
                if spec.ffn_kind == FfnKind::LatentMoe {
                    Some(m.effective_moe_ffn_dim(l) as usize)
                } else {
                    None
                }
            })
            .max()
            .unwrap_or(0);
        let mut max_q = h;
        let mut max_k = h;
        let mut max_v = h;
        let mut max_attn = h;
        let mut max_head = 1usize;
        let mut max_nv = 1usize;
        for layer in 0..m.num_layers {
            let heads = m.effective_num_heads(layer) as usize;
            let kv_heads = m.effective_num_kv_heads(layer) as usize;
            let hd = m.effective_head_dim(layer) as usize;
            max_head = max_head.max(hd.max(1));
            match m.attn_kind(layer) {
                AttnKind::Gated => {
                    max_q = max_q.max(heads.saturating_mul(hd).saturating_mul(2));
                    max_k = max_k.max(kv_heads.saturating_mul(hd));
                    max_v = max_v.max(kv_heads.saturating_mul(hd));
                    max_attn = max_attn.max(heads.saturating_mul(hd));
                }
                AttnKind::Gdn => {
                    let qkv = heads
                        .saturating_mul(hd)
                        .saturating_mul(2)
                        .saturating_add(kv_heads.saturating_mul(hd));
                    max_q = max_q.max(qkv);
                    max_k = max_k.max(kv_heads.saturating_mul(hd));
                    max_v = max_v.max(kv_heads.saturating_mul(hd));
                    max_attn = max_attn.max(kv_heads.saturating_mul(hd));
                    max_nv = max_nv.max(kv_heads);
                }
                _ => {
                    let hd = m.effective_head_dim(layer) as usize;
                    max_q = max_q.max(heads.saturating_mul(hd));
                    max_k = max_k.max(kv_heads.saturating_mul(hd));
                    max_v = max_v.max(kv_heads.saturating_mul(hd));
                    max_attn = max_attn.max(heads.saturating_mul(hd));
                    max_head = max_head.max(hd.max(1));
                }
            }
        }
        let act_dim = ffn.max(latent_max).max(h).max(max_q).max(max_attn);
        let kv_cap = (m.max_seq as usize).min(256).max(32);
        let n_exp = m.max_router_experts() as usize;
        Self {
            residual: vec![0.0; h],
            norm_w: vec![0.0; h.max(max_head)],
            q: vec![0.0; max_q.max(h)],
            k: vec![0.0; max_k.max(1)],
            v: vec![0.0; max_v.max(1)],
            up: vec![0.0; act_dim],
            gate: vec![0.0; act_dim.max(max_q)],
            attn_out: vec![0.0; max_attn.max(h)],
            head_out: vec![0.0; max_head],
            router: vec![0.0; n_exp.max(max_nv).max(1)],
            moe_acc: vec![0.0; h.max(latent_max).max(max_nv)],
            mass_buf: vec![0.0; kv_cap],
        }
    }
}

/// Un matvec de un lote repartido en una sola región paralela (ver `moe_expertos_agrupados`).
#[derive(Clone, Copy)]
pub(crate) struct TrabajoMatvec {
    bytes: *const u8,
    len: usize,
    dtype: u8,
    rows: usize,
    cols: usize,
    x: *const f32,
    out: *mut f32,
}
unsafe impl Send for TrabajoMatvec {}
unsafe impl Sync for TrabajoMatvec {}

/// Ejecuta todos los matvec del lote repartiendo el total de filas entre los workers. Cada tramo de filas de una
/// matriz se calcula con el kernel de siempre sobre una vista de esas filas, así que el resultado es idéntico bit a bit.
pub(crate) fn matvec_multi_par(jobs: &[TrabajoMatvec], par: &dyn RowParallel) -> Result<(), ()> {
    let total: usize = jobs.iter().map(|j| j.rows).sum();
    let err = core::sync::atomic::AtomicBool::new(false);
    par.for_rows(total, &|r0, r1| {
        let mut base = 0usize;
        for j in jobs {
            let (lo, hi) = (r0.max(base), r1.min(base + j.rows));
            if lo < hi {
                let (a, b) = (lo - base, hi - base);
                let rb = if j.dtype == DTYPE_F32 { Some(j.cols * 4) } else { sosomodel::dequant::row_bytes(j.dtype, j.cols) };
                let ok = match rb {
                    Some(rb) if j.len == j.rows * rb => {
                        let bytes = unsafe { core::slice::from_raw_parts(j.bytes.add(a * rb), (b - a) * rb) };
                        let x = unsafe { core::slice::from_raw_parts(j.x, j.cols) };
                        let out = unsafe { core::slice::from_raw_parts_mut(j.out.add(a), b - a) };
                        let v = TensorView { bytes, dtype: j.dtype, elems: (b - a) * j.cols };
                        matvec_view_par(&v, b - a, j.cols, x, out, &Sequential).is_ok()
                    }
                    _ => false,
                };
                if !ok {
                    err.store(true, Ordering::Relaxed);
                }
            }
            base += j.rows;
            if base >= r1 {
                break;
            }
        }
    });
    if err.load(Ordering::Relaxed) { Err(()) } else { Ok(()) }
}

/// Diagnóstico: `false` desactiva el lote de las capas de atención (cada matvec por su cuenta).
pub static LOTE_ACTIVO: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// Lote de matvec independientes (mismas o distintas entradas): se ejecutan juntos (ver [`ejecutar_lote`]). Guarda punteros
/// crudos a los pesos (del mmap de la fuente), a `x` y a `out`: quien lo use no debe tocar esos buffers hasta `ejecutar`.
pub(crate) struct Lote {
    jobs: Vec<TrabajoMatvec>,
    claves: Vec<String>,
}

impl Lote {
    pub(crate) fn nuevo() -> Self {
        Lote { jobs: Vec::new(), claves: Vec::new() }
    }

    pub(crate) fn anadir(&mut self, layer: u32, key: &str, v: TensorView<'_>, rows: usize, cols: usize, x: &[f32], out: &mut [f32]) -> Result<(), ()> {
        if v.elems != rows * cols || x.len() != cols || out.len() != rows {
            return Err(());
        }
        note_infer_op(Some(layer), key);
        self.jobs.push(TrabajoMatvec { bytes: v.bytes.as_ptr(), len: v.bytes.len(), dtype: v.dtype, rows, cols, x: x.as_ptr(), out: out.as_mut_ptr() });
        self.claves.push(String::from(key));
        Ok(())
    }

    pub(crate) fn ejecutar(self, par: &dyn RowParallel, gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>, use_gpu: bool) -> Result<(), ()> {
        if !LOTE_ACTIVO.load(Ordering::Relaxed) {
            // Diagnóstico: uno a uno, como antes del lote.
            for (j, k) in self.jobs.iter().zip(self.claves.iter()) {
                let v = TensorView { bytes: unsafe { core::slice::from_raw_parts(j.bytes, j.len) }, dtype: j.dtype, elems: j.rows * j.cols };
                let (x, o) = unsafe { (core::slice::from_raw_parts(j.x, j.cols), core::slice::from_raw_parts_mut(j.out, j.rows)) };
                matvec_step(use_gpu, gpu, k, v, j.rows, j.cols, x, o, par, None, 0)?;
            }
            return Ok(());
        }
        ejecutar_lote(&self.jobs, &self.claves, par, gpu, use_gpu)
    }
}

/// Ejecuta un lote de matvec: lo que la GPU resuelva en una sola espera, y el resto repartido por la CPU.
fn ejecutar_lote(
    jobs: &[TrabajoMatvec],
    claves: &[String],
    par: &dyn RowParallel,
    gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    use_gpu: bool,
) -> Result<(), ()> {
    let mut hecho = vec![false; jobs.len()];
    if use_gpu {
        if let Some(g) = gpu.as_deref_mut().filter(|g| g.available()) {
            let vistas: Vec<TensorView<'_>> = jobs
                .iter()
                .map(|j| TensorView { bytes: unsafe { core::slice::from_raw_parts(j.bytes, j.len) }, dtype: j.dtype, elems: j.rows * j.cols })
                .collect();
            let mut ops: Vec<crate::gpu::MatvecOp<'_>> = jobs
                .iter()
                .zip(vistas.iter())
                .zip(claves.iter())
                .map(|((j, v), k)| crate::gpu::MatvecOp {
                    key: k.as_str(),
                    view: v,
                    rows: j.rows,
                    cols: j.cols,
                    x: unsafe { core::slice::from_raw_parts(j.x, j.cols) },
                    out: unsafe { core::slice::from_raw_parts_mut(j.out, j.rows) },
                })
                .collect();
            let _ = g.matvec_lote(&mut ops, &mut hecho);
        }
    }
    if hecho.iter().all(|&d| d) {
        return Ok(());
    }
    let resto: Vec<TrabajoMatvec> = jobs.iter().zip(&hecho).filter(|(_, h)| !**h).map(|(j, _)| *j).collect();
    matvec_multi_par(&resto, par)
}

/// matvec con offload GPU opcional (G5).
pub(crate) fn matvec_step(
    use_gpu: bool,
    gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    key: &str,
    v: TensorView<'_>,
    rows: usize,
    cols: usize,
    x: &[f32],
    out: &mut [f32],
    par: &dyn RowParallel,
    planner: Option<&crate::plan::ResourcePlanner>,
    layer: u32,
) -> Result<(), ()> {
    note_infer_op(Some(layer), key);
    let gpu_ok = use_gpu
        && planner
            .map(|p| p.gpu_tensor_allowed(layer, key))
            .unwrap_or(true);
    if gpu_ok {
        if let Some(g) = gpu.as_deref_mut() {
            // Un Err de GPU no debe abortar la inferencia: CPU calcula esa capa.
            match crate::gpu::try_gpu_matvec(g, key, &v, rows, cols, x, out) {
                Ok(true) => return Ok(()),
                Ok(false) | Err(()) => {}
            }
        }
    }
    matvec_view_par(&v, rows, cols, x, out, par)
}

/// Suma `name.bias` a `out` si el tensor existe (Qwen2 Q/K/V).
fn add_optional_bias<S: TensorSource>(
    source: &mut S,
    name: &str,
    out: &mut [f32],
    scratch: &mut [f32],
) {
    let n = out.len();
    if scratch.len() < n {
        return;
    }
    if source.load_f32(name, &mut scratch[..n]).is_ok() {
        add_assign_f32(out, &scratch[..n]);
    }
}

/// Buffers del prefill por bloques (T82): `cap` tokens a la vez.
pub struct BlockScratch {
    pub cap: usize,
    pub residual: Vec<f32>,
    pub q: Vec<f32>,
    pub k: Vec<f32>,
    pub v: Vec<f32>,
    pub attn: Vec<f32>,
    pub o: Vec<f32>,
    pub up: Vec<f32>,
    pub gate: Vec<f32>,
    /// Activaciones cuantizadas a Q8_K para matvec Q4_K (reutilizado entre capas del bloque).
    pub q8k_x: Vec<u8>,
}

impl BlockScratch {
    pub fn new(m: &Manifest, cap: usize) -> Self {
        let h = m.hidden_dim as usize;
        let mut q = h;
        let mut kv = h;
        for l in 0..m.num_layers {
            let hd = m.effective_head_dim(l) as usize;
            q = q.max(m.effective_num_heads(l) as usize * hd);
            kv = kv.max(m.effective_num_kv_heads(l) as usize * hd);
        }
        let ffn = m.max_ffn_dim() as usize;
        BlockScratch {
            cap,
            residual: vec![0.0; cap * h],
            q: vec![0.0; cap * q],
            k: vec![0.0; cap * kv],
            v: vec![0.0; cap * kv],
            attn: vec![0.0; cap * q],
            o: vec![0.0; cap * h],
            up: vec![0.0; cap * ffn],
            gate: vec![0.0; cap * ffn],
            q8k_x: Vec::new(),
        }
    }

    /// Reserva el buffer Q8_K para `n` vectores de `cols` elementos (múltiplo de 256).
    pub fn ensure_q8k(&mut self, n: usize, cols: usize) {
        let need = crate::q8k::q8k_buffer_len(n, cols);
        if self.q8k_x.len() < need {
            self.q8k_x.resize(need, 0);
        }
    }
}

/// Perfil por fases del prefill por bloques (diagnóstico). Apagado por defecto: cuesta una lectura
/// atómica por fase. Quien lo quiera registra un reloj monótono en nanosegundos con
/// [`perfil_activar`] y lee los acumulados con [`perfil_fases_ns`].
pub const FASE_QKV: usize = 0;
pub const FASE_ATENCION: usize = 1;
pub const FASE_O: usize = 2;
pub const FASE_FFN_UP_GATE: usize = 3;
pub const FASE_SWIGLU: usize = 4;
pub const FASE_FFN_DOWN: usize = 5;
pub const FASE_OTROS: usize = 6;
pub const FASE_ROPE_KV: usize = 7;
pub const N_FASES: usize = 8;
pub const NOMBRES_FASES: [&str; N_FASES] =
    ["qkv", "atención", "o-proj", "ffn up+gate", "swiglu", "ffn down", "otros", "rope+kv"];

static PERFIL_RELOJ_NS: AtomicUsize = AtomicUsize::new(0);
static PERFIL_FASES_NS: [AtomicU64; N_FASES] = [const { AtomicU64::new(0) }; N_FASES];

/// Activa el perfil con un reloj en ns (`None` lo apaga) y pone los acumulados a cero.
pub fn perfil_activar(reloj: Option<fn() -> u64>) {
    for f in &PERFIL_FASES_NS {
        f.store(0, Ordering::Relaxed);
    }
    PERFIL_RELOJ_NS.store(reloj.map_or(0, |f| f as usize), Ordering::Release);
}

pub fn perfil_fases_ns() -> [u64; N_FASES] {
    let mut o = [0u64; N_FASES];
    for (i, f) in PERFIL_FASES_NS.iter().enumerate() {
        o[i] = f.load(Ordering::Relaxed);
    }
    o
}

#[inline]
pub(crate) fn perfil_ahora() -> u64 {
    let r = PERFIL_RELOJ_NS.load(Ordering::Relaxed);
    if r == 0 {
        0
    } else {
        let f: fn() -> u64 = unsafe { core::mem::transmute::<usize, fn() -> u64>(r) };
        f()
    }
}

/// Suma a `fase` lo transcurrido desde `*t` y reinicia `*t`.
#[inline]
pub(crate) fn perfil_marca(t: &mut u64, fase: usize) {
    if PERFIL_RELOJ_NS.load(Ordering::Relaxed) == 0 {
        return;
    }
    let ahora = perfil_ahora();
    PERFIL_FASES_NS[fase].fetch_add(ahora.saturating_sub(*t), Ordering::Relaxed);
    *t = ahora;
}

/// Atención f16 por grupos GQA para `n` vectores de consulta (`q_dim` cada uno), repartida entre
/// hilos por pares (vector, cabeza KV). El vector `i` ve `seq_primero + i` posiciones del KV
/// (atención causal en el prefill por bloques; en la decodificación, `n = 1`).
///
/// Cada tarea escribe una porción distinta de `outs` y sólo lee el KV, así que el reparto no
/// cambia el resultado: el de cada consulta sale del mismo kernel con el mismo orden de sumas.
pub(crate) fn atencion_f16_grupos(
    par: &dyn RowParallel,
    qs: &[f32],
    outs: &mut [f32],
    n: usize,
    q_dim: usize,
    kv: &LayerKv,
    kv_heads: usize,
    group: usize,
    head_dim: usize,
    kv_dim: usize,
    seq_primero: usize,
) {
    let k = kv.k_f16_slice();
    let v = kv.v_f16_slice();
    let q_ptr = qs.as_ptr() as usize;
    let o_ptr = outs.as_mut_ptr() as usize;
    // Decodificación con contexto largo: el contexto se parte en trozos de `TROZO_CONTEXTO` posiciones (el reparto depende sólo de la
    // longitud, no de los hilos) y cada (cabeza KV, trozo) es una tarea; sin esto un solo token usaba tantos hilos como cabezas KV.
    if n == 1 && seq_primero >= 2 * TROZO_CONTEXTO && !SIN_PARTIR.load(Ordering::Relaxed) && crate::attn_grupo::puede_partir(head_dim, group) {
        let nt = seq_primero.div_ceil(TROZO_CONTEXTO);
        let tareas = kv_heads * nt;
        let mut partes = alloc::vec![0.0f32; tareas * group * head_dim];
        let mut ml = alloc::vec![0.0f32; tareas * 2 * group];
        let (p_ptr, m_ptr) = (partes.as_mut_ptr() as usize, ml.as_mut_ptr() as usize);
        par.for_rows(tareas, &move |t0, t1| {
            for t in t0..t1 {
                let (kvh, c) = (t / nt, t % nt);
                let (lo, hi) = (c * TROZO_CONTEXTO, ((c + 1) * TROZO_CONTEXTO).min(seq_primero));
                unsafe {
                    crate::attn_grupo::attention_f16_trozo_raw(
                        (q_ptr as *const f32).add(kvh * group * head_dim),
                        (p_ptr as *mut f32).add(t * group * head_dim),
                        (m_ptr as *mut f32).add(t * 2 * group),
                        group,
                        lo,
                        hi,
                        k,
                        v,
                        head_dim,
                        kv_dim,
                        kvh,
                    );
                }
            }
        });
        for kvh in 0..kv_heads {
            let o = &mut outs[kvh * group * head_dim..(kvh + 1) * group * head_dim];
            crate::attn_grupo::combinar_trozos(
                &partes[kvh * nt * group * head_dim..(kvh + 1) * nt * group * head_dim],
                &ml[kvh * nt * 2 * group..(kvh + 1) * nt * 2 * group],
                nt,
                group,
                head_dim,
                o,
            );
        }
        return;
    }
    // Cada tarea: una cabeza KV y hasta `TOKENS_POR_TAREA` tokens seguidos, que comparten la
    // conversión de K/V (hasta 4 × 7 cabezas por fila convertida).
    let trozos = n.div_ceil(TOKENS_POR_TAREA);
    let tareas = kv_heads * trozos;
    let trabajo = move |t0: usize, t1: usize| {
        for t in t0..t1 {
            let (kvh, c) = (t / trozos, t % trozos);
            let i0 = c * TOKENS_POR_TAREA;
            let nt = (n - i0).min(TOKENS_POR_TAREA);
            let base = i0 * q_dim + kvh * group * head_dim;
            unsafe {
                crate::attn_grupo::attention_f16_tokens_raw(
                    (q_ptr as *const f32).add(base),
                    (o_ptr as *mut f32).add(base),
                    q_dim,
                    nt,
                    group,
                    seq_primero + i0,
                    k,
                    v,
                    head_dim,
                    kv_dim,
                    kvh,
                );
            }
        }
    };
    // Con poco trabajo despertar a los hilos cuesta más que lo que ahorra.
    if tareas * seq_primero < UMBRAL_ATENCION_PAR {
        trabajo(0, tareas);
    } else {
        par.for_rows(tareas, &trabajo);
    }
}

/// Tokens consecutivos del bloque que una tarea de atención procesa juntos.
const TOKENS_POR_TAREA: usize = 4;
/// Diagnóstico: `true` desactiva la atención partida por trozos (decodificación con el kernel de siempre).
pub static SIN_PARTIR: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
/// Posiciones de contexto por tarea en la decodificación partida (ver `atencion_f16_grupos`).
const TROZO_CONTEXTO: usize = 1024;

/// `up[i] = silu(gate[i]) * up[i]` repartido entre hilos. Es elemento a elemento, así que el
/// reparto no cambia el resultado.
fn swiglu_par(par: &dyn RowParallel, up: &mut [f32], gate: &[f32]) {
    let n = up.len().min(gate.len());
    if n < UMBRAL_ELEMENTOS_PAR {
        swiglu_inplace(&mut up[..n], &gate[..n]);
        return;
    }
    let u_ptr = up.as_mut_ptr() as usize;
    let g_ptr = gate.as_ptr() as usize;
    par.for_rows(n, &move |a, b| {
        if a >= b {
            return;
        }
        let u = unsafe { core::slice::from_raw_parts_mut((u_ptr as *mut f32).add(a), b - a) };
        let g = unsafe { core::slice::from_raw_parts((g_ptr as *const f32).add(a), b - a) };
        swiglu_inplace(u, g);
    });
}

/// Elementos por debajo de los cuales una operación elemento a elemento no compensa el reparto.
const UMBRAL_ELEMENTOS_PAR: usize = 32_768;

/// Producto (tareas × posiciones) por debajo del cual la atención se hace en el hilo actual.
const UMBRAL_ATENCION_PAR: usize = 4096;

pub struct LayerExecutor<'a> {
    pub manifest: &'a Manifest,
    /// El modelo trae proyección ffn_gate (SwiGLU completo).
    pub has_gate: bool,
    /// Paralelismo de matvec (None = secuencial).
    pub parallel: Option<&'a dyn RowParallel>,
}

/// Timing del hot path de una capa (ms del reloj del runtime).
#[derive(Clone, Copy, Debug, Default)]
pub struct LayerTiming {
    pub matvec_ms: u64,
    pub attn_ms: u64,
}

impl<'a> LayerExecutor<'a> {
    pub fn forward_layer<S: TensorSource>(
        &self,
        layer: u32,
        pos: usize,
        hidden: &mut [f32],
        s: &mut LayerScratch,
        kv: &mut LayerKv,
        source: &mut S,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        use_gpu: bool,
        mut planner: Option<&mut crate::plan::ResourcePlanner>,
        index: Option<&TensorIndex>,
        clock_ms: Option<fn() -> u64>,
    ) -> Result<LayerTiming, ()> {
        let tick = |c: Option<fn() -> u64>| c.map(|f| f()).unwrap_or(0);
        let h = self.manifest.hidden_dim as usize;
        let heads = self.manifest.effective_num_heads(layer) as usize;
        let kv_heads = self.manifest.effective_num_kv_heads(layer) as usize;
        let head_dim = self.manifest.effective_head_dim(layer) as usize;
        let kv_dim = kv_heads * head_dim;
        let q_dim = heads * head_dim;
        let group = heads / kv_heads;
        let ffn = self.manifest.layer_expert_ffn_dim(layer) as usize;
        let eps = self.manifest.rms_eps;
        let theta = self.manifest.layer_rope_theta(layer);
        let ventana = self.manifest.layer_window(layer) as usize;
        let prefix = format!("L{layer:02}");
        // Los nombres se construyen UNA vez y se usan dos: para leer el tensor y
        // como clave estable del despacho a GPU (ver `GpuDispatch::matvec_f32`).
        let name_attn_q = format!("{prefix}.attn_q");
        let name_attn_k = format!("{prefix}.attn_k");
        let name_attn_v = format!("{prefix}.attn_v");
        let name_attn_output = format!("{prefix}.attn_output");
        let name_ffn_up = format!("{prefix}.ffn_up");
        let name_ffn_gate = format!("{prefix}.ffn_gate");
        let name_ffn_down = format!("{prefix}.ffn_down");
        let seq = Sequential;
        let par: &dyn RowParallel = self.parallel.unwrap_or(&seq);

        // Prefetch MoE especulativo (expertos del token anterior) antes de attn.
        if self.manifest.layer_is_moe(layer) {
            if let (Some(pl), Some(idx)) = (planner.as_mut(), index) {
                let hint = pl.moe_speculative_shards(layer, idx);
                if !hint.is_empty() {
                    source.kick_moe_prefetch(&hint);
                }
            }
        }

        let planner_ro = planner.as_deref();
        let spec = self
            .manifest
            .layer(layer)
            .cloned()
            .unwrap_or_default();
        // Gemma 4: normas en sándwich, V = K en las globales, ventana, escala 1.0… capa propia.
        if crate::gemma::es_capa_gemma(spec.flags) {
            return crate::gemma::forward_layer_gemma(
                self, layer, pos, hidden, s, kv, source, gpu, use_gpu, planner_ro,
            );
        }
        let mut t_perfil = perfil_ahora();
        let mut timing = LayerTiming::default();
        // MoE entera en la GPU (router, expertos y atención con KV residente), si el despacho puede.
        if spec.attn_kind == AttnKind::Gqa
            && self.manifest.layer_is_moe(layer)
            && crate::moe_gpu::capa_moe_en_gpu(self, layer, pos, 1, hidden, kv, source, gpu, use_gpu)?
        {
            return Ok(timing);
        }

        // --- atención ---
        if spec.attn_kind == AttnKind::Mla {
            let (mv, att) = crate::arch::forward_mla_attn(
                &spec,
                self.manifest,
                layer,
                pos,
                &prefix,
                hidden,
                s,
                kv,
                source,
                gpu,
                use_gpu,
                par,
                planner_ro,
                clock_ms,
            )?;
            timing.matvec_ms = mv;
            timing.attn_ms = att;
        } else if spec.attn_kind == AttnKind::Kda {
            let (mv, att) = crate::arch::forward_kda_attn(
                self.manifest,
                layer,
                pos,
                &prefix,
                hidden,
                s,
                kv,
                source,
                gpu,
                use_gpu,
                par,
                planner_ro,
                clock_ms,
            )?;
            timing.matvec_ms = mv;
            timing.attn_ms = att;
        } else if spec.attn_kind == AttnKind::Gated {
            let (mv, att) = crate::arch::forward_gated_attn(
                &spec,
                self.manifest,
                layer,
                pos,
                &prefix,
                hidden,
                s,
                kv,
                source,
                gpu,
                use_gpu,
                par,
                planner_ro,
                clock_ms,
            )?;
            timing.matvec_ms = mv;
            timing.attn_ms = att;
        } else if spec.attn_kind == AttnKind::Gdn {
            let (mv, att) = crate::arch::forward_gdn_attn(
                &spec,
                self.manifest,
                layer,
                pos,
                &prefix,
                hidden,
                s,
                kv,
                source,
                gpu,
                use_gpu,
                par,
                planner_ro,
                clock_ms,
            )?;
            timing.matvec_ms = mv;
            timing.attn_ms = att;
        } else {
        s.residual.copy_from_slice(hidden);
        source.load_f32(&format!("{prefix}.attn_norm"), &mut s.norm_w)?;
        let gpu_rms = use_gpu
            && gpu
                .as_deref_mut()
                .and_then(|g| crate::gpu::try_gpu_rmsnorm(g, hidden, &s.norm_w, eps).ok())
                .unwrap_or(false);
        if !gpu_rms {
            rmsnorm(hidden, &s.norm_w, eps);
        }

        let t_mv0 = tick(clock_ms);
        matvec_step(
            use_gpu,
            gpu,
            &name_attn_q,
            source.tensor_view(&name_attn_q)?,
            q_dim,
            h,
            hidden,
            &mut s.q[..q_dim],
            par,
            planner_ro,
            layer,
        )?;
        matvec_step(
            use_gpu,
            gpu,
            &name_attn_k,
            source.tensor_view(&name_attn_k)?,
            kv_dim,
            h,
            hidden,
            &mut s.k[..kv_dim],
            par,
            planner_ro,
            layer,
        )?;
        matvec_step(
            use_gpu,
            gpu,
            &name_attn_v,
            source.tensor_view(&name_attn_v)?,
            kv_dim,
            h,
            hidden,
            &mut s.v[..kv_dim],
            par,
            planner_ro,
            layer,
        )?;
        add_optional_bias(
            source,
            &format!("{name_attn_q}.bias"),
            &mut s.q[..q_dim],
            &mut s.attn_out,
        );
        add_optional_bias(
            source,
            &format!("{name_attn_k}.bias"),
            &mut s.k[..kv_dim],
            &mut s.attn_out,
        );
        add_optional_bias(
            source,
            &format!("{name_attn_v}.bias"),
            &mut s.v[..kv_dim],
            &mut s.attn_out,
        );
        if spec.flags & FLAG_QK_NORM != 0 {
            source.load_f32(&format!("{prefix}.attn_q_norm"), &mut s.head_out[..head_dim])?;
            for head in 0..heads {
                rmsnorm(
                    &mut s.q[head * head_dim..(head + 1) * head_dim],
                    &s.head_out[..head_dim],
                    eps,
                );
            }
            source.load_f32(&format!("{prefix}.attn_k_norm"), &mut s.head_out[..head_dim])?;
            for head in 0..kv_heads {
                rmsnorm(
                    &mut s.k[head * head_dim..(head + 1) * head_dim],
                    &s.head_out[..head_dim],
                    eps,
                );
            }
        }
        let t_attn0 = tick(clock_ms);

        let neox = spec.flags & FLAG_ROPE_NEOX != 0;
        // YaRN sólo en las capas de atención completa. Las de ventana (Mellum, 3 de cada 4) llevan
        // RoPE plano: llama.cpp les pone freq_scale=1 y ext_factor=0.
        if self.manifest.ext.yarn.activa() && ventana == 0 {
            rope_neox_yarn(&mut s.q[..q_dim], heads, head_dim, pos, theta, self.manifest.ext.yarn);
            rope_neox_yarn(&mut s.k[..kv_dim], kv_heads, head_dim, pos, theta, self.manifest.ext.yarn);
        } else {
            rope_cabezas(&mut s.q[..q_dim], heads, head_dim, pos, theta, neox);
            rope_cabezas(&mut s.k[..kv_dim], kv_heads, head_dim, pos, theta, neox);
        }

        if ventana > 0 {
            crate::gemma::expulsar_si_hace_falta(kv, ventana, kv_dim);
        }
        kv.append_f16(&s.k[..kv_dim], &s.v[..kv_dim]);
        let seq = kv.tokens(kv_dim);
        let t_ini = if ventana > 0 { seq.saturating_sub(ventana) } else { 0 };
        let _ = pos;
        // La ventana ya es la máscara del modelo: H2O y la atención dispersa no deben recortarla más.
        let sparse = t_ini == 0 && planner_ro.is_some_and(|p| p.use_sparse_attn(seq));
        let use_h2o = t_ini == 0 && planner_ro.is_some_and(|p| p.h2o_masas(seq));
        // Camino rápido: KV f16 denso sin masa → kernel por grupos GQA, repartido entre hilos.
        let fast_f16 = matches!(kv.dtype, KvDtype::F16) && !sparse && !use_h2o;
        if fast_f16 && t_ini == 0 {
            atencion_f16_grupos(
                par,
                &s.q[..q_dim],
                &mut s.attn_out[..q_dim],
                1,
                q_dim,
                kv,
                kv_heads,
                group,
                head_dim,
                kv_dim,
                seq,
            );
        } else if fast_f16 {
            // Ventana deslizante: el kernel rápido atiende desde el token 0. Éste, el de Gemma, no.
            let escala = 1.0 / libm::sqrtf(head_dim as f32);
            let k_f16 = kv.k_f16_slice();
            let v_f16 = kv.v_f16_slice();
            s.attn_out[..q_dim].fill(0.0);
            for kvh in 0..kv_heads {
                let a = kvh * group * head_dim;
                let b = a + group * head_dim;
                crate::attn_gemma::attention_gqa_f16(
                    &s.q[a..b],
                    group,
                    k_f16,
                    v_f16,
                    head_dim,
                    kv_dim,
                    kvh,
                    t_ini,
                    seq,
                    escala,
                    &mut s.attn_out[a..b],
                );
            }
        } else {
            if use_h2o && s.mass_buf.len() < seq {
                s.mass_buf.resize(seq, 0.0);
            }
            s.attn_out.fill(0.0);
            for head in 0..heads {
                let q_h = &s.q[head * head_dim..(head + 1) * head_dim];
                let kv_head = head / group;
                let mass = if use_h2o {
                    Some(&mut s.mass_buf[..seq])
                } else {
                    None
                };
                crate::attn::attention_decode_kv(
                    q_h,
                    kv,
                    head_dim,
                    kv_dim,
                    kv_head,
                    seq,
                    &mut s.head_out,
                    mass,
                    sparse,
                    t_ini,
                );
                s.attn_out[head * head_dim..(head + 1) * head_dim].copy_from_slice(&s.head_out);
                if use_h2o {
                    let inv_h = 1.0 / heads as f32;
                    for t in 0..seq {
                        if t < kv.mass.len() {
                            kv.mass[t] += s.mass_buf[t] * inv_h;
                        }
                    }
                }
            }
        }
        let t_attn1 = tick(clock_ms);

        // proyección de salida de la atención (Wo) y residual
        matvec_step(
            use_gpu,
            gpu,
            &name_attn_output,
            source.tensor_view(&name_attn_output)?,
            h,
            q_dim,
            &s.attn_out[..q_dim],
            &mut s.q[..h],
            par,
            planner_ro,
            layer,
        )?;
        add_f32(&s.residual, &s.q[..h], hidden);
        if clock_ms.is_some() {
            timing.matvec_ms = t_attn0.saturating_sub(t_mv0);
            timing.attn_ms = t_attn1.saturating_sub(t_attn0);
        }
        } // Gqa

        perfil_marca(&mut t_perfil, FASE_ATENCION);
        // --- FFN (denso, MoE o LatentMoE) ---
        s.residual.copy_from_slice(hidden);
        source.load_f32(&format!("{prefix}.ffn_norm"), &mut s.norm_w)?;
        let gpu_rms_ffn = use_gpu
            && gpu
                .as_deref_mut()
                .and_then(|g| crate::gpu::try_gpu_rmsnorm(g, hidden, &s.norm_w, eps).ok())
                .unwrap_or(false);
        if !gpu_rms_ffn {
            rmsnorm(hidden, &s.norm_w, eps);
        }

        let t_ffn0 = tick(clock_ms);
        if spec.ffn_kind == FfnKind::LatentMoe {
            let latent = self.manifest.effective_moe_ffn_dim(layer) as usize;
            crate::arch::forward_latent_moe_ffn(
                self.manifest,
                layer,
                prefix.clone(),
                h,
                latent,
                hidden,
                s,
                source,
                gpu,
                use_gpu,
                par,
                planner,
                index,
            )?;
        } else if self.manifest.layer_is_moe(layer) {
            self.forward_moe_ffn(
                layer,
                prefix,
                h,
                ffn,
                hidden,
                s,
                source,
                gpu,
                use_gpu,
                par,
                planner,
                index,
            )?;
        } else {
            if self.has_gate {
                let mut lote = Lote::nuevo();
                lote.anadir(layer, &name_ffn_up, source.tensor_view(&name_ffn_up)?, ffn, h, hidden, &mut s.up[..ffn])?;
                lote.anadir(layer, &name_ffn_gate, source.tensor_view(&name_ffn_gate)?, ffn, h, hidden, &mut s.gate[..ffn])?;
                lote.ejecutar(par, gpu, use_gpu)?;
                swiglu_inplace(&mut s.up[..ffn], &s.gate[..ffn]);
            } else {
                matvec_step(
                    use_gpu,
                    gpu,
                    &name_ffn_up,
                    source.tensor_view(&name_ffn_up)?,
                    ffn,
                    h,
                    hidden,
                    &mut s.up[..ffn],
                    par,
                    planner_ro,
                    layer,
                )?;
                silu_inplace(&mut s.up[..ffn]);
            }
            matvec_step(
                use_gpu,
                gpu,
                &name_ffn_down,
                source.tensor_view(&name_ffn_down)?,
                h,
                ffn,
                &s.up[..ffn],
                hidden,
                par,
                planner_ro,
                layer,
            )?;
        }

        add_assign_f32(hidden, &s.residual);
        let t_ffn1 = tick(clock_ms);
        if clock_ms.is_some() {
            timing.matvec_ms = timing
                .matvec_ms
                .saturating_add(t_ffn1.saturating_sub(t_ffn0));
        }
        perfil_marca(&mut t_perfil, FASE_FFN_UP_GATE);
        Ok(timing)
    }

    /// ¿Es una capa MoE de atención GQA clásica, de las que la GPU puede ejecutar enteras (`moe_gpu`)?
    pub fn es_capa_moe_gpu(&self, layer: u32) -> bool {
        let spec = self.manifest.layer(layer).cloned().unwrap_or_default();
        spec.attn_kind == AttnKind::Gqa
            && spec.ffn_kind != FfnKind::LatentMoe
            && self.manifest.layer_is_moe(layer)
            && !crate::gemma::es_capa_gemma(spec.flags)
    }

    /// ¿Se puede ejecutar esta capa por bloques (T82)? Sólo atención clásica y FFN denso
    /// (KV f16 o int8: `append` y la atención genérica ya saben de dtype): el resto (MLA,
    /// KDA, gated, GDN, MoE) sigue token a token.
    pub fn soporta_bloque(&self, layer: u32, kv: &LayerKv) -> bool {
        let _ = kv;
        let spec = self.manifest.layer(layer).cloned().unwrap_or_default();
        let atencion_clasica = !matches!(
            spec.attn_kind,
            AttnKind::Mla | AttnKind::Kda | AttnKind::Gated | AttnKind::Gdn
        );
        atencion_clasica
            && spec.ffn_kind != FfnKind::LatentMoe
            && !self.manifest.layer_is_moe(layer)
            && self.manifest.layer_window(layer) == 0
            && !self.manifest.ext.yarn.activa()
    }

    /// Una capa para `n` tokens consecutivos (posiciones `pos0..pos0+n`) a la vez.
    ///
    /// Las proyecciones y la FFN —el 99 % del coste del prefill— leen cada matriz de
    /// pesos una vez para los `n` vectores (`matvec_view_batch_par`). La atención va
    /// token a token con el mismo código de siempre y los K/V se añaden en orden, así
    /// que el resultado es **idéntico bit a bit** al de `forward_layer` repetido `n`
    /// veces. `hs` son `n` vectores de `hidden_dim` seguidos y se actualizan en su sitio.
    pub fn forward_layer_block<S: TensorSource>(
        &self,
        layer: u32,
        pos0: usize,
        n: usize,
        hs: &mut [f32],
        bs: &mut BlockScratch,
        s: &mut LayerScratch,
        kv: &mut LayerKv,
        source: &mut S,
        planner: Option<&crate::plan::ResourcePlanner>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        use_gpu: bool,
    ) -> Result<(), ()> {
        if crate::bloque35::soporta(self, layer, kv) {
            return crate::bloque35::forward_layer_block_q35(self, layer, pos0, n, hs, bs, s, kv, source, gpu, use_gpu);
        }
        if crate::gemma::es_capa_gemma(self.manifest.layer(layer).map_or(0, |l| l.flags)) {
            return crate::gemma::forward_layer_gemma_block(self, layer, pos0, n, hs, bs, s, kv, source, gpu, use_gpu);
        }
        // MoE con GPU: el bloque entero en el dispositivo; si la GPU no puede, token a token por el camino de siempre.
        if use_gpu && self.es_capa_moe_gpu(layer) {
            if crate::moe_gpu::capa_moe_en_gpu(self, layer, pos0, n, hs, kv, source, gpu, use_gpu)? {
                return Ok(());
            }
            let h = self.manifest.hidden_dim as usize;
            for t in 0..n {
                self.forward_layer(layer, pos0 + t, &mut hs[t * h..(t + 1) * h], s, kv, source, gpu, use_gpu, None, None, None)?;
            }
            return Ok(());
        }
        let h = self.manifest.hidden_dim as usize;
        let heads = self.manifest.effective_num_heads(layer) as usize;
        let kv_heads = self.manifest.effective_num_kv_heads(layer) as usize;
        let head_dim = self.manifest.effective_head_dim(layer) as usize;
        let kv_dim = kv_heads * head_dim;
        let q_dim = heads * head_dim;
        let group = heads / kv_heads;
        let ffn = self.manifest.layer_expert_ffn_dim(layer) as usize;
        let eps = self.manifest.rms_eps;
        let theta = self.manifest.rope_theta;
        let prefix = format!("L{layer:02}");
        let spec = self.manifest.layer(layer).cloned().unwrap_or_default();
        let seq_par = Sequential;
        let par: &dyn RowParallel = self.parallel.unwrap_or(&seq_par);
        if hs.len() != n * h || n == 0 || n > bs.cap {
            return Err(());
        }
        let max_cols = h.max(q_dim).max(ffn);
        bs.ensure_q8k(n, max_cols);

        let mut t_perfil = perfil_ahora();
        // --- atención ---
        bs.residual[..n * h].copy_from_slice(hs);
        source.load_f32(&format!("{prefix}.attn_norm"), &mut s.norm_w)?;
        for i in 0..n {
            rmsnorm(&mut hs[i * h..(i + 1) * h], &s.norm_w, eps);
        }
        perfil_marca(&mut t_perfil, FASE_OTROS);
        for (clave, dim, destino) in [
            ("attn_q", q_dim, 0u8),
            ("attn_k", kv_dim, 1u8),
            ("attn_v", kv_dim, 2u8),
        ] {
            let name = format!("{prefix}.{clave}");
            note_infer_op(Some(layer), &name);
            let vista = source.tensor_view(&name)?;
            let out = match destino {
                0 => &mut bs.q[..n * dim],
                1 => &mut bs.k[..n * dim],
                _ => &mut bs.v[..n * dim],
            };
            matvec_view_batch_par(&vista, dim, h, hs, n, out, par, Some(&mut bs.q8k_x))?;
        }
        perfil_marca(&mut t_perfil, FASE_QKV);
        for (clave, dim, destino) in [
            ("attn_q", q_dim, 0u8),
            ("attn_k", kv_dim, 1u8),
            ("attn_v", kv_dim, 2u8),
        ] {
            // El sesgo se lee una vez y se suma a cada token (misma suma que por token).
            let name = format!("{prefix}.{clave}.bias");
            if s.attn_out.len() >= dim && source.load_f32(&name, &mut s.attn_out[..dim]).is_ok() {
                for i in 0..n {
                    let out = match destino {
                        0 => &mut bs.q[i * dim..(i + 1) * dim],
                        1 => &mut bs.k[i * dim..(i + 1) * dim],
                        _ => &mut bs.v[i * dim..(i + 1) * dim],
                    };
                    add_assign_f32(out, &s.attn_out[..dim]);
                }
            }
        }
        if spec.flags & FLAG_QK_NORM != 0 {
            source.load_f32(&format!("{prefix}.attn_q_norm"), &mut s.head_out[..head_dim])?;
            for i in 0..n {
                for head in 0..heads {
                    let o = i * q_dim + head * head_dim;
                    rmsnorm(&mut bs.q[o..o + head_dim], &s.head_out[..head_dim], eps);
                }
            }
            source.load_f32(&format!("{prefix}.attn_k_norm"), &mut s.head_out[..head_dim])?;
            for i in 0..n {
                for head in 0..kv_heads {
                    let o = i * kv_dim + head * head_dim;
                    rmsnorm(&mut bs.k[o..o + head_dim], &s.head_out[..head_dim], eps);
                }
            }
        }
        // RoPE y K/V de los n tokens, en orden; el KV acaba con `seq_antes + n` posiciones.
        let seq_antes = kv.tokens(kv_dim);
        for i in 0..n {
            let pos = pos0 + i;
            let neox = spec.flags & FLAG_ROPE_NEOX != 0;
            rope_cabezas(&mut bs.q[i * q_dim..(i + 1) * q_dim], heads, head_dim, pos, theta, neox);
            rope_cabezas(&mut bs.k[i * kv_dim..(i + 1) * kv_dim], kv_heads, head_dim, pos, theta, neox);
            kv.append_f16(
                &bs.k[i * kv_dim..(i + 1) * kv_dim],
                &bs.v[i * kv_dim..(i + 1) * kv_dim],
            );
        }
        perfil_marca(&mut t_perfil, FASE_ROPE_KV);
        // Atención causal: el token i ve `seq_antes + i + 1` posiciones. Con KV f16 y sin masa de
        // H2O ni dispersa (lo habitual) van todos a la vez, repartidos entre hilos.
        let sparse_max = planner.is_some_and(|p| p.use_sparse_attn(seq_antes + n));
        let h2o_max = planner.is_some_and(|p| p.h2o_masas(seq_antes + n));
        if matches!(kv.dtype, KvDtype::F16) && !sparse_max && !h2o_max {
            atencion_f16_grupos(
                par,
                &bs.q[..n * q_dim],
                &mut bs.attn[..n * q_dim],
                n,
                q_dim,
                kv,
                kv_heads,
                group,
                head_dim,
                kv_dim,
                seq_antes + 1,
            );
        } else {
            for i in 0..n {
                let seq = seq_antes + i + 1;
                // Misma decisión que `forward_layer`: así bloque y token a token coinciden.
                let sparse = planner.is_some_and(|p| p.use_sparse_attn(seq));
                let use_h2o = planner.is_some_and(|p| p.h2o_masas(seq));
                let fast_f16 = matches!(kv.dtype, KvDtype::F16) && !sparse && !use_h2o;
                if fast_f16 {
                    let o = i * q_dim;
                    atencion_f16_grupos(
                        par,
                        &bs.q[o..o + q_dim],
                        &mut bs.attn[o..o + q_dim],
                        1,
                        q_dim,
                        kv,
                        kv_heads,
                        group,
                        head_dim,
                        kv_dim,
                        seq,
                    );
                    continue;
                }
                if use_h2o && s.mass_buf.len() < seq {
                    s.mass_buf.resize(seq, 0.0);
                }
                for head in 0..heads {
                    let o = i * q_dim + head * head_dim;
                    let kv_head = head / group;
                    let mass = if use_h2o {
                        Some(&mut s.mass_buf[..seq])
                    } else {
                        None
                    };
                    crate::attn::attention_decode_kv(
                        &bs.q[o..o + head_dim],
                        kv,
                        head_dim,
                        kv_dim,
                        kv_head,
                        seq,
                        &mut s.head_out,
                        mass,
                        sparse,
                        0,
                    );
                    bs.attn[o..o + head_dim].copy_from_slice(&s.head_out[..head_dim]);
                    if use_h2o {
                        let inv_h = 1.0 / heads as f32;
                        for t in 0..seq {
                            if t < kv.mass.len() {
                                kv.mass[t] += s.mass_buf[t] * inv_h;
                            }
                        }
                    }
                }
            }
        }
        perfil_marca(&mut t_perfil, FASE_ATENCION);
        {
            let name = format!("{prefix}.attn_output");
            note_infer_op(Some(layer), &name);
            let vista = source.tensor_view(&name)?;
            matvec_view_batch_par(
                &vista,
                h,
                q_dim,
                &bs.attn[..n * q_dim],
                n,
                &mut bs.o[..n * h],
                par,
                Some(&mut bs.q8k_x),
            )?;
        }
        for i in 0..n {
            add_f32(
                &bs.residual[i * h..(i + 1) * h],
                &bs.o[i * h..(i + 1) * h],
                &mut hs[i * h..(i + 1) * h],
            );
        }

        perfil_marca(&mut t_perfil, FASE_O);
        // --- FFN denso ---
        bs.residual[..n * h].copy_from_slice(hs);
        source.load_f32(&format!("{prefix}.ffn_norm"), &mut s.norm_w)?;
        for i in 0..n {
            rmsnorm(&mut hs[i * h..(i + 1) * h], &s.norm_w, eps);
        }
        perfil_marca(&mut t_perfil, FASE_OTROS);
        {
            let name = format!("{prefix}.ffn_up");
            note_infer_op(Some(layer), &name);
            let vista = source.tensor_view(&name)?;
            matvec_view_batch_par(
                &vista,
                ffn,
                h,
                hs,
                n,
                &mut bs.up[..n * ffn],
                par,
                Some(&mut bs.q8k_x),
            )?;
        }
        if self.has_gate {
            let name = format!("{prefix}.ffn_gate");
            note_infer_op(Some(layer), &name);
            let vista = source.tensor_view(&name)?;
            matvec_view_batch_par(
                &vista,
                ffn,
                h,
                hs,
                n,
                &mut bs.gate[..n * ffn],
                par,
                Some(&mut bs.q8k_x),
            )?;
            perfil_marca(&mut t_perfil, FASE_FFN_UP_GATE);
            swiglu_par(par, &mut bs.up[..n * ffn], &bs.gate[..n * ffn]);
        } else {
            for i in 0..n {
                silu_inplace(&mut bs.up[i * ffn..(i + 1) * ffn]);
            }
        }
        perfil_marca(&mut t_perfil, FASE_SWIGLU);
        {
            let name = format!("{prefix}.ffn_down");
            note_infer_op(Some(layer), &name);
            let vista = source.tensor_view(&name)?;
            matvec_view_batch_par(
                &vista,
                h,
                ffn,
                &bs.up[..n * ffn],
                n,
                hs,
                par,
                Some(&mut bs.q8k_x),
            )?;
        }
        perfil_marca(&mut t_perfil, FASE_FFN_DOWN);
        for i in 0..n {
            add_assign_f32(&mut hs[i * h..(i + 1) * h], &bs.residual[i * h..(i + 1) * h]);
        }
        perfil_marca(&mut t_perfil, FASE_OTROS);
        Ok(())
    }

    /// Expertos del MoE (los elegidos y los compartidos) con **una región paralela por fase** en vez de una por matvec:
    /// con matrices de 512×2048 el trabajo de cada una es menor que la barrera del pool, y 27 matvec por capa
    /// dominaban el decode del 35B-A3B. Fases: gate+up de todos, SwiGLU, down de todos.
    fn moe_expertos_agrupados<S: TensorSource>(
        &self,
        layer: u32,
        prefix: &str,
        h: usize,
        ffn: usize,
        ranked: &[(usize, f32)],
        hidden: &mut [f32],
        s: &mut LayerScratch,
        source: &mut S,
        par: &dyn RowParallel,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        use_gpu: bool,
    ) -> Result<(), ()> {
        let mut t_perfil = perfil_ahora();
        let n_shared = self.manifest.layer(layer).map(|sp| sp.num_shared_experts).unwrap_or(0) as usize;
        let mut nombres: Vec<String> = Vec::with_capacity(ranked.len() + n_shared);
        for (e, _) in ranked {
            nombres.push(format!("{prefix}.E{e:02}"));
        }
        for sh in 0..n_shared {
            nombres.push(format!("{prefix}.S{sh:02}"));
        }
        let n = nombres.len();
        // Las vistas apuntan al mmap del shard, que vive en la fuente hasta que se liberen los shards: aquí no se libera.
        let mut vistas: Vec<[(usize, usize, u8, usize); 3]> = Vec::with_capacity(n);
        for nom in &nombres {
            let mut v3 = [(0usize, 0usize, 0u8, 0usize); 3];
            for (i, suf) in ["ffn_gate", "ffn_up", "ffn_down"].iter().enumerate() {
                let name = format!("{nom}.{suf}");
                note_infer_op(Some(layer), &name);
                let v = source.tensor_view(&name)?;
                v3[i] = (v.bytes.as_ptr() as usize, v.bytes.len(), v.dtype, v.elems);
            }
            vistas.push(v3);
        }
        perfil_marca(&mut t_perfil, FASE_QKV);
        let mut puerta_compartido = 1.0f32;
        let name_pg = format!("{prefix}.ffn_gate_inp_shexp");
        let pg = if n_shared > 0 { source.tensor_view(&name_pg).ok().map(|v| (v.bytes.as_ptr() as usize, v.bytes.len(), v.dtype, v.elems)) } else { None };
        let mut gates = vec![0.0f32; n * ffn];
        let mut ups = vec![0.0f32; n * ffn];
        let mut pg_out = [0.0f32; 1];
        let mut jobs: Vec<TrabajoMatvec> = Vec::with_capacity(2 * n + 1);
        let mut claves: Vec<String> = Vec::with_capacity(2 * n + 1);
        for (i, v3) in vistas.iter().enumerate() {
            for (k, buf) in [(0usize, &mut gates), (1usize, &mut ups)] {
                let (p, l, d, _) = v3[k];
                claves.push(format!("{}.{}", nombres[i], if k == 0 { "ffn_gate" } else { "ffn_up" }));
                jobs.push(TrabajoMatvec { bytes: p as *const u8, len: l, dtype: d, rows: ffn, cols: h, x: hidden.as_ptr(), out: buf[i * ffn..].as_mut_ptr() });
            }
        }
        if let Some((p, l, d, _)) = pg {
            claves.push(name_pg.clone());
            jobs.push(TrabajoMatvec { bytes: p as *const u8, len: l, dtype: d, rows: 1, cols: h, x: hidden.as_ptr(), out: pg_out.as_mut_ptr() });
        }
        perfil_marca(&mut t_perfil, FASE_ROPE_KV);
        ejecutar_lote(&jobs, &claves, par, gpu, use_gpu)?;
        perfil_marca(&mut t_perfil, FASE_O);
        if pg.is_some() {
            puerta_compartido = 1.0 / (1.0 + libm::expf(-pg_out[0]));
        }
        for i in 0..n {
            swiglu_inplace(&mut ups[i * ffn..(i + 1) * ffn], &gates[i * ffn..(i + 1) * ffn]);
        }
        perfil_marca(&mut t_perfil, FASE_SWIGLU);
        let mut downs = vec![0.0f32; n * h];
        jobs.clear();
        claves.clear();
        for (i, v3) in vistas.iter().enumerate() {
            let (p, l, d, _) = v3[2];
            claves.push(format!("{}.ffn_down", nombres[i]));
            jobs.push(TrabajoMatvec { bytes: p as *const u8, len: l, dtype: d, rows: h, cols: ffn, x: ups[i * ffn..].as_ptr(), out: downs[i * h..].as_mut_ptr() });
        }
        ejecutar_lote(&jobs, &claves, par, gpu, use_gpu)?;
        perfil_marca(&mut t_perfil, FASE_FFN_DOWN);
        s.moe_acc.fill(0.0);
        for (i, (_, w)) in ranked.iter().enumerate() {
            for j in 0..h {
                s.moe_acc[j] += downs[i * h + j] * w;
            }
        }
        for i in ranked.len()..n {
            for j in 0..h {
                s.moe_acc[j] += downs[i * h + j] * puerta_compartido;
            }
        }
        hidden.copy_from_slice(&s.moe_acc);
        Ok(())
    }

    /// FFN MoE estilo Mixtral: router → top-k → SwiGLU por experto → suma ponderada.
    fn forward_moe_ffn<S: TensorSource>(
        &self,
        layer: u32,
        prefix: alloc::string::String,
        h: usize,
        ffn: usize,
        hidden: &mut [f32],
        s: &mut LayerScratch,
        source: &mut S,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        use_gpu: bool,
        par: &dyn RowParallel,
        mut planner: Option<&mut crate::plan::ResourcePlanner>,
        index: Option<&TensorIndex>,
    ) -> Result<(), ()> {
        let n_exp = self.manifest.effective_num_experts(layer) as usize;
        let top_k = self.manifest.effective_num_experts_per_tok(layer) as usize;
        if n_exp == 0 || top_k == 0 || s.router.len() < n_exp {
            return Err(());
        }
        let name_router = format!("{prefix}.ffn_gate_inp");
        matvec_step(
            use_gpu,
            gpu,
            &name_router,
            source.tensor_view(&name_router)?,
            n_exp,
            h,
            hidden,
            &mut s.router[..n_exp],
            par,
            planner.as_deref(),
            layer,
        )?;
        let ranked = topk_softmax(&mut s.router[..n_exp], top_k);
        source.wait_moe_prefetch();
        let prefetch_shards = if let (Some(pl), Some(idx)) = (planner.as_mut(), index) {
            let ids: alloc::vec::Vec<(u32, f32)> =
                ranked.iter().map(|(i, w)| (*i as u32, *w)).collect();
            pl.touch_moe_experts(layer, &ids, idx)
        } else {
            alloc::vec::Vec::new()
        };
        if !prefetch_shards.is_empty() {
            source.prefetch_shards(&prefetch_shards);
            if let Some(pl) = planner.as_mut() {
                pl.note_moe_jit_served(pl.last_moe_cold());
            }
        }

        if !use_gpu || gpu.as_deref().is_some_and(|g| g.available()) {
            return self.moe_expertos_agrupados(layer, &prefix, h, ffn, &ranked, hidden, s, source, par, gpu, use_gpu);
        }
        s.moe_acc.fill(0.0);
        for (expert, weight) in &ranked {
            let ep = format!("{prefix}.E{expert:02}");
            let name_gate = format!("{ep}.ffn_gate");
            let name_up = format!("{ep}.ffn_up");
            let name_down = format!("{ep}.ffn_down");
            matvec_step(
                use_gpu,
                gpu,
                &name_gate,
                source.tensor_view(&name_gate)?,
                ffn,
                h,
                hidden,
                &mut s.gate[..ffn],
                par,
                planner.as_deref(),
                layer,
            )?;
            matvec_step(
                use_gpu,
                gpu,
                &name_up,
                source.tensor_view(&name_up)?,
                ffn,
                h,
                hidden,
                &mut s.up[..ffn],
                par,
                planner.as_deref(),
                layer,
            )?;
            swiglu_inplace(&mut s.up[..ffn], &s.gate[..ffn]);
            matvec_step(
                use_gpu,
                gpu,
                &name_down,
                source.tensor_view(&name_down)?,
                h,
                ffn,
                &s.up[..ffn],
                &mut s.q[..h],
                par,
                planner.as_deref(),
                layer,
            )?;
            for i in 0..h {
                s.moe_acc[i] += s.q[i] * weight;
            }
        }
        let n_shared = self
            .manifest
            .layer(layer)
            .map(|sp| sp.num_shared_experts)
            .unwrap_or(0);
        // qwen35moe: el experto compartido lleva una puerta sigmoide escalar (`ffn_gate_inp_shexp`).
        let mut puerta_compartido = 1.0f32;
        if n_shared > 0 {
            let name_pg = format!("{prefix}.ffn_gate_inp_shexp");
            if let Ok(view) = source.tensor_view(&name_pg) {
                matvec_step(use_gpu, gpu, &name_pg, view, 1, h, hidden, &mut s.router[..1], par, planner.as_deref(), layer)?;
                puerta_compartido = 1.0 / (1.0 + libm::expf(-s.router[0]));
            }
        }
        for shared in 0..n_shared {
            let ep = format!("{prefix}.S{shared:02}");
            let name_gate = format!("{ep}.ffn_gate");
            let name_up = format!("{ep}.ffn_up");
            let name_down = format!("{ep}.ffn_down");
            matvec_step(
                use_gpu,
                gpu,
                &name_gate,
                source.tensor_view(&name_gate)?,
                ffn,
                h,
                hidden,
                &mut s.gate[..ffn],
                par,
                planner.as_deref(),
                layer,
            )?;
            matvec_step(
                use_gpu,
                gpu,
                &name_up,
                source.tensor_view(&name_up)?,
                ffn,
                h,
                hidden,
                &mut s.up[..ffn],
                par,
                planner.as_deref(),
                layer,
            )?;
            swiglu_inplace(&mut s.up[..ffn], &s.gate[..ffn]);
            matvec_step(
                use_gpu,
                gpu,
                &name_down,
                source.tensor_view(&name_down)?,
                h,
                ffn,
                &s.up[..ffn],
                &mut s.q[..h],
                par,
                planner.as_deref(),
                layer,
            )?;
            for i in 0..h {
                s.moe_acc[i] += s.q[i] * puerta_compartido;
            }
        }
        hidden.copy_from_slice(&s.moe_acc);
        Ok(())
    }
}

#[cfg(test)]
mod last_op_tests {
    use super::*;

    #[test]
    fn last_infer_op_recuerda_capa_y_tensor() {
        clear_infer_op();
        note_infer_op(Some(3), "L03.attn_q");
        let (l, k) = last_infer_op();
        assert_eq!(l, Some(3));
        assert_eq!(k, "L03.attn_q");
        clear_infer_op();
        assert_eq!(last_infer_op(), (None, String::new()));
    }
}
