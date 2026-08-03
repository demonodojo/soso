//! Runtime de inferencia: bucle de generación de tokens.

use crate::gemm::rmsnorm;
use crate::kv::LayerKv;
use crate::layer::{matvec_view_par, LayerExecutor, LayerScratch, TensorSource, TensorView};
use crate::parallel::{RowParallel, Sequential};
use crate::plan::{ExecDest, ResourcePlanner};
use crate::tier::TierManager;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::pipeline::PipelineRole;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::{AttnKind, Manifest};

fn kv_storage_dim(manifest: &Manifest, layer: u32) -> usize {
    let spec = manifest.layer(layer).cloned().unwrap_or_default();
    if spec.attn_kind == AttnKind::Mla && spec.kv_lora_rank > 0 {
        spec.kv_lora_rank as usize
    } else {
        let head_dim =
            (manifest.hidden_dim / manifest.effective_num_heads(layer)) as usize;
        manifest.effective_num_kv_heads(layer) as usize * head_dim
    }
}

fn make_layer_kv(
    manifest: &Manifest,
    layer: u32,
    kv_cap: usize,
    dtype: crate::kv::KvDtype,
) -> LayerKv {
    let spec = manifest.layer(layer).cloned().unwrap_or_default();
    if spec.attn_kind == AttnKind::Mla && spec.kv_lora_rank > 0 {
        LayerKv::with_capacity_mla_dtype(kv_cap, spec.kv_lora_rank as usize, dtype)
    } else {
        let head_dim =
            (manifest.hidden_dim / manifest.effective_num_heads(layer)) as usize;
        let kv_dim = manifest.effective_num_kv_heads(layer) as usize * head_dim;
        LayerKv::with_capacity_dtype(kv_cap, kv_dim, dtype)
    }
}

pub struct MemoryTensorSource {
    pub tensors: alloc::collections::BTreeMap<String, Vec<f32>>,
}

impl TensorSource for MemoryTensorSource {
    fn load_f32(&mut self, name: &str, out: &mut [f32]) -> Result<(), ()> {
        let data = self.tensors.get(name).ok_or(())?;
        if data.len() != out.len() {
            return Err(());
        }
        out.copy_from_slice(data);
        Ok(())
    }

    fn load_f32_range(&mut self, name: &str, elem_off: usize, out: &mut [f32]) -> Result<(), ()> {
        let data = self.tensors.get(name).ok_or(())?;
        let end = elem_off.checked_add(out.len()).ok_or(())?;
        if end > data.len() {
            return Err(());
        }
        out.copy_from_slice(&data[elem_off..end]);
        Ok(())
    }

    fn tensor_view(&mut self, name: &str) -> Result<TensorView<'_>, ()> {
        let data = self.tensors.get(name).ok_or(())?;
        let bytes = unsafe {
            core::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 4)
        };
        Ok(TensorView {
            bytes,
            dtype: sosomodel::layout::DTYPE_F32,
            elems: data.len(),
        })
    }
}

pub struct Runtime {
    pub manifest: Manifest,
    pub index: TensorIndex,
    pub tiers: TierManager,
    pub hidden: Vec<f32>,
    pub kv: Vec<LayerKv>,
    pub pos: usize,
    pub backend: Backend,
    scratch: LayerScratch,
    logits_buf: Vec<f32>,
    has_gate: bool,
    has_lm_head: bool,
    has_output_norm: bool,
    pub planner: Option<ResourcePlanner>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Cpu,
    Gpu,
    Auto,
}

impl Runtime {
    pub fn new(manifest: Manifest, index: TensorIndex, ram_budget: usize, vram_budget: usize) -> Self {
        let h = manifest.hidden_dim as usize;
        let vocab = manifest.vocab_size as usize;
        let n = manifest.num_layers as usize;
        let scratch = LayerScratch::new(&manifest);
        let has_gate = index.find("L00.ffn_gate").is_some();
        let has_lm_head = index.find("lm_head").is_some();
        let has_output_norm = index.find("output_norm").is_some();
        let head_dim = (manifest.hidden_dim / manifest.num_heads) as usize;
        let _kv_dim = manifest.num_kv_heads as usize * head_dim;
        let kv_cap = (manifest.max_seq as usize).min(256).max(32);
        let manifest_for_kv = manifest.clone();
        Self {
            manifest,
            index,
            tiers: TierManager::new(ram_budget, vram_budget),
            hidden: vec![0.0f32; h],
            kv: (0..n)
                .map(|l| {
                    make_layer_kv(
                        &manifest_for_kv,
                        l as u32,
                        kv_cap,
                        crate::kv::KvDtype::F16,
                    )
                })
                .collect(),
            pos: 0,
            backend: Backend::Auto,
            scratch,
            logits_buf: vec![0.0f32; vocab],
            has_gate,
            has_lm_head,
            has_output_norm,
            planner: None,
        }
    }

    pub fn set_planner(&mut self, planner: ResourcePlanner) {
        // Antes del primer token: alinear dtype KV (KIVI-lite) con el planner.
        if self.pos == 0 {
            let dtype = planner.kv_dtype();
            let kv_cap = planner.kv_window_tokens().min(256).max(32);
            let manifest = self.manifest.clone();
            self.kv = (0..self.manifest.num_layers as usize)
                .map(|l| make_layer_kv(&manifest, l as u32, kv_cap, dtype))
                .collect();
        }
        self.planner = Some(planner);
    }

    /// Comprueba que las shapes del index casan con lo que espera el ejecutor
    /// (convención row-major `[filas, columnas]` = `[out_dim, in_dim]`).
    pub fn validate_shapes(&self) -> Result<(), ()> {
        self.manifest.supported_by_runtime().map_err(|_| ())?;
        self.validate_shapes_for_role(PipelineRole::Full, 0, self.manifest.num_layers)
    }

    /// Valida solo los tensores necesarios para un rol/rango de capas.
    pub fn validate_shapes_for_role(
        &self,
        role: PipelineRole,
        layer_start: u32,
        layer_end: u32,
    ) -> Result<(), ()> {
        let h = self.manifest.hidden_dim;
        let vocab = self.manifest.vocab_size;

        let check = |name: &str, want: &[u32], required: bool| -> Result<(), ()> {
            match self.index.find(name) {
                Some(e) => {
                    if e.shape == want {
                        Ok(())
                    } else {
                        Err(())
                    }
                }
                None if required => Err(()),
                None => Ok(()),
            }
        };

        let needs_embed = matches!(role, PipelineRole::Head | PipelineRole::Full);
        let needs_logits = matches!(role, PipelineRole::Tail | PipelineRole::Full);
        check("embed", &[vocab, h], needs_embed)?;
        check("lm_head", &[vocab, h], false)?;
        check("output_norm", &[h], needs_logits && self.has_output_norm)?;
        for layer in layer_start..layer_end {
            let p = alloc::format!("L{layer:02}");
            let spec = self.manifest.layer(layer).cloned().unwrap_or_default();
            let layer_heads = self.manifest.effective_num_heads(layer);
            let layer_kv_heads = self.manifest.effective_num_kv_heads(layer);
            let head_dim = h / layer_heads;
            let kv_dim = layer_kv_heads * head_dim;
            let ffn = self.manifest.effective_ffn_dim(layer);
            let moe_ffn = self.manifest.effective_moe_ffn_dim(layer);
            let is_moe = self.manifest.layer_is_moe(layer)
                || spec.ffn_kind == sosomodel::FfnKind::LatentMoe;
            let n_exp = self.manifest.effective_num_experts(layer);
            let n_shared = spec.num_shared_experts;
            check(&alloc::format!("{p}.attn_norm"), &[h], true)?;
            if spec.attn_kind == sosomodel::AttnKind::Mla {
                let q_rank = spec.q_lora_rank;
                let kv_rank = spec.kv_lora_rank;
                check(&alloc::format!("{p}.attn_q_down"), &[q_rank, h], true)?;
                check(&alloc::format!("{p}.attn_q_up"), &[h, q_rank], true)?;
                check(&alloc::format!("{p}.attn_kv_down"), &[kv_rank, h], true)?;
                check(&alloc::format!("{p}.attn_k_up"), &[kv_dim, kv_rank], true)?;
                check(&alloc::format!("{p}.attn_v_up"), &[kv_dim, kv_rank], true)?;
                check(&alloc::format!("{p}.attn_output"), &[h, h], true)?;
            } else {
                check(&alloc::format!("{p}.attn_q"), &[h, h], true)?;
                check(&alloc::format!("{p}.attn_k"), &[kv_dim, h], true)?;
                check(&alloc::format!("{p}.attn_v"), &[kv_dim, h], true)?;
                check(&alloc::format!("{p}.attn_output"), &[h, h], true)?;
            }
            check(&alloc::format!("{p}.ffn_norm"), &[h], true)?;
            if spec.ffn_kind == sosomodel::FfnKind::LatentMoe {
                let latent = moe_ffn;
                check(&alloc::format!("{p}.ffn_latent_in"), &[latent, h], true)?;
                check(&alloc::format!("{p}.ffn_latent_out"), &[h, latent], true)?;
                check(&alloc::format!("{p}.ffn_gate_inp"), &[n_exp, latent], true)?;
                for e in 0..n_exp {
                    let ep = alloc::format!("{p}.E{e:02}");
                    check(&alloc::format!("{ep}.ffn_gate"), &[latent, latent], true)?;
                    check(&alloc::format!("{ep}.ffn_up"), &[latent, latent], true)?;
                    check(&alloc::format!("{ep}.ffn_down"), &[latent, latent], true)?;
                }
            } else if is_moe {
                check(
                    &alloc::format!("{p}.ffn_gate_inp"),
                    &[n_exp, h],
                    true,
                )?;
                for e in 0..n_exp {
                    let ep = alloc::format!("{p}.E{e:02}");
                    check(&alloc::format!("{ep}.ffn_gate"), &[moe_ffn, h], true)?;
                    check(&alloc::format!("{ep}.ffn_up"), &[moe_ffn, h], true)?;
                    check(&alloc::format!("{ep}.ffn_down"), &[h, moe_ffn], true)?;
                }
                for s in 0..n_shared {
                    let sp = alloc::format!("{p}.S{s:02}");
                    check(&alloc::format!("{sp}.ffn_gate"), &[moe_ffn, h], true)?;
                    check(&alloc::format!("{sp}.ffn_up"), &[moe_ffn, h], true)?;
                    check(&alloc::format!("{sp}.ffn_down"), &[h, moe_ffn], true)?;
                }
            } else {
                check(&alloc::format!("{p}.ffn_up"), &[ffn, h], true)?;
                check(&alloc::format!("{p}.ffn_down"), &[h, ffn], true)?;
                check(&alloc::format!("{p}.ffn_gate"), &[ffn, h], self.has_gate)?;
            }
        }
        Ok(())
    }

    pub fn set_backend(&mut self, backend: Backend) {
        self.backend = backend;
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn reset_sequence(&mut self) {
        self.pos = 0;
        for layer in &mut self.kv {
            layer.reset();
        }
        if let Some(pl) = self.planner.as_mut() {
            pl.reset_moe_hints();
        }
    }

    pub fn embed_token(&mut self, token: u32, source: &mut impl TensorSource) -> Result<(), ()> {
        let h = self.manifest.hidden_dim as usize;
        if token >= self.manifest.vocab_size {
            return Err(());
        }
        source.load_f32_range("embed", token as usize * h, &mut self.hidden)
    }

    /// Prefetch de la fila de embed (solapa I/O con el forward del token actual).
    pub fn prefetch_embed(&mut self, token: u32, source: &mut impl TensorSource) {
        let h = self.manifest.hidden_dim as usize;
        if token < self.manifest.vocab_size {
            source.prefetch_embed_row(token, h);
        }
    }

    /// Prefill del prompt: prefetch del siguiente embed mientras se calcula
    /// el forward del token actual.
    pub fn prefill_prompt(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        clock_ms: Option<fn() -> u64>,
    ) -> Result<(), ()> {
        for (i, &tok) in prompt.iter().enumerate() {
            if let Some(&next) = prompt.get(i + 1) {
                self.prefetch_embed(next, source);
            }
            self.embed_token(tok, source)?;
            if let Some(c) = clock_ms {
                let _ = self.forward_step_timed(source, parallel, gpu, c)?;
            } else {
                self.forward_step_par(source, parallel, gpu)?;
            }
        }
        Ok(())
    }

    pub fn forward_step(&mut self, source: &mut impl TensorSource) -> Result<(), ()> {
        self.forward_step_par(source, None, &mut None)
    }

    pub fn forward_step_par(
        &mut self,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    ) -> Result<(), ()> {
        self.forward_layers_range_clock(
            0,
            self.manifest.num_layers,
            source,
            parallel,
            gpu,
            None,
        )?;
        self.advance_pos();
        self.slide_kv_if_needed();
        Ok(())
    }

    /// Como `forward_step_par` con reloj y replanificación al final del token.
    pub fn forward_step_timed(
        &mut self,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        clock_ms: fn() -> u64,
    ) -> Result<bool, ()> {
        self.forward_layers_range_clock(
            0,
            self.manifest.num_layers,
            source,
            parallel,
            gpu,
            Some(clock_ms),
        )?;
        self.advance_pos();
        self.slide_kv_if_needed();
        let replanned = self
            .planner
            .as_mut()
            .map(|p| p.on_token_complete(&self.manifest, &self.index))
            .unwrap_or(false);
        Ok(replanned)
    }

    /// Ejecuta un rango de capas en la posición actual sin avanzar `pos`.
    pub fn forward_layers_range(
        &mut self,
        layer_start: u32,
        layer_end: u32,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    ) -> Result<(), ()> {
        self.forward_layers_range_clock(
            layer_start,
            layer_end,
            source,
            parallel,
            gpu,
            None,
        )
    }

    /// Como `forward_layers_range` pero cronometra capas si se pasa `clock_ms`.
    pub fn forward_layers_range_clock(
        &mut self,
        layer_start: u32,
        layer_end: u32,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        clock_ms: Option<fn() -> u64>,
    ) -> Result<(), ()> {
        // Con planner: la ventana KV deslizante permite superar max_seq.
        if self.planner.is_none() && self.pos >= self.manifest.max_seq as usize {
            return Err(());
        }
        if layer_start > layer_end || layer_end > self.manifest.num_layers {
            return Err(());
        }
        let base_gpu = matches!(self.backend, Backend::Gpu | Backend::Auto)
            && gpu.as_ref().is_some_and(|g| g.available());
        let exec = LayerExecutor {
            manifest: &self.manifest,
            has_gate: self.has_gate,
            parallel,
        };
        // Kick capa inicial (prefetch adelantado antes del bucle).
        if let Some(pf) = self.manifest.prefetch.get(layer_start as usize) {
            source.kick_prefetch_shards(&pf.shards);
            if let Some(pl) = self.planner.as_mut() {
                pl.note_prefetch();
            }
        }
        for layer in layer_start..layer_end {
            // Esperar capa N (prefetchada mientras se calculó N-1).
            let wait0 = clock_ms.map(|c| c());
            source.wait_prefetch();
            if let (Some(c), Some(t0)) = (clock_ms, wait0) {
                let ms = c().saturating_sub(t0);
                if let Some(pl) = self.planner.as_mut() {
                    pl.note_stage_wait_ms(ms);
                }
            }
            // Kick capa N+1 antes del cómputo (solape I/O ∥ matvec/attn).
            if let Some(next) = self.manifest.prefetch.get((layer + 1) as usize) {
                self.tiers.schedule_prefetch(&next.shards);
                self.tiers.kick_pending(source);
                if let Some(pl) = self.planner.as_mut() {
                    pl.note_prefetch();
                }
            }
            let use_gpu = base_gpu
                && self
                    .planner
                    .as_ref()
                    .map(|p| p.use_gpu_for_layer(layer))
                    .unwrap_or(true);
            let dest = if use_gpu {
                ExecDest::Gpu
            } else if self
                .planner
                .as_ref()
                .is_some_and(|p| p.layer_dest(layer) == ExecDest::Remote)
            {
                ExecDest::Remote
            } else {
                ExecDest::Cpu
            };
            if let Some(pl) = self.planner.as_mut() {
                pl.note_trunk_layer(layer, &self.index);
            }
            let t0 = clock_ms.map(|c| c());
            let timing = exec.forward_layer(
                layer,
                self.pos,
                &mut self.hidden,
                &mut self.scratch,
                &mut self.kv[layer as usize],
                source,
                gpu,
                use_gpu,
                self.planner.as_mut(),
                Some(&self.index),
                clock_ms,
            )?;
            if let (Some(c), Some(pl)) = (clock_ms, self.planner.as_mut()) {
                let ms = c().saturating_sub(t0.unwrap_or(0));
                pl.observe_layer(layer, dest, ms);
                pl.observe_hotpath(timing.matvec_ms, timing.attn_ms);
            }
            // Liberar shards fuera del working set (streaming FlexGen).
            let keep = self
                .planner
                .as_ref()
                .map(|pl| pl.keep_all_shards_after(layer, layer_end, &self.manifest));
            if let Some(keep) = keep {
                if !keep.is_empty() {
                    let tr0 = clock_ms.map(|c| c());
                    source.release_shards_except(&keep);
                    let ms = match (clock_ms, tr0) {
                        (Some(c), Some(t)) => c().saturating_sub(t),
                        _ => 0,
                    };
                    if let Some(pl) = self.planner.as_mut() {
                        pl.note_shard_release();
                        pl.note_release_ms(ms);
                    }
                }
            }
        }
        Ok(())
    }

    /// StreamingLLM / H2O: recorta KV a la ventana presupuestada.
    pub fn slide_kv_if_needed(&mut self) {
        let (keep, sink, use_h2o) = match self.planner.as_ref() {
            Some(pl) => (pl.kv_window_tokens(), pl.sink_tokens(), pl.use_h2o()),
            None => return,
        };
        let recent = keep.saturating_sub(sink) / 2;
        let mut slid = false;
        for (layer_idx, kv) in self.kv.iter_mut().enumerate() {
            let storage = kv_storage_dim(&self.manifest, layer_idx as u32);
            if kv.tokens(storage) > keep {
                kv.slide_window_h2o(keep, sink, recent, storage, use_h2o);
                slid = true;
            }
        }
        if slid {
            if let Some(pl) = self.planner.as_mut() {
                pl.note_kv_slide();
            }
        }
    }

    pub fn advance_pos(&mut self) {
        self.pos += 1;
    }

    pub fn set_hidden(&mut self, hidden: &[f32]) -> Result<(), ()> {
        if hidden.len() != self.hidden.len() {
            return Err(());
        }
        self.hidden.copy_from_slice(hidden);
        Ok(())
    }

    pub fn hidden_slice(&self) -> &[f32] {
        &self.hidden
    }

    /// Calcula los logits directamente sobre la vista zero-copy de lm_head
    /// (o embed con weight-tying): un solo matvec sin copiar la matriz.
    pub fn logits(&mut self, source: &mut impl TensorSource) -> Result<&[f32], ()> {
        let h = self.manifest.hidden_dim as usize;
        let vocab = self.manifest.vocab_size as usize;
        let name = if self.has_lm_head { "lm_head" } else { "embed" };

        // norma final (si el modelo la trae) sobre una copia del hidden
        self.scratch.attn_out.copy_from_slice(&self.hidden);
        if self.has_output_norm {
            source.load_f32("output_norm", &mut self.scratch.norm_w)?;
            rmsnorm(&mut self.scratch.attn_out, &self.scratch.norm_w, self.manifest.rms_eps);
        }

        let view = source.tensor_view(name)?;
        matvec_view_par(
            &view,
            vocab,
            h,
            &self.scratch.attn_out,
            &mut self.logits_buf,
            &Sequential,
        )?;
        Ok(&self.logits_buf)
    }

    pub fn logits_par(
        &mut self,
        source: &mut impl TensorSource,
        parallel: Option<&dyn RowParallel>,
    ) -> Result<&[f32], ()> {
        let h = self.manifest.hidden_dim as usize;
        let vocab = self.manifest.vocab_size as usize;
        let name = if self.has_lm_head { "lm_head" } else { "embed" };
        self.scratch.attn_out.copy_from_slice(&self.hidden);
        if self.has_output_norm {
            source.load_f32("output_norm", &mut self.scratch.norm_w)?;
            rmsnorm(&mut self.scratch.attn_out, &self.scratch.norm_w, self.manifest.rms_eps);
        }
        let view = source.tensor_view(name)?;
        let seq = Sequential;
        let par: &dyn RowParallel = parallel.unwrap_or(&seq);
        matvec_view_par(
            &view,
            vocab,
            h,
            &self.scratch.attn_out,
            &mut self.logits_buf,
            par,
        )?;
        Ok(&self.logits_buf)
    }

    pub fn argmax(logits: &[f32]) -> u32 {
        logits
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal))
            .map(|(i, _)| i as u32)
            .unwrap_or(0)
    }

    /// Genera hasta `max_new` tokens muestreando con `sampler`, llamando a
    /// `on_token` según se emite cada uno (streaming). Devuelve la secuencia
    /// completa (prompt + generados).
    pub fn generate_stream(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        max_new: usize,
        eos: Option<u32>,
        sampler: &mut crate::sample::Sampler,
        on_token: impl FnMut(u32),
    ) -> Result<Vec<u32>, ()> {
        self.generate_stream_par(source, prompt, max_new, eos, sampler, on_token, None, &mut None)
    }

    /// Como `generate_stream` con matvec paralelo opcional.
    pub fn generate_stream_par(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        max_new: usize,
        eos: Option<u32>,
        sampler: &mut crate::sample::Sampler,
        mut on_token: impl FnMut(u32),
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
    ) -> Result<Vec<u32>, ()> {
        if prompt.is_empty() {
            return Err(());
        }
        self.reset_sequence();
        let mut tokens: Vec<u32> = prompt.to_vec();
        self.prefill_prompt(source, prompt, parallel, gpu, None)?;
        let greedy = sampler.temp <= 0.0;
        let mut remaining = max_new;
        while remaining > 0 {
            if self.planner.is_none() && self.pos >= self.manifest.max_seq as usize {
                break;
            }
            let logits = self.logits_par(source, parallel)?;
            let mut next = sampler.sample(logits);
            if eos == Some(next) {
                break;
            }
            tokens.push(next);
            on_token(next);
            remaining -= 1;
            let drafts = if greedy {
                crate::attn::prompt_lookup_draft(&tokens, remaining.min(8))
            } else {
                alloc::vec::Vec::new()
            };
            if let Some(&d0) = drafts.first() {
                self.prefetch_embed(d0, source);
            }
            self.embed_token(next, source)?;
            self.forward_step_par(source, parallel, gpu)?;
            for (di, &draft) in drafts.iter().enumerate() {
                if remaining == 0 {
                    break;
                }
                let logits = self.logits_par(source, parallel)?;
                next = Self::argmax(logits);
                if next != draft || eos == Some(next) {
                    break;
                }
                tokens.push(next);
                on_token(next);
                remaining -= 1;
                if let Some(&nxt) = drafts.get(di + 1) {
                    self.prefetch_embed(nxt, source);
                }
                self.embed_token(next, source)?;
                self.forward_step_par(source, parallel, gpu)?;
            }
        }
        Ok(tokens)
    }

    /// Como `generate_stream_par` con cronometraje de capas y replanificación.
    pub fn generate_stream_planned(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        max_new: usize,
        eos: Option<u32>,
        sampler: &mut crate::sample::Sampler,
        mut on_token: impl FnMut(u32),
        parallel: Option<&dyn RowParallel>,
        gpu: &mut Option<&mut dyn crate::gpu::GpuDispatch>,
        clock_ms: fn() -> u64,
        mut refresh_mem: impl FnMut() -> crate::plan::MemSnapshot,
    ) -> Result<Vec<u32>, ()> {
        if prompt.is_empty() {
            return Err(());
        }
        self.reset_sequence();
        if let Some(pl) = self.planner.as_mut() {
            pl.refresh_mem(refresh_mem());
        }
        let mut tokens: Vec<u32> = prompt.to_vec();
        self.prefill_prompt(source, prompt, parallel, gpu, Some(clock_ms))?;
        let greedy = sampler.temp <= 0.0;
        let mut remaining = max_new;
        while remaining > 0 {
            if let Some(pl) = self.planner.as_mut() {
                pl.begin_token();
            }
            let logits = self.logits_par(source, parallel)?;
            let mut next = sampler.sample(logits);
            if eos == Some(next) {
                break;
            }
            tokens.push(next);
            on_token(next);
            remaining -= 1;
            let drafts = if greedy {
                let (max_d, min_n, max_n, hint) = self
                    .planner
                    .as_ref()
                    .map(|p| p.pld_params())
                    .unwrap_or((8, 2, 7, 4));
                crate::attn::prompt_lookup_draft_hinted(
                    &tokens,
                    remaining.min(max_d),
                    min_n,
                    max_n,
                    hint,
                )
            } else {
                alloc::vec::Vec::new()
            };
            if let Some(&d0) = drafts.first() {
                self.prefetch_embed(d0, source);
            }
            self.embed_token(next, source)?;
            if self.forward_step_timed(source, parallel, gpu, clock_ms)? {
                if let Some(pl) = self.planner.as_mut() {
                    pl.refresh_mem(refresh_mem());
                    pl.recompute_streaming_budgets(&self.manifest, &self.index);
                }
            }
            if drafts.is_empty() {
                continue;
            }
            let offered = drafts.len();
            if let Some(pl) = self.planner.as_mut() {
                pl.note_pld_attempt();
            }
            let mut accepted = 0u32;
            for (di, &draft) in drafts.iter().enumerate() {
                if remaining == 0 {
                    break;
                }
                let logits = self.logits_par(source, parallel)?;
                next = Self::argmax(logits);
                if next != draft || eos == Some(next) {
                    break;
                }
                tokens.push(next);
                on_token(next);
                remaining -= 1;
                accepted += 1;
                if let Some(&nxt) = drafts.get(di + 1) {
                    self.prefetch_embed(nxt, source);
                }
                self.embed_token(next, source)?;
                let _ = self.forward_step_timed(source, parallel, gpu, clock_ms)?;
            }
            if let Some(pl) = self.planner.as_mut() {
                if accepted > 0 {
                    pl.note_pld_accepted(accepted);
                }
                pl.tune_pld(offered, accepted);
            }
        }
        Ok(tokens)
    }

    /// Decode greedy sin streaming (tests y usos simples).
    pub fn generate(
        &mut self,
        source: &mut impl TensorSource,
        prompt: &[u32],
        max_new: usize,
        eos: Option<u32>,
    ) -> Result<Vec<u32>, ()> {
        let mut sampler = crate::sample::Sampler::greedy();
        self.generate_stream(source, prompt, max_new, eos, &mut sampler, |_| {})
    }

    pub fn load_shard_to_ram(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), ()> {
        use crate::tier::Tier;
        self.tiers.promote(name, bytes, Tier::Ram)
    }
}
