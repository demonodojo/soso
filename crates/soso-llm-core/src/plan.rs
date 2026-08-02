//! Planificador consciente de recursos: presupuesto de memoria, latencia
//! por capa/destino y replanificación adaptativa.
//!
//! Optimizaciones inspiradas en papers recientes de inferencia:
//! - **LayerKV / FlexGen**: working set de pocas capas residentes + prefetch
//!   layer-ahead (solapar I/O con cómputo).
//! - **StreamingLLM**: ventana KV (sink + recientes) bajo presión de memoria.
//! - **PagedAttention (espíritu)**: capacidad KV pre-reservada; no crecer
//!   ciegamente hasta OOM.
//! - **KIVI-lite**: KV int8 por token bajo presión de memoria (~2× ahorro).
//! - **H2O**: eviction por masa de atención + sink + recientes.
//! - **Quest-lite**: atención sparse por bloques en secuencias largas.
//!
//! Tras cada etapa de `/loop` que toque esto: actualizar skills
//! `soso-architecture` / `soso-dev` y `MANUAL-USUARIO.md` si hay UX nueva.

use crate::attn::SPARSE_TOKEN_THRESHOLD;
use crate::kv::KvDtype;
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
    /// Milisegundos gastados mapeando shards (prefetch).
    ///
    /// Esto NO entra en `avg_*_ms`: el cronómetro de la capa arranca después del
    /// prefetch y para antes del release, así que el streaming era invisible.
    pub stream_ms: u64,
    /// Milisegundos gastados desmapeando shards fuera del working set.
    pub release_ms: u64,
    /// Tokens aceptados por Prompt Lookup Decoding.
    pub pld_accepted: u32,
    /// Intentos de draft PLD (cadenas iniciadas).
    pub pld_attempts: u32,
    /// Cache LRU MoE: aciertos (experto ya residente).
    pub moe_hits: u32,
    /// Cache LRU MoE: fallos (experto frío, prefetch desde disco).
    pub moe_misses: u32,
    /// Prefetch MoE especulativo: acierto (hint = router real).
    pub moe_spec_hits: u32,
    /// Prefetch MoE especulativo: fallo (router distinto al hint).
    pub moe_spec_misses: u32,
    /// Milisegundos esperando staging layer-ahead (wait_prefetch).
    pub stage_wait_ms: u64,
    /// 0 = f16, 1 = int8 (KIVI-lite).
    pub kv_dtype_i8: u32,
    pub h2o_enabled: u32,
    pub sparse_attn: u32,
    /// EWMA ms por capa: proyecciones matvec vs atención softmax.
    pub avg_matvec_ms: f64,
    pub avg_attn_ms: f64,
    /// n-gramo preferido actual del PLD (autotune).
    pub pld_prefer_n: u32,
    pub pld_max_draft: u32,
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
    /// Bytes KV f16 por token (base); I8 usa la mitad efectiva.
    kv_bytes_per_token_f16: u64,
    kv_dtype: KvDtype,
    use_h2o: bool,
    use_sparse: bool,
    /// Hot path: EWMA de ms matvec (proyecciones) vs attn por capa.
    matvec_ms_ewma: f64,
    attn_ms_ewma: f64,
    /// PLD: n preferido y tope de draft (se afina con la tasa de aceptación).
    pld_prefer_n: usize,
    pld_max_draft: usize,
    /// Cache LRU de expertos MoE calientes (shard names).
    moe_cache: Vec<(u32, u32, Vec<String>)>,
    moe_cache_budget: u64,
    moe_cache_bytes: u64,
    /// Top-k del token anterior por capa (prefetch MoE especulativo).
    last_experts: Vec<Vec<u32>>,
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

/// ¿Tensor de un experto MoE? (`L00.E02.ffn_gate`, etc.)
pub fn is_expert_tensor(name: &str) -> bool {
    // Tras "Lxx." debe aparecer "E" + dos dígitos.
    let Some(rest) = name.strip_prefix("L") else {
        return false;
    };
    let Some(after_layer) = rest.get(2..) else {
        return false;
    };
    after_layer.starts_with(".E")
}

pub fn expert_tensor_prefix(layer: u32, expert: u32) -> String {
    format!("L{layer:02}.E{expert:02}.")
}

pub fn expert_shard_names(layer: u32, expert: u32) -> [String; 3] {
    let p = format!("L{layer:02}.E{expert:02}");
    [
        format!("{p}.ffn_gate"),
        format!("{p}.ffn_up"),
        format!("{p}.ffn_down"),
    ]
}

pub fn bytes_for_layer(layer: u32, index: &TensorIndex) -> u64 {
    let prefix = layer_tensor_prefix(layer);
    index
        .entries
        .iter()
        .filter(|e| e.name.starts_with(&prefix) && !is_expert_tensor(&e.name))
        .map(|e| e.byte_len)
        .sum()
}

pub fn bytes_for_expert(layer: u32, expert: u32, index: &TensorIndex) -> u64 {
    let prefix = expert_tensor_prefix(layer, expert);
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
            kv_bytes_per_token_f16: kv_bpt,
            kv_dtype: KvDtype::F16,
            use_h2o: false,
            use_sparse: false,
            matvec_ms_ewma: 0.0,
            attn_ms_ewma: 0.0,
            pld_prefer_n: 4,
            pld_max_draft: 8,
            moe_cache: Vec::new(),
            moe_cache_budget: weight_budget / 4,
            moe_cache_bytes: 0,
            last_experts: vec![Vec::new(); n],
            stats: PlannerStats {
                weight_budget_bytes: weight_budget,
                model_weight_bytes,
                pld_prefer_n: 4,
                pld_max_draft: 8,
                ..Default::default()
            },
        };
        planner.recompute_streaming_budgets(manifest, index);
        planner.rebuild_plan(manifest, index);
        planner
    }

    pub fn refresh_mem(&mut self, mem: MemSnapshot) {
        self.mem = mem;
        self.weight_budget = mem.weight_budget_bytes();
        self.stats.weight_budget_bytes = self.weight_budget;
    }

    pub fn recompute_streaming_budgets(&mut self, manifest: &Manifest, index: &TensorIndex) {
        // LayerKV: cuantas capas caben en el presupuesto de pesos.
        //
        // El tope son las capas del modelo, NO `DEFAULT_RESIDENT_LAYERS`. Ese 2
        // es el valor de reserva para cuando no sabemos cuánto pesa una capa; se
        // estaba usando además como máximo, así que un modelo que cabía entero en
        // memoria se quedaba con dos capas mapeadas y liberaba el resto —
        // obligando a remapearlo y a refaltar sus páginas en el token siguiente.
        //
        // Medido (2026-08-02): con `tiny` (2308 KiB) y presupuesto de 1 GiB —cabe
        // 400 veces— el planificador anunciaba «working-set 2 capas» y hacía 3
        // prefetch y 4 liberaciones POR TOKEN. El coste no salía en ningún
        // cronómetro porque `observe_layer` mide `forward_layer` y el streaming
        // ocurre fuera: 160 ms de capa medidos frente a 4,3 s de token real.
        let layers = if self.avg_layer_bytes == 0 {
            DEFAULT_RESIDENT_LAYERS
        } else {
            let fit = (self.weight_budget / self.avg_layer_bytes.max(1)).max(1) as u32;
            fit.min(manifest.num_layers.max(1))
        };
        self.resident_layers = layers.max(1);

        // Mitad del presupuesto para KV (el resto son pesos streaming + scratch).
        let kv_budget = self.weight_budget / 2;
        let bpt_f16 = self.kv_bytes_per_token_f16.max(1);
        let mut window = if bpt_f16 == 0 {
            manifest.max_seq as usize
        } else {
            let w = (kv_budget / bpt_f16) as usize;
            w.max(self.sink_tokens + 8)
                .min(manifest.max_seq as usize)
        };

        // KIVI-lite: si la ventana f16 queda muy corta vs max_seq, pasar a int8
        // (≈ mitad de bytes) y recalcular.
        let tight = window < (manifest.max_seq as usize / 4).max(64)
            || self.weight_budget < self.model_weight_bytes / 4;
        if tight {
            self.kv_dtype = KvDtype::I8;
            self.kv_bytes_per_token = bpt_f16 / 2;
            window = ((kv_budget / self.kv_bytes_per_token.max(1)) as usize)
                .max(self.sink_tokens + 8)
                .min(manifest.max_seq as usize);
        } else {
            self.kv_dtype = KvDtype::F16;
            self.kv_bytes_per_token = bpt_f16;
        }

        // H2O si hay presión (ventana < max_seq); sparse si la ventana puede ser larga.
        self.use_h2o = window < manifest.max_seq as usize || tight;
        self.use_sparse = window >= SPARSE_TOKEN_THRESHOLD;

        self.kv_window_tokens = window;
        self.stats.resident_layers = self.resident_layers;
        self.stats.kv_window_tokens = self.kv_window_tokens as u32;
        // Presupuesto LRU para expertos MoE: ~25 % del de pesos, mínimo 1 experto.
        self.moe_cache_budget = self.weight_budget / 4;
        if manifest.is_moe() {
            let per_expert = bytes_for_expert(0, 0, index);
            if per_expert > 0 {
                self.moe_cache_budget = self
                    .moe_cache_budget
                    .max(per_expert * manifest.num_experts_per_tok as u64);
            }
        }
        self.stats.kv_dtype_i8 = u32::from(matches!(self.kv_dtype, KvDtype::I8));
        self.stats.h2o_enabled = u32::from(self.use_h2o);
        self.stats.sparse_attn = u32::from(self.use_sparse);
    }

    pub fn kv_dtype(&self) -> KvDtype {
        self.kv_dtype
    }

    pub fn use_h2o(&self) -> bool {
        self.use_h2o
    }

    /// Quest-lite: sparse solo si el planner lo activó y la seq supera el umbral.
    pub fn use_sparse_attn(&self, seq: usize) -> bool {
        self.use_sparse && seq > SPARSE_TOKEN_THRESHOLD
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

    pub fn note_stream_ms(&mut self, ms: u64) {
        self.stats.stream_ms = self.stats.stream_ms.saturating_add(ms);
    }

    pub fn note_release_ms(&mut self, ms: u64) {
        self.stats.release_ms = self.stats.release_ms.saturating_add(ms);
    }

    pub fn note_stage_wait_ms(&mut self, ms: u64) {
        self.stats.stage_wait_ms = self.stats.stage_wait_ms.saturating_add(ms);
    }

    pub fn note_moe_spec_hit(&mut self) {
        self.stats.moe_spec_hits = self.stats.moe_spec_hits.saturating_add(1);
    }

    pub fn note_moe_spec_miss(&mut self) {
        self.stats.moe_spec_misses = self.stats.moe_spec_misses.saturating_add(1);
    }

    pub fn reset_moe_hints(&mut self) {
        for row in &mut self.last_experts {
            row.clear();
        }
    }

    /// Shards de expertos del token anterior (hint para prefetch especulativo).
    pub fn moe_speculative_shards(&self, layer: u32, index: &TensorIndex) -> Vec<String> {
        let li = layer as usize;
        if li >= self.last_experts.len() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for &expert in &self.last_experts[li] {
            for name in expert_shard_names(layer, expert) {
                out.push(format!("{name}.tensor"));
            }
        }
        let _ = index;
        out
    }

    pub fn note_kv_slide(&mut self) {
        self.stats.kv_slides = self.stats.kv_slides.saturating_add(1);
    }

    pub fn note_pld_attempt(&mut self) {
        self.stats.pld_attempts = self.stats.pld_attempts.saturating_add(1);
    }

    pub fn note_pld_accepted(&mut self, n: u32) {
        self.stats.pld_accepted = self.stats.pld_accepted.saturating_add(n);
    }

    /// Acumula timing del hot path (una capa).
    pub fn observe_hotpath(&mut self, matvec_ms: u64, attn_ms: u64) {
        ewma(&mut self.matvec_ms_ewma, matvec_ms as f64);
        ewma(&mut self.attn_ms_ewma, attn_ms as f64);
        self.stats.avg_matvec_ms = self.matvec_ms_ewma;
        self.stats.avg_attn_ms = self.attn_ms_ewma;
    }

    /// Parámetros PLD actuales: `(max_draft, min_n, max_n, hint_n)`.
    pub fn pld_params(&self) -> (usize, usize, usize, usize) {
        let max_d = self.pld_max_draft.clamp(1, 16);
        let hint = self.pld_prefer_n.clamp(2, 7);
        (max_d, 2, 7, hint)
    }

    /// Ajusta n-gramo / draft según tokens aceptados vs ofrecidos en un intento.
    pub fn tune_pld(&mut self, offered: usize, accepted: u32) {
        if offered == 0 {
            return;
        }
        let rate = accepted as f64 / offered as f64;
        if rate >= 0.75 {
            // Buena aceptación: drafts más largos; n un poco más corto (más hits).
            self.pld_max_draft = (self.pld_max_draft + 1).min(12);
            self.pld_prefer_n = self.pld_prefer_n.saturating_sub(1).max(2);
        } else if rate == 0.0 {
            // Fallo total: n más largo (más precisión), draft más corto.
            self.pld_prefer_n = (self.pld_prefer_n + 1).min(7);
            self.pld_max_draft = self.pld_max_draft.saturating_sub(1).max(2);
        } else if rate < 0.35 {
            self.pld_prefer_n = (self.pld_prefer_n + 1).min(7);
        }
        self.stats.pld_prefer_n = self.pld_prefer_n as u32;
        self.stats.pld_max_draft = self.pld_max_draft as u32;
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

    pub fn note_moe_miss(&mut self) {
        self.stats.moe_misses = self.stats.moe_misses.saturating_add(1);
    }

    pub fn note_moe_hit(&mut self) {
        self.stats.moe_hits = self.stats.moe_hits.saturating_add(1);
    }

    /// Registra uso de expertos; devuelve shards fríos a prefetch (post-router).
    pub fn touch_moe_experts(
        &mut self,
        layer: u32,
        experts: &[(u32, f32)],
        index: &TensorIndex,
    ) -> Vec<String> {
        let actual: alloc::vec::Vec<u32> = experts.iter().map(|(id, _)| *id).collect();
        let li = layer as usize;
        if li < self.last_experts.len() {
            let hint = &self.last_experts[li];
            if !hint.is_empty() {
                let mut hint_sorted = hint.clone();
                hint_sorted.sort_unstable();
                let mut actual_sorted = actual.clone();
                actual_sorted.sort_unstable();
                if hint_sorted == actual_sorted {
                    self.note_moe_spec_hit();
                } else {
                    self.note_moe_spec_miss();
                }
            }
            self.last_experts[li] = actual;
        }
        let mut prefetch = Vec::new();
        for &(expert, _) in experts {
            let shards: Vec<String> = expert_shard_names(layer, expert)
                .into_iter()
                .map(|n| format!("{n}.tensor"))
                .collect();
            let bytes = bytes_for_expert(layer, expert, index);
            let hit = self
                .moe_cache
                .iter()
                .any(|(l, e, _)| *l == layer && *e == expert);
            if hit {
                self.note_moe_hit();
            } else {
                self.note_moe_miss();
                prefetch.extend(shards.iter().cloned());
            }
            // Quitar entrada previa del mismo experto.
            if let Some(pos) = self
                .moe_cache
                .iter()
                .position(|(l, e, _)| *l == layer && *e == expert)
            {
                let (_, _, old) = self.moe_cache.remove(pos);
                self.moe_cache_bytes = self.moe_cache_bytes.saturating_sub(bytes);
                let _ = old;
            }
            // Insertar al frente (MRU).
            while self.moe_cache_bytes.saturating_add(bytes) > self.moe_cache_budget
                && !self.moe_cache.is_empty()
            {
                let (_, _, evicted) = self.moe_cache.pop().unwrap();
                let evicted_bytes: u64 = evicted
                    .iter()
                    .filter_map(|s| {
                        let name = s.strip_suffix(".tensor")?;
                        index.find(name).map(|e| e.byte_len)
                    })
                    .sum();
                self.moe_cache_bytes = self.moe_cache_bytes.saturating_sub(evicted_bytes);
            }
            self.moe_cache
                .insert(0, (layer, expert, shards.clone()));
            self.moe_cache_bytes = self.moe_cache_bytes.saturating_add(bytes);
        }
        prefetch
    }

    /// Shards de expertos en el cache LRU (mantener mapeados).
    pub fn moe_cached_shards(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (_, _, shards) in &self.moe_cache {
            out.extend(shards.iter().cloned());
        }
        out
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
        // La ventana se ancla en `start` y se extiende `resident_layers` HACIA
        // DELANTE. Antes acababa en `layer + 1`, o sea que sólo miraba hacia
        // atrás: en la capa 0 la ventana era `[0,1)` y liberaba las capas 2 y 3
        // recién mapeadas, para volver a mapearlas dos capas después. Con un
        // presupuesto que da para el modelo entero eso son dos desalojos y sus
        // refaltos por token, gratis para nadie.
        let start = layer.saturating_sub(self.resident_layers.saturating_sub(1));
        let end = start
            .saturating_add(self.resident_layers)
            .max(layer + 1)
            .min(layer_end)
            .min(manifest.num_layers);
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

    /// Shards a retener tras `layer`, incluyendo expertos MoE en cache LRU.
    pub fn keep_all_shards_after(
        &self,
        layer: u32,
        layer_end: u32,
        manifest: &Manifest,
    ) -> Vec<String> {
        let mut keep = self.keep_shards_after(layer, layer_end, manifest);
        keep.extend(self.moe_cached_shards());
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
        self.recompute_streaming_budgets(manifest, index);
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
    use sosomodel::index::{make_f32_entry, TensorIndex};
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
    fn hotpath_and_pld_tune() {
        let manifest = Manifest::tiny("t");
        let index = TensorIndex::default();
        let mut planner = ResourcePlanner::new(&manifest, &index, MemSnapshot::default(), 0, false);
        planner.observe_hotpath(10, 2);
        planner.observe_hotpath(14, 4);
        assert!(planner.stats().avg_matvec_ms > 0.0);
        assert!(planner.stats().avg_attn_ms > 0.0);
        assert!(planner.stats().avg_matvec_ms > planner.stats().avg_attn_ms);
        let n0 = planner.pld_prefer_n;
        planner.tune_pld(8, 0);
        assert!(planner.pld_prefer_n >= n0);
        let d0 = planner.pld_max_draft;
        planner.tune_pld(4, 4);
        assert!(planner.pld_max_draft >= d0);
    }

    #[test]
    fn moe_expert_cache_tracks_hits() {
        let manifest = Manifest::tiny_moe("moe");
        let mut index = TensorIndex::default();
        for layer in 0..manifest.num_layers {
            for expert in 0..manifest.num_experts {
                let p = format!("L{layer:02}.E{expert:02}.ffn_gate");
                index.entries.push(make_f32_entry(
                    index.entries.len() as u32,
                    &p,
                    &format!("{p}.tensor"),
                    0,
                    &[manifest.expert_ffn_dim(), manifest.hidden_dim],
                ));
            }
        }
        let mem = MemSnapshot {
            total_frames: 100_000,
            free_frames: 50_000,
            reclaimable_frames: 0,
        };
        let mut planner = ResourcePlanner::new(&manifest, &index, mem, 0, false);
        let experts = [(0u32, 1.0), (1, 0.0)];
        let pf = planner.touch_moe_experts(0, &experts, &index);
        assert!(!pf.is_empty());
        assert_eq!(planner.stats().moe_misses, 2);
        let _ = planner.touch_moe_experts(0, &experts, &index);
        assert!(planner.stats().moe_hits >= 2);
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
        use crate::kv::LayerKv;
        let kv_dim = 4;
        let mut kv = LayerKv::new();
        for t in 0..20u16 {
            let k = [t as f32; 4];
            let v = [(t + 100) as f32; 4];
            kv.append_f16(&k, &v);
        }
        kv.slide_window(8, 2, kv_dim);
        assert_eq!(kv.tokens(kv_dim), 8);
        let k = kv.k_f16_slice();
        // sink: tokens 0,1
        assert_eq!(k[0], crate::f16::f32_to_f16(0.0));
        assert_eq!(k[kv_dim], crate::f16::f32_to_f16(1.0));
        // recent: 14..19
        assert_eq!(k[2 * kv_dim], crate::f16::f32_to_f16(14.0));
    }
}
