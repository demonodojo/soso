//! Runtime de inferencia: bucle de generación de tokens.

use crate::gemm::rmsnorm;
use crate::layer::{matvec_view, LayerExecutor, LayerKv, LayerScratch, TensorSource, TensorView};
use crate::tier::TierManager;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;

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
        Self {
            manifest,
            index,
            tiers: TierManager::new(ram_budget, vram_budget),
            hidden: vec![0.0f32; h],
            kv: (0..n).map(|_| LayerKv::new()).collect(),
            pos: 0,
            backend: Backend::Auto,
            scratch,
            logits_buf: vec![0.0f32; vocab],
            has_gate,
            has_lm_head,
            has_output_norm,
        }
    }

    /// Comprueba que las shapes del index casan con lo que espera el ejecutor
    /// (convención row-major `[filas, columnas]` = `[out_dim, in_dim]`).
    pub fn validate_shapes(&self) -> Result<(), ()> {
        let h = self.manifest.hidden_dim;
        let vocab = self.manifest.vocab_size;
        let ffn = self.manifest.ffn_dim;
        let heads = self.manifest.num_heads;
        let head_dim = h / heads;
        let kv_dim = self.manifest.num_kv_heads * head_dim;

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

        check("embed", &[vocab, h], true)?;
        check("lm_head", &[vocab, h], false)?;
        check("output_norm", &[h], false)?;
        for layer in 0..self.manifest.num_layers {
            let p = alloc::format!("L{layer:02}");
            check(&alloc::format!("{p}.attn_norm"), &[h], true)?;
            check(&alloc::format!("{p}.attn_q"), &[h, h], true)?;
            check(&alloc::format!("{p}.attn_k"), &[kv_dim, h], true)?;
            check(&alloc::format!("{p}.attn_v"), &[kv_dim, h], true)?;
            check(&alloc::format!("{p}.attn_output"), &[h, h], true)?;
            check(&alloc::format!("{p}.ffn_norm"), &[h], true)?;
            check(&alloc::format!("{p}.ffn_up"), &[ffn, h], true)?;
            check(&alloc::format!("{p}.ffn_down"), &[h, ffn], true)?;
            // Si hay gate en la capa 0, debe estar en todas.
            check(&alloc::format!("{p}.ffn_gate"), &[ffn, h], self.has_gate)?;
        }
        Ok(())
    }

    pub fn reset_sequence(&mut self) {
        self.pos = 0;
        for layer in &mut self.kv {
            layer.reset();
        }
    }

    pub fn embed_token(&mut self, token: u32, source: &mut impl TensorSource) -> Result<(), ()> {
        let h = self.manifest.hidden_dim as usize;
        if token >= self.manifest.vocab_size {
            return Err(());
        }
        source.load_f32_range("embed", token as usize * h, &mut self.hidden)
    }

    pub fn forward_step(&mut self, source: &mut impl TensorSource) -> Result<(), ()> {
        if self.pos >= self.manifest.max_seq as usize {
            return Err(());
        }
        let exec = LayerExecutor {
            manifest: &self.manifest,
            has_gate: self.has_gate,
        };
        for layer in 0..self.manifest.num_layers {
            if let Some(pf) = self.manifest.prefetch.get(layer as usize) {
                self.tiers.schedule_prefetch(&pf.shards);
            }
            exec.forward_layer(
                layer,
                self.pos,
                &mut self.hidden,
                &mut self.scratch,
                &mut self.kv[layer as usize],
                source,
            )?;
        }
        self.pos += 1;
        Ok(())
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
        matvec_view(&view, vocab, h, &self.scratch.attn_out, &mut self.logits_buf)?;
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
        mut on_token: impl FnMut(u32),
    ) -> Result<Vec<u32>, ()> {
        if prompt.is_empty() {
            return Err(());
        }
        self.reset_sequence();
        let mut tokens: Vec<u32> = prompt.to_vec();
        for &tok in prompt {
            self.embed_token(tok, source)?;
            self.forward_step(source)?;
        }
        for _ in 0..max_new {
            if self.pos >= self.manifest.max_seq as usize {
                break;
            }
            let logits = self.logits(source)?;
            let next = sampler.sample(logits);
            if eos == Some(next) {
                break;
            }
            tokens.push(next);
            on_token(next);
            self.embed_token(next, source)?;
            self.forward_step(source)?;
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
