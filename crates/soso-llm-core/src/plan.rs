//! Planificador consciente de recursos: presupuesto de memoria, latencia
//! por capa/destino y replanificación adaptativa.
//!
//! Optimizaciones inspiradas en papers recientes de inferencia:
//! - **LayerKV / FlexGen**: working set de pocas capas residentes + prefetch
//!   layer-ahead (solapar I/O con cómputo).
//! - **StreamingLLM**: ventana KV (sink + recientes) bajo presión de memoria.
//! - **PagedAttention (espíritu)**: capacidad KV pre-reservada; no crecer
//!   ciegamente hasta OOM.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;

const FRAME_BYTES: u64 = 4096;
const REPLAN_EVERY_TOKENS: u32 = 8;
const EWMA_ALPHA: f64 = 0.25;
/// Reserva el 30 % de frames libres+reclaimable para el sistema.
const MEM_RESERVE_PCT: u64 = 30;
/// Si la EWMA remota supera esto, degradar a CPU local.
const REMOTE_SLOW_MS: f64 = 500.0;
/// Capas de pesos a mantener mapeadas (actual + prefetch).
const DEFAULT_RESIDENT_LAYERS: u32 = 2;
/// Tokens sink (StreamingLLM) que nunca se evictan del KV.
const DEFAULT_SINK_TOKENS: usize = 4;

/// Instantánea de memoria (compatible con `soso_abi::MemInfo`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemSnapshot {
    pub total_frames: u64,
    pub free_frames: u64,
    pub reclaimable_frames: u64,
}

impl MemSnapshot {
    pub fn free_bytes(&self) -> u64 {
        self.free_frames.saturating_mul(FRAME_BYTES)
    }

    pub fn reclaimable_bytes(&self) -> u64 {
        self.reclaimable_frames.saturating_mul(FRAME_BYTES)
    }

    /// Bytes que el planner puede usar para pesos residentes (70 % de libre+reclaimable).
    pub fn weight_budget_bytes(&self) -> u64 {
        let usable = self.free_bytes().saturating_add(self.reclaimable_bytes());
        usable.saturating_mul(100 - MEM_RESERVE_PCT) / 100
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecDest {
    Cpu,
    Gpu,
    Remote,
}

#[derive(Clone, Debug)]
pub struct LayerPlan {
    pub layer: u32,
    pub dest: ExecDest,
    /// Proyecciones que pueden ir a VRAM en esta capa.
    pub gpu_tensors: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlannerStats {
    pub replans: u32,
    pub cpu_layers: u32,
    pub gpu_layers: u32,
    pub remote_layers: u32,
    pub tokens_observed: u32,
    pub avg_cpu_ms: f64,
    pub avg_gpu_ms: f64,
    pub avg_remote_ms: f64,
    pub weight_budget_bytes: u64,
    pub model_weight_bytes: u64,
    /// Capas de pesos en el working set (streaming).
    pub resident_layers: u32,
    /// Ventana máxima de tokens KV bajo presupuesto.
    pub kv_window_tokens: u32,
    pub shard_releases: u32,
    pub kv_slides: u32,
    pub prefeches: u32,
}

pub struct ResourcePlanner {
    pub layer_plans: Vec<LayerPlan>,
    layer_ms_cpu: Vec<f64>,
    layer_ms_gpu: Vec<f64>,
    layer_ms_remote: Vec<f64>,
    tokens_since_replan: u32,
    mem: MemSnapshot,
    weight_budget: u64,
    model_weight_bytes: u64,
    vram_free: u64,
    remote_available: bool,
    remote_degraded: bool,
    remote_rtt_ms: f64,
    /// Capas de pesos a mantener mapeadas a la vez.
    resident_layers: u32,
    /// Máximo de tokens en KV (todas las capas) bajo presupuesto.
    kv_window_tokens: usize,
    sink_tokens: usize,
    avg_layer_bytes: u64,
    kv_bytes_per_token: u64,
    stats: PlannerStats,
}

const GPU_PROJ: [&str; 6] = [
    "attn_q",
    "attn_k",
    "attn_v",
    "attn_output",
    "ffn_up",
    "ffn_down",
];

pub fn layer_tensor_prefix(layer: u32) -> String {
    format!("L{layer:02}.")
}

pub fn bytes_for_layer(layer: u32, index: &TensorIndex) -> u64 {
    let prefix = layer_tensor_prefix(layer);
    index
        .entries
        .iter()
        .filter(|e| e.name.starts_with(&prefix))
        .map(|e| e.byte_len)
        .sum()
}

pub fn total_model_weight_bytes(index: &TensorIndex) -> u64 {
    index.entries.iter().map(|e| e.byte_len).sum()
}

/// Bytes de KV f16 por token de secuencia (todas las capas).
pub fn kv_bytes_per_token(manifest: &Manifest) -> u64 {
    let head_dim = (manifest.hidden_dim / manifest.num_heads) as u64;
    let kv_dim = manifest.num_kv_heads as u64 * head_dim;
    // K + V, f16 = 2 bytes, × num_layers
    kv_dim * 2 * 2 * manifest.num_layers as u64
}

impl ResourcePlanner {
    pub fn new(
        manifest: &Manifest,
        index: &TensorIndex,
        mem: MemSnapshot,
        vram_free: u64,
        remote_available: bool,
    ) -> Self {
        let n = manifest.num_layers as usize;
        let model_weight_bytes = total_model_weight_bytes(index);
        let weight_budget = mem.weight_budget_bytes();
        let avg_layer = if n == 0 {
            0
        } else {
            (0..manifest.num_layers)
                .map(|l| bytes_for_layer(l, index))
                .sum::<u64>()
                / n as u64
        };
        let kv_bpt = kv_bytes_per_token(manifest);
        let mut planner = Self {
            layer_plans: Vec::with_capacity(n),
            layer_ms_cpu: vec![0.0; n],
            layer_ms_gpu: vec![0.0; n],
            layer_ms_remote: vec![0.0; n],
            tokens_since_replan: 0,
            mem,
            weight_budget,
            model_weight_bytes,
            vram_free,
            remote_available,
            remote_degraded: false,
            remote_rtt_ms: 0.0,
            resident_layers: DEFAULT_RESIDENT_LAYERS,
            kv_window_tokens: manifest.max_seq as usize,
            sink_tokens: DEFAULT_SINK_TOKENS,
            avg_layer_bytes: avg_layer,
            kv_bytes_per_token: kv_bpt,
            stats: PlannerStats {
                weight_budget_bytes: weight_budget,
                model_weight_bytes,
                ..Default::default()
            },
        };
        planner.recompute_streaming_budgets(manifest);
        planner.rebuild_plan(manifest, index);
        planner
    }

    pub fn refresh_mem(&mut self, mem: MemSnapshot) {
        self.mem = mem;
        self.weight_budget = mem.weight_budget_bytes();
        self.stats.weight_budget_bytes = self.weight_budget;
    }

    pub fn recompute_streaming_budgets(&mut self, manifest: &Manifest) {
        // LayerKV: cuantas capas caben en el presupuesto de pesos.
        let layers = if self.avg_layer_bytes == 0 {
            DEFAULT_RESIDENT_LAYERS
        } else {
            let fit = (self.weight_budget / self.avg_layer_bytes.max(1)).max(1) as u32;
            fit.min(DEFAULT_RESIDENT_LAYERS.max(1)).min(manifest.num_layers.max(1))
        };
        self.resident_layers = layers.max(1);

        // Mitad del presupuesto para KV (el resto son pesos streaming + scratch).
        let kv_budget = self.weight_budget / 2;
        let window = if self.kv_bytes_per_token == 0 {
            manifest.max_seq as usize
        } else {
            let w = (kv_budget / self.kv_bytes_per_token) as usize;
            w.max(self.sink_tokens + 8)
                .min(manifest.max_seq as usize)
        };
        self.kv_window_tokens = window;
        self.stats.resident_layers = self.resident_layers;
        self.stats.kv_window_tokens = self.kv_window_tokens as u32;
    }

    pub fn stats(&self) -> &PlannerStats {
        &self.stats
    }

    pub fn resident_layers(&self) -> u32 {
        self.resident_layers
    }

    pub fn kv_window_tokens(&self) -> usize {
        self.kv_window_tokens
    }

    pub fn sink_tokens(&self) -> usize {
        self.sink_tokens
    }

    pub fn note_prefetch(&mut self) {
        self.stats.prefeches = self.stats.prefeches.saturating_add(1);
    }

    pub fn note_shard_release(&mut self) {
        self.stats.shard_releases = self.stats.shard_releases.saturating_add(1);
    }

    pub fn note_kv_slide(&mut self) {
        self.stats.kv_slides = self.stats.kv_slides.saturating_add(1);
    }

    pub fn layer_dest(&self, layer: u32) -> ExecDest {
        self.layer_plans
            .get(layer as usize)
            .map(|p| p.dest)
            .unwrap_or(ExecDest::Cpu)
    }

    pub fn use_gpu_for_layer(&self, layer: u32) -> bool {
        matches!(self.layer_dest(layer), ExecDest::Gpu) && self.vram_free > 0
    }

    pub fn gpu_tensor_allowed(&self, layer: u32, tensor: &str) -> bool {
        self.layer_plans
            .get(layer as usize)
            .map(|p| p.gpu_tensors.iter().any(|t| tensor.ends_with(t)))
            .unwrap_or(false)
    }

    /// Shards a mantener tras terminar `layer` (working set = capas recientes).
    pub fn keep_shards_after(
        &self,
        layer: u32,
        layer_end: u32,
        manifest: &Manifest,
    ) -> Vec<String> {
        let start = layer.saturating_sub(self.resident_layers.saturating_sub(1));
        let end = (layer + 1).min(layer_end).min(manifest.num_layers);
        let mut keep = Vec::new();
        for l in start..end {
            if let Some(pf) = manifest.prefetch.get(l as usize) {
                keep.extend(pf.shards.iter().cloned());
            }
        }
        // Prefetch layer-ahead
        if end < layer_end.min(manifest.num_layers) {
            if let Some(pf) = manifest.prefetch.get(end as usize) {
                keep.extend(pf.shards.iter().cloned());
            }
        }
        keep
    }

    pub fn observe_layer(&mut self, layer: u32, dest: ExecDest, ms: u64) {
        let i = layer as usize;
        if i >= self.layer_ms_cpu.len() {
            return;
        }
        let msf = ms.max(1) as f64;
        match dest {
            ExecDest::Cpu => ewma(&mut self.layer_ms_cpu[i], msf),
            ExecDest::Gpu => ewma(&mut self.layer_ms_gpu[i], msf),
            ExecDest::Remote => ewma(&mut self.layer_ms_remote[i], msf),
        }
        self.stats.tokens_observed += 1;
        self.recompute_avg_stats();
    }

    pub fn observe_remote_rtt(&mut self, ms: u64) {
        ewma(&mut self.remote_rtt_ms, ms.max(1) as f64);
        if self.remote_rtt_ms > REMOTE_SLOW_MS {
            self.remote_degraded = true;
        }
    }

    pub fn on_token_complete(&mut self, manifest: &Manifest, index: &TensorIndex) -> bool {
        self.tokens_since_replan += 1;
        if self.tokens_since_replan < REPLAN_EVERY_TOKENS {
            return false;
        }
        self.tokens_since_replan = 0;
        self.recompute_streaming_budgets(manifest);
        self.rebuild_plan(manifest, index);
        self.stats.replans += 1;
        true
    }

    pub fn set_vram_free(&mut self, bytes: u64) {
        self.vram_free = bytes;
    }

    fn recompute_avg_stats(&mut self) {
        self.stats.avg_cpu_ms = avg_nonzero(&self.layer_ms_cpu);
        self.stats.avg_gpu_ms = avg_nonzero(&self.layer_ms_gpu);
        self.stats.avg_remote_ms = avg_nonzero(&self.layer_ms_remote);
    }

    fn rebuild_plan(&mut self, manifest: &Manifest, index: &TensorIndex) {
        self.layer_plans.clear();
        let mut vram_left = self.vram_free;
        let mut cpu_layers = 0u32;
        let mut gpu_layers = 0u32;
        let mut remote_layers = 0u32;

        // Con streaming, el presupuesto efectivo es capas residentes × tamaño.
        let stream_budget = self
            .avg_layer_bytes
            .saturating_mul(self.resident_layers as u64)
            .max(self.weight_budget.min(self.avg_layer_bytes.saturating_mul(2)));

        for layer in 0..manifest.num_layers {
            let lb = bytes_for_layer(layer, index);
            let dest = choose_dest(
                lb,
                stream_budget,
                self.model_weight_bytes,
                vram_left,
                self.remote_available,
                self.remote_degraded,
                self.layer_ms_cpu.get(layer as usize).copied().unwrap_or(0.0),
                self.layer_ms_remote.get(layer as usize).copied().unwrap_or(0.0),
                self.remote_rtt_ms,
            );
            let mut gpu_tensors = Vec::new();
            if dest == ExecDest::Gpu {
                let prefix = layer_tensor_prefix(layer);
                for suffix in GPU_PROJ {
                    let name = format!("{prefix}{suffix}");
                    if let Some(e) = index.find(&name) {
                        let need = e.byte_len.saturating_mul(8);
                        if vram_left >= need {
                            gpu_tensors.push(String::from(suffix));
                            vram_left = vram_left.saturating_sub(need);
                        }
                    }
                }
                if gpu_tensors.is_empty() {
                    cpu_layers += 1;
                    self.layer_plans.push(LayerPlan {
                        layer,
                        dest: ExecDest::Cpu,
                        gpu_tensors: Vec::new(),
                    });
                    continue;
                }
                gpu_layers += 1;
            } else if dest == ExecDest::Remote {
                remote_layers += 1;
            } else {
                cpu_layers += 1;
            }
            self.layer_plans.push(LayerPlan {
                layer,
                dest,
                gpu_tensors,
            });
        }
        self.stats.cpu_layers = cpu_layers;
        self.stats.gpu_layers = gpu_layers;
        self.stats.remote_layers = remote_layers;
        self.stats.resident_layers = self.resident_layers;
        self.stats.kv_window_tokens = self.kv_window_tokens as u32;
    }
}

fn ewma(slot: &mut f64, sample: f64) {
    if *slot <= 0.0 {
        *slot = sample;
    } else {
        *slot = EWMA_ALPHA * sample + (1.0 - EWMA_ALPHA) * *slot;
    }
}

fn avg_nonzero(v: &[f64]) -> f64 {
    let mut sum = 0.0;
    let mut n = 0u32;
    for &x in v {
        if x > 0.0 {
            sum += x;
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum / n as f64
    }
}

fn choose_dest(
    layer_bytes: u64,
    weight_budget: u64,
    model_bytes: u64,
    vram_free: u64,
    remote_available: bool,
    remote_degraded: bool,
    cpu_ms: f64,
    remote_layer_ms: f64,
    remote_rtt_ms: f64,
) -> ExecDest {
    if remote_available && !remote_degraded {
        if remote_layer_ms > 0.0 && cpu_ms > 0.0 && remote_layer_ms + remote_rtt_ms < cpu_ms {
            return ExecDest::Remote;
        }
        if model_bytes > weight_budget.saturating_mul(2) && remote_rtt_ms < REMOTE_SLOW_MS {
            return ExecDest::Remote;
        }
    }
    if vram_free > layer_bytes.saturating_mul(4) && layer_bytes > 256 * 1024 {
        return ExecDest::Gpu;
    }
    ExecDest::Cpu
}

#[cfg(test)]
mod tests {
    use super::*;
    use sosomodel::manifest::Manifest;

    #[test]
    fn weight_budget_reserves_thirty_percent() {
        let mem = MemSnapshot {
            total_frames: 10_000,
            free_frames: 1000,
            reclaimable_frames: 500,
        };
        let budget = mem.weight_budget_bytes();
        let total = mem.free_bytes() + mem.reclaimable_bytes();
        assert_eq!(budget, total * 70 / 100);
    }

    #[test]
    fn always_cpu_when_no_gpu_no_remote() {
        let manifest = Manifest::tiny("t");
        let index = TensorIndex::default();
        let mem = MemSnapshot {
            total_frames: 1000,
            free_frames: 100,
            reclaimable_frames: 0,
        };
        let planner = ResourcePlanner::new(&manifest, &index, mem, 0, false);
        for p in &planner.layer_plans {
            assert_eq!(p.dest, ExecDest::Cpu);
        }
    }

    #[test]
    fn replan_increments_counter() {
        let manifest = Manifest::tiny("t");
        let index = TensorIndex::default();
        let mem = MemSnapshot::default();
        let mut planner = ResourcePlanner::new(&manifest, &index, mem, 0, false);
        planner.tokens_since_replan = REPLAN_EVERY_TOKENS - 1;
        assert!(planner.on_token_complete(&manifest, &index));
        assert_eq!(planner.stats().replans, 1);
    }

    #[test]
    fn ewma_tracks_latency() {
        let mut v = 0.0;
        ewma(&mut v, 100.0);
        ewma(&mut v, 200.0);
        assert!(v > 100.0 && v < 200.0);
    }

    #[test]
    fn remote_degraded_on_slow_rtt() {
        let manifest = Manifest::tiny("t");
        let index = TensorIndex::default();
        let mem = MemSnapshot {
            free_frames: 1000,
            ..Default::default()
        };
        let mut planner = ResourcePlanner::new(&manifest, &index, mem, 0, true);
        planner.observe_remote_rtt(1000);
        assert!(planner.remote_degraded);
        planner.rebuild_plan(&manifest, &index);
        assert!(planner.layer_plans.iter().all(|p| p.dest != ExecDest::Remote));
    }

    #[test]
    fn streaming_budgets_under_pressure() {
        let manifest = Manifest::tiny("t");
        let index = TensorIndex::default();
        // Muy poca RAM: el working set debe ser 1 capa y la ventana KV acotada.
        let mem = MemSnapshot {
            total_frames: 256,
            free_frames: 64,
            reclaimable_frames: 0,
        };
        let planner = ResourcePlanner::new(&manifest, &index, mem, 0, false);
        assert!(planner.resident_layers() >= 1);
        assert!(planner.kv_window_tokens() >= DEFAULT_SINK_TOKENS + 8);
    }

    #[test]
    fn kv_slide_preserves_sink_and_recent() {
        use crate::layer::LayerKv;
        let kv_dim = 4;
        let mut kv = LayerKv::new();
        for t in 0..20u16 {
            let k = [t as f32; 4];
            let v = [(t + 100) as f32; 4];
            kv.append_f16(&k, &v);
        }
        kv.slide_window(8, 2, kv_dim);
        assert_eq!(kv.tokens(kv_dim), 8);
        // sink: tokens 0,1
        assert_eq!(kv.k[0], crate::f16::f32_to_f16(0.0));
        assert_eq!(kv.k[kv_dim], crate::f16::f32_to_f16(1.0));
        // recent: 14..19
        assert_eq!(kv.k[2 * kv_dim], crate::f16::f32_to_f16(14.0));
    }
}
