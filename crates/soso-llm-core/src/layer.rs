//! Ejecutor capa a capa: atención (RoPE + GQA), FFN SwiGLU y residuales.
//!
//! Los pesos se leen como vistas zero-copy (`TensorView`) sobre los shards
//! mapeados: no hay copia de matrices por token, solo lecturas en streaming
//! durante el matvec. El KV cache se guarda en f16 (mitad de ancho de banda).

use crate::f16::f32_to_f16;
use crate::gemm::{matvec_f32, matvec_q4_k, matvec_q8_0, rmsnorm, rope_inplace, silu};
use crate::parallel::{RowParallel, Sequential};
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use sosomodel::layout::{DTYPE_F32, DTYPE_Q4_K, DTYPE_Q8_0};
use sosomodel::manifest::Manifest;

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
    /// Liberar shards fuera del working set (streaming FlexGen/LayerKV).
    fn release_shards_except(&mut self, _keep: &[alloc::string::String]) {}
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
            _ => false,
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

/// KV cache por capa en f16.
///
/// Estilo PagedAttention/StreamingLLM: capacidad pre-reservada y ventana
/// deslizante (sink + recientes) cuando el presupuesto de memoria no permite
/// la secuencia completa.
pub struct LayerKv {
    pub k: Vec<u16>,
    pub v: Vec<u16>,
}

impl LayerKv {
    pub fn new() -> Self {
        Self {
            k: Vec::new(),
            v: Vec::new(),
        }
    }

    /// Reserva capacidad para `tokens` posiciones × `kv_dim` elementos f16.
    pub fn with_capacity(tokens: usize, kv_dim: usize) -> Self {
        let n = tokens.saturating_mul(kv_dim);
        Self {
            k: Vec::with_capacity(n),
            v: Vec::with_capacity(n),
        }
    }

    pub fn reset(&mut self) {
        self.k.clear();
        self.v.clear();
    }

    pub fn tokens(&self, kv_dim: usize) -> usize {
        if kv_dim == 0 {
            0
        } else {
            self.k.len() / kv_dim
        }
    }

    pub fn append_f16(&mut self, k: &[f32], v: &[f32]) {
        self.k.extend(k.iter().map(|&x| f32_to_f16(x)));
        self.v.extend(v.iter().map(|&x| f32_to_f16(x)));
    }

    /// Ventana StreamingLLM: conserva `sink` tokens iniciales + los más recientes
    /// hasta `keep` en total. Los K/V ya llevan RoPE aplicado; truncar el frente
    /// no invalida las posiciones restantes.
    pub fn slide_window(&mut self, keep: usize, sink: usize, kv_dim: usize) {
        let n = self.tokens(kv_dim);
        if n <= keep || keep == 0 || kv_dim == 0 {
            return;
        }
        let sink = sink.min(keep);
        let drop = n - keep;
        let start_recent = sink + drop;
        let mut new_k = Vec::with_capacity(keep * kv_dim);
        let mut new_v = Vec::with_capacity(keep * kv_dim);
        new_k.extend_from_slice(&self.k[..sink * kv_dim]);
        new_v.extend_from_slice(&self.v[..sink * kv_dim]);
        new_k.extend_from_slice(&self.k[start_recent * kv_dim..]);
        new_v.extend_from_slice(&self.v[start_recent * kv_dim..]);
        self.k = new_k;
        self.v = new_v;
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
    pub scores: Vec<f32>,
    pub head_out: Vec<f32>,
}

impl LayerScratch {
    pub fn new(m: &Manifest) -> Self {
        let h = m.hidden_dim as usize;
        let ffn = m.ffn_dim as usize;
        let heads = m.num_heads as usize;
        let head_dim = h / heads;
        let kv_dim = m.num_kv_heads as usize * head_dim;
        Self {
            residual: vec![0.0; h],
            norm_w: vec![0.0; h],
            q: vec![0.0; h],
            k: vec![0.0; kv_dim],
            v: vec![0.0; kv_dim],
            up: vec![0.0; ffn],
            gate: vec![0.0; ffn],
            attn_out: vec![0.0; h],
            scores: Vec::new(),
            head_out: vec![0.0; head_dim],
        }
    }
}

/// matvec con offload GPU opcional (G5).
fn matvec_step(
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
    let gpu_ok = use_gpu
        && planner
            .map(|p| p.gpu_tensor_allowed(layer, key))
            .unwrap_or(true);
    if gpu_ok {
        if let Some(g) = gpu.as_deref_mut() {
            if crate::gpu::try_gpu_matvec(g, key, &v, rows, cols, x, out)? {
                return Ok(());
            }
        }
    }
    matvec_view_par(&v, rows, cols, x, out, par)
}

pub struct LayerExecutor<'a> {
    pub manifest: &'a Manifest,
    /// El modelo trae proyección ffn_gate (SwiGLU completo).
    pub has_gate: bool,
    /// Paralelismo de matvec (None = secuencial).
    pub parallel: Option<&'a dyn RowParallel>,
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
        planner: Option<&crate::plan::ResourcePlanner>,
    ) -> Result<(), ()> {
        let h = self.manifest.hidden_dim as usize;
        let heads = self.manifest.num_heads as usize;
        let kv_heads = self.manifest.num_kv_heads as usize;
        let head_dim = h / heads;
        let kv_dim = kv_heads * head_dim;
        let group = heads / kv_heads;
        let ffn = self.manifest.ffn_dim as usize;
        let eps = self.manifest.rms_eps;
        let theta = self.manifest.rope_theta;
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

        // --- atención ---
        s.residual.copy_from_slice(hidden);
        source.load_f32(&format!("{prefix}.attn_norm"), &mut s.norm_w)?;
        rmsnorm(hidden, &s.norm_w, eps);

        matvec_step(
            use_gpu,
            gpu,
            &name_attn_q,
            source.tensor_view(&name_attn_q)?,
            h,
            h,
            hidden,
            &mut s.q,
            par,
            planner,
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
            &mut s.k,
            par,
            planner,
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
            &mut s.v,
            par,
            planner,
            layer,
        )?;

        for head in 0..heads {
            rope_inplace(&mut s.q[head * head_dim..(head + 1) * head_dim], pos, theta);
        }
        for head in 0..kv_heads {
            rope_inplace(&mut s.k[head * head_dim..(head + 1) * head_dim], pos, theta);
        }

        kv.append_f16(&s.k, &s.v);
        // Longitud real del cache (ventana deslizante puede ser < pos+1).
        let seq = kv.tokens(kv_dim);
        let _ = pos; // RoPE ya aplicado con posición absoluta del token actual

        // FlashAttention-style decode: online softmax por tiles de KV.
        s.attn_out.fill(0.0);
        for head in 0..heads {
            let q_h = &s.q[head * head_dim..(head + 1) * head_dim];
            let kv_head = head / group;
            crate::attn::attention_decode_f16_tiled(
                q_h,
                &kv.k,
                &kv.v,
                head_dim,
                kv_dim,
                kv_head,
                seq,
                &mut s.head_out,
            );
            s.attn_out[head * head_dim..(head + 1) * head_dim].copy_from_slice(&s.head_out);
        }

        // proyección de salida de la atención (Wo) y residual
        matvec_step(
            use_gpu,
            gpu,
            &name_attn_output,
            source.tensor_view(&name_attn_output)?,
            h,
            h,
            &s.attn_out,
            &mut s.q,
            par,
            planner,
            layer,
        )?;
        for i in 0..h {
            hidden[i] = s.residual[i] + s.q[i];
        }

        // --- FFN ---
        s.residual.copy_from_slice(hidden);
        source.load_f32(&format!("{prefix}.ffn_norm"), &mut s.norm_w)?;
        rmsnorm(hidden, &s.norm_w, eps);

        matvec_step(
            use_gpu,
            gpu,
            &name_ffn_up,
            source.tensor_view(&name_ffn_up)?,
            ffn,
            h,
            hidden,
            &mut s.up,
            par,
            planner,
            layer,
        )?;
        if self.has_gate {
            matvec_step(
                use_gpu,
                gpu,
                &name_ffn_gate,
                source.tensor_view(&name_ffn_gate)?,
                ffn,
                h,
                hidden,
                &mut s.gate,
                par,
                planner,
                layer,
            )?;
            for i in 0..ffn {
                s.up[i] = silu(s.gate[i]) * s.up[i];
            }
        } else {
            for x in s.up.iter_mut() {
                *x = silu(*x);
            }
        }
        matvec_step(
            use_gpu,
            gpu,
            &name_ffn_down,
            source.tensor_view(&name_ffn_down)?,
            h,
            ffn,
            &s.up,
            hidden,
            par,
            planner,
            layer,
        )?;

        for i in 0..h {
            hidden[i] += s.residual[i];
        }
        Ok(())
    }
}
