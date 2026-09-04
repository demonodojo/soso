//! Ejecutor capa a capa: atención (RoPE + GQA), FFN SwiGLU y residuales.
//!
//! Los pesos se leen como vistas zero-copy (`TensorView`) sobre los shards
//! mapeados: no hay copia de matrices por token, solo lecturas en streaming
//! durante el matvec. El KV cache es f16 o int8 (KIVI-lite) según el planner.

use crate::gemm::{
    add_assign_f32, add_f32, matvec_f32, matvec_q4_k, matvec_q8_0, rmsnorm, rope_inplace,
    silu_inplace, swiglu_inplace, topk_softmax,
};
pub use crate::kv::{KvDtype, LayerKv};
use crate::parallel::{RowParallel, Sequential};
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use sosomodel::index::TensorIndex;
use sosomodel::layout::{DTYPE_F32, DTYPE_MXFP4, DTYPE_Q4_K, DTYPE_Q8_0};
use sosomodel::manifest::{AttnKind, FfnKind, Manifest};

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
                    if heads > 0 {
                        let classic = h / heads;
                        max_k = max_k.max(kv_heads.saturating_mul(classic));
                        max_v = max_v.max(kv_heads.saturating_mul(classic));
                        max_head = max_head.max(classic.max(1));
                    }
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
        let head_dim = h / heads;
        let kv_dim = kv_heads * head_dim;
        let group = heads / kv_heads;
        let ffn = self.manifest.layer_expert_ffn_dim(layer) as usize;
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
        let mut timing = LayerTiming::default();

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
        rmsnorm(hidden, &s.norm_w, eps);

        let t_mv0 = tick(clock_ms);
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
            &mut s.k,
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
            &mut s.v,
            par,
            planner_ro,
            layer,
        )?;
        let t_attn0 = tick(clock_ms);

        for head in 0..heads {
            rope_inplace(&mut s.q[head * head_dim..(head + 1) * head_dim], pos, theta);
        }
        for head in 0..kv_heads {
            rope_inplace(&mut s.k[head * head_dim..(head + 1) * head_dim], pos, theta);
        }

        kv.append_f16(&s.k, &s.v);
        let seq = kv.tokens(kv_dim);
        let _ = pos;
        let sparse = planner_ro.is_some_and(|p| p.use_sparse_attn(seq));
        let use_h2o = planner_ro.is_some_and(|p| p.use_h2o());
        // Fast path: KV f16 denso sin masa → tiled SIMD (sin dequant por token).
        let fast_f16 = matches!(kv.dtype, KvDtype::F16) && !sparse && !use_h2o;
        if use_h2o && s.mass_buf.len() < seq {
            s.mass_buf.resize(seq, 0.0);
        }
        s.attn_out.fill(0.0);
        for head in 0..heads {
            let q_h = &s.q[head * head_dim..(head + 1) * head_dim];
            let kv_head = head / group;
            if fast_f16 {
                crate::attn::attention_decode_f16_tiled(
                    q_h,
                    kv.k_f16_slice(),
                    kv.v_f16_slice(),
                    head_dim,
                    kv_dim,
                    kv_head,
                    seq,
                    &mut s.head_out,
                );
            } else {
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
                );
            }
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
        let t_attn1 = tick(clock_ms);

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
            planner_ro,
            layer,
        )?;
        add_f32(&s.residual, &s.q, hidden);
        if clock_ms.is_some() {
            timing.matvec_ms = t_attn0.saturating_sub(t_mv0);
            timing.attn_ms = t_attn1.saturating_sub(t_attn0);
        }
        } // Gqa

        // --- FFN (denso, MoE o LatentMoE) ---
        s.residual.copy_from_slice(hidden);
        source.load_f32(&format!("{prefix}.ffn_norm"), &mut s.norm_w)?;
        rmsnorm(hidden, &s.norm_w, eps);

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
            if self.has_gate {
                matvec_step(
                    use_gpu,
                    gpu,
                    &name_ffn_gate,
                    source.tensor_view(&name_ffn_gate)?,
                    ffn,
                    h,
                    hidden,
                    &mut s.gate[..ffn],
                    par,
                    planner_ro,
                    layer,
                )?;
                swiglu_inplace(&mut s.up[..ffn], &s.gate[..ffn]);
            } else {
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
        Ok(timing)
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
                &mut s.q,
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
                &mut s.q,
                par,
                planner.as_deref(),
                layer,
            )?;
            for i in 0..h {
                s.moe_acc[i] += s.q[i];
            }
        }
        hidden.copy_from_slice(&s.moe_acc);
        Ok(())
    }
}
