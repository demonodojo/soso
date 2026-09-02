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
#[cfg(feature = "std")]
extern crate std;
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
/// Anillo de capas en streaming (prefetch capa N+1 mientras se computa N).
const DEFAULT_RING_SLOTS: u32 = 2;
/// Tokens sink (StreamingLLM) que nunca se evictan del KV.
const DEFAULT_SINK_TOKENS: usize = 4;
/// Suelo absoluto para clasificar embed como gather-only (estilo airllm).
const GATHER_ONLY_TABLE_FLOOR_BYTES: u64 = 64 * 1024 * 1024;

/// Presets de reparto trunk-first (estilo kimi-k3-in-c).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MemoryPreset {
    #[default]
    Auto,
    /// Prioriza tronco al máximo; caché MoE mínima.
    Tight,
    /// Reparto equilibrado trunk/expert.
    Balanced,
    /// Pin de todas las capas que quepan; residual a expertos.
    MaxPin,
}

/// Configuración explícita del plan de memoria (genérica por índice de tensores).
#[derive(Clone, Copy, Debug)]
pub struct MemoryPlanConfig {
    pub preset: MemoryPreset,
    /// Fracción 0–100 del presupuesto de pesos para tronco (pin+anillo).
    /// `None` = derivar del preset.
    pub trunk_frac_pct: Option<u32>,
    /// Capas en anillo de streaming (1–2).
    pub ring_slots: u32,
}

impl Default for MemoryPlanConfig {
    fn default() -> Self {
        Self {
            preset: MemoryPreset::Auto,
            trunk_frac_pct: None,
            ring_slots: DEFAULT_RING_SLOTS,
        }
    }
}

/// Clasificación de bytes del índice (independiente de arquitectura concreta).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeightClassBytes {
    pub trunk_bytes: u64,
    pub routed_expert_bytes: u64,
    pub always_resident_bytes: u64,
}

impl WeightClassBytes {
    pub fn total(&self) -> u64 {
        self.trunk_bytes
            .saturating_add(self.routed_expert_bytes)
            .saturating_add(self.always_resident_bytes)
    }
}

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
    /// Sufijos de tronco en VRAM (`attn_q`, `S00.ffn_up`, …), exclusivos por capa.
    pub gpu_tensors: Vec<String>,
    /// Los expertos enrutados de esta capa pueden usar el pool compartido de VRAM.
    pub gpu_experts: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlannerStats {
    pub replans: u32,
    pub cpu_layers: u32,
    pub gpu_layers: u32,
    pub remote_layers: u32,
    /// Expertos completos (gate+up+down) que caben a la vez en el pool de VRAM.
    pub gpu_expert_slots: u32,
    /// Capas Gpu en las que los expertos MoE pueden offloadearse.
    pub gpu_expert_layers: u32,
    pub tokens_observed: u32,
    pub avg_cpu_ms: f64,
    pub avg_gpu_ms: f64,
    pub avg_remote_ms: f64,
    pub weight_budget_bytes: u64,
    pub model_weight_bytes: u64,
    /// Capas de pesos en el working set (streaming + pin).
    pub resident_layers: u32,
    /// Capas pinneadas al inicio (nunca desmapeadas entre tokens).
    pub pinned_layers: u32,
    /// Slots del anillo de streaming tras el prefijo pinneado.
    pub ring_slots: u32,
    /// Presupuesto de bytes para tronco (pin + anillo).
    pub trunk_budget_bytes: u64,
    /// Presupuesto LRU de expertos enrutados.
    pub moe_cache_budget_bytes: u64,
    /// Aciertos de tronco: capa ya residente (pin o anillo caliente).
    pub trunk_hits: u32,
    /// Fallos de tronco: capa fría remapeada/refaultada.
    pub trunk_misses: u32,
    /// Bytes de tronco leídos por fallos (aprox).
    pub trunk_bytes_read: u64,
    /// MoE: acierto con experto ya en LRU residente.
    pub moe_resident_hits: u32,
    /// MoE: acierto tras prefetch JIT (no estaba en LRU al router).
    pub moe_jit_hits: u32,
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
    /// Cache LRU MoE: aciertos totales (resident + JIT).
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
    /// EWMA ms esperando prefetch layer-ahead (I/O-bound detector).
    pub avg_stage_wait_ms: f64,
    /// 1 si stage_wait domina matvec+attn (cuello de botella en disco).
    pub io_bound: u32,
    /// 1 si el anillo está forzado a 1 slot (capas demasiado grandes para double-buffer).
    pub prefetch_single_buffered: u32,
    /// 1 si embed es gather-only (no se mantiene mapeado entero).
    pub embed_gather_only: u32,
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
    /// Capas de pesos a mantener mapeadas a la vez (pin + anillo).
    resident_layers: u32,
    /// Prefijo de capas pinneadas (0..pinned_layers).
    pinned_layers: u32,
    /// Slots del anillo tras el prefijo.
    ring_slots: u32,
    /// Presupuesto explícito de tronco y expertos.
    trunk_budget: u64,
    plan_config: MemoryPlanConfig,
    weight_classes: WeightClassBytes,
    /// Capas del anillo ya calientes en el token actual.
    ring_warm: Vec<u32>,
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
    /// EWMA ms esperando staging layer-ahead.
    stage_wait_ms_ewma: f64,
    /// Cuello de botella en I/O de pesos (stage_wait ≫ compute).
    io_bound: bool,
    /// Embed demasiado grande: gather por fila, no residente entero.
    embed_gather_only: bool,
    /// PLD: n preferido y tope de draft (se afina con la tasa de aceptación).
    pld_prefer_n: usize,
    pld_max_draft: usize,
    /// Cache LRU de expertos MoE calientes (shard names).
    moe_cache: Vec<(u32, u32, Vec<String>)>,
    moe_cache_budget: u64,
    moe_cache_bytes: u64,
    /// Top-k del token anterior por capa (prefetch MoE especulativo).
    last_experts: Vec<Vec<u32>>,
    /// Expertos fríos del último touch_moe_experts (para telemetría JIT).
    last_moe_cold: u32,
    #[cfg(feature = "std")]
    /// Traza (layer, expert) para sim-moe-cache en host.
    moe_trace: Vec<(u32, u32)>,
    stats: PlannerStats,
}

/// Tronco que se pinnea en VRAM por capa (atención + router + FFN denso).
/// Los expertos enrutados van en un pool aparte: no se reserva uno por capa
/// (Mixtral usa 2 de 8; pinnear los 8 desperdicia la tarjeta).
const TRUNK_GPU_PROJ: [&str; 8] = [
    "attn_q",
    "attn_k",
    "attn_v",
    "attn_output",
    "ffn_gate_inp",
    "ffn_gate",
    "ffn_up",
    "ffn_down",
];

pub fn layer_tensor_prefix(layer: u32) -> String {
    format!("L{layer:02}.")
}

/// ¿Tensor de experto compartido MoE? (`L00.S02.ffn_gate`, etc.)
pub fn is_shared_expert_tensor(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("L") else {
        return false;
    };
    let Some(after_layer) = rest.get(2..) else {
        return false;
    };
    after_layer.starts_with(".S")
}

pub fn shared_expert_shard_names(layer: u32, shared: u32) -> [String; 3] {
    let p = format!("L{layer:02}.S{shared:02}");
    [
        format!("{p}.ffn_gate"),
        format!("{p}.ffn_up"),
        format!("{p}.ffn_down"),
    ]
}

/// ¿Tensor de un experto MoE enrutado? (`L00.E02.ffn_gate`, etc.)
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

/// Bytes del tensor de embedding (tabla de lookup).
pub fn bytes_for_embed(index: &TensorIndex) -> u64 {
    index.find("embed").map(|e| e.byte_len).unwrap_or(0)
}

/// ¿Tensor siempre residente (embed, norm final, lm_head)?
pub fn is_always_resident_tensor(name: &str) -> bool {
    matches!(name, "embed" | "output_norm" | "lm_head")
        || name.starts_with("embed.")
        || name.starts_with("output_norm.")
        || name.starts_with("lm_head.")
}

/// Clasifica bytes del índice en tronco / expertos enrutados / siempre residentes.
pub fn classify_weight_bytes(index: &TensorIndex) -> WeightClassBytes {
    let mut out = WeightClassBytes::default();
    for e in &index.entries {
        if is_always_resident_tensor(&e.name) {
            out.always_resident_bytes = out.always_resident_bytes.saturating_add(e.byte_len);
        } else if is_shared_expert_tensor(&e.name) {
            out.trunk_bytes = out.trunk_bytes.saturating_add(e.byte_len);
        } else if is_expert_tensor(&e.name) {
            out.routed_expert_bytes = out.routed_expert_bytes.saturating_add(e.byte_len);
        } else {
            out.trunk_bytes = out.trunk_bytes.saturating_add(e.byte_len);
        }
    }
    out
}

/// Bytes de tronco (sin expertos) de una capa.
pub fn bytes_for_trunk_layer(layer: u32, index: &TensorIndex) -> u64 {
    bytes_for_layer(layer, index)
}

/// Fracción trunk-first según preset (0–100).
pub fn trunk_frac_for_preset(preset: MemoryPreset, model_fits: bool) -> u32 {
    match preset {
        MemoryPreset::Auto => {
            if model_fits {
                100
            } else {
                90
            }
        }
        MemoryPreset::Tight => 95,
        MemoryPreset::Balanced => 85,
        MemoryPreset::MaxPin => 98,
    }
}

/// Calcula reparto trunk-first: tronco pin+anillo antes que caché MoE.
pub fn compute_trunk_first_split(
    weight_budget: u64,
    classes: &WeightClassBytes,
    avg_layer_bytes: u64,
    num_layers: u32,
    per_expert_bytes: u64,
    top_k: u32,
    trunk_frac_pct: u32,
    ring_slots: u32,
) -> (u64, u64, u32, u32) {
    let ring = ring_slots.clamp(1, 2);
    let trunk_frac = trunk_frac_pct.min(100) as u64;
    let mut trunk_budget = weight_budget.saturating_mul(trunk_frac) / 100;
    let min_moe = per_expert_bytes.saturating_mul(top_k.max(1) as u64);
    if classes.routed_expert_bytes > 0 {
        trunk_budget = trunk_budget.min(weight_budget.saturating_sub(min_moe));
    }
    let moe_budget = weight_budget.saturating_sub(trunk_budget);

    let layer_bytes = avg_layer_bytes.max(1);
    let ring_bytes = layer_bytes.saturating_mul(ring as u64);
    let pin_bytes = trunk_budget.saturating_sub(ring_bytes);
    let mut pinned = (pin_bytes / layer_bytes) as u32;
    pinned = pinned.min(num_layers);
    let streaming = num_layers.saturating_sub(pinned);
    let resident = if streaming == 0 {
        pinned.max(1)
    } else {
        pinned.saturating_add(ring).min(num_layers).max(1)
    };
    (trunk_budget, moe_budget.max(min_moe), pinned, resident)
}

/// Bytes de KV f16 por token de secuencia (todas las capas).
pub fn kv_bytes_per_token(manifest: &Manifest) -> u64 {
    use sosomodel::manifest::AttnKind;
    let mut total = 0u64;
    for layer in 0..manifest.num_layers {
        let spec = manifest.layer(layer).cloned().unwrap_or_default();
        if spec.attn_kind == AttnKind::Mla && spec.kv_lora_rank > 0 {
            // Un vector latente c_kv por token (f16).
            total += spec.kv_lora_rank as u64 * 2;
        } else {
            let head_dim = manifest.hidden_dim as u64 / manifest.effective_num_heads(layer) as u64;
            let kv_dim = manifest.effective_num_kv_heads(layer) as u64 * head_dim;
            total += kv_dim * 2 * 2;
        }
    }
    total
}

impl ResourcePlanner {
    pub fn new(
        manifest: &Manifest,
        index: &TensorIndex,
        mem: MemSnapshot,
        vram_free: u64,
        remote_available: bool,
    ) -> Self {
        Self::with_config(
            manifest,
            index,
            mem,
            vram_free,
            remote_available,
            MemoryPlanConfig::default(),
        )
    }

    pub fn with_config(
        manifest: &Manifest,
        index: &TensorIndex,
        mem: MemSnapshot,
        vram_free: u64,
        remote_available: bool,
        plan_config: MemoryPlanConfig,
    ) -> Self {
        let n = manifest.num_layers as usize;
        let weight_budget = mem.weight_budget_bytes();
        let weight_classes = classify_weight_bytes(index);
        let avg_layer = if n == 0 {
            0
        } else {
            (0..manifest.num_layers)
                .map(|l| bytes_for_trunk_layer(l, index))
                .sum::<u64>()
                / n as u64
        };
        let kv_bpt = kv_bytes_per_token(manifest);
        let ring_slots = plan_config.ring_slots.clamp(1, 2);
        let mut planner = Self {
            layer_plans: Vec::with_capacity(n),
            layer_ms_cpu: vec![0.0; n],
            layer_ms_gpu: vec![0.0; n],
            layer_ms_remote: vec![0.0; n],
            tokens_since_replan: 0,
            mem,
            weight_budget,
            model_weight_bytes: weight_classes.total(),
            vram_free,
            remote_available,
            remote_degraded: false,
            remote_rtt_ms: 0.0,
            resident_layers: DEFAULT_RESIDENT_LAYERS,
            pinned_layers: 0,
            ring_slots,
            trunk_budget: 0,
            plan_config,
            weight_classes,
            ring_warm: Vec::new(),
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
            stage_wait_ms_ewma: 0.0,
            io_bound: false,
            embed_gather_only: false,
            pld_prefer_n: 4,
            pld_max_draft: 8,
            moe_cache: Vec::new(),
            moe_cache_budget: 0,
            moe_cache_bytes: 0,
            last_experts: vec![Vec::new(); n],
            last_moe_cold: 0,
            #[cfg(feature = "std")]
            moe_trace: Vec::new(),
            stats: PlannerStats {
                weight_budget_bytes: weight_budget,
                model_weight_bytes: weight_classes.total(),
                ring_slots,
                pld_prefer_n: 4,
                pld_max_draft: 8,
                ..Default::default()
            },
        };
        planner.recompute_streaming_budgets(manifest, index);
        planner.rebuild_plan(manifest, index);
        planner
    }

    pub fn plan_config(&self) -> MemoryPlanConfig {
        self.plan_config
    }

    pub fn pinned_layers(&self) -> u32 {
        self.pinned_layers
    }

    pub fn ring_slots(&self) -> u32 {
        self.ring_slots
    }

    pub fn trunk_budget(&self) -> u64 {
        self.trunk_budget
    }

    pub fn moe_cache_budget(&self) -> u64 {
        self.moe_cache_budget
    }

    pub fn weight_classes(&self) -> WeightClassBytes {
        self.weight_classes
    }

    /// Resumen legible del plan (para logs al arrancar).
    pub fn memory_plan_summary(&self) -> String {
        format!(
            "trunk {} KiB (pin {} capas, anillo {}), expert cache {} KiB, always-resident {} KiB",
            self.trunk_budget / 1024,
            self.pinned_layers,
            self.ring_slots,
            self.moe_cache_budget / 1024,
            self.weight_classes.always_resident_bytes / 1024,
        )
    }

    pub fn refresh_mem(&mut self, mem: MemSnapshot) {
        self.mem = mem;
        self.weight_budget = mem.weight_budget_bytes();
        self.stats.weight_budget_bytes = self.weight_budget;
    }

    pub fn recompute_streaming_budgets(&mut self, manifest: &Manifest, index: &TensorIndex) {
        self.weight_classes = classify_weight_bytes(index);
        self.stats.model_weight_bytes = self.weight_classes.total();

        let model_fits = self.weight_budget >= self.weight_classes.trunk_bytes;
        let trunk_frac = self
            .plan_config
            .trunk_frac_pct
            .unwrap_or_else(|| trunk_frac_for_preset(self.plan_config.preset, model_fits));

        let per_expert = if manifest.is_moe() {
            bytes_for_expert(0, 0, index)
        } else {
            0
        };
        let top_k = manifest.num_experts_per_tok.max(1);

        // Anillo efectivo: config → cap double-buffer (airllm) → hold bajo I/O-bound.
        let config_ring = self.plan_config.ring_slots.clamp(1, 2);
        let can_double_buffer =
            self.avg_layer_bytes.saturating_mul(2) <= self.mem.free_bytes();
        let mut effective_ring = if can_double_buffer { config_ring } else { 1 };
        if self.io_bound && can_double_buffer {
            effective_ring = 2;
        }
        self.ring_slots = effective_ring.clamp(1, 2);
        self.stats.prefetch_single_buffered = u32::from(!can_double_buffer);
        self.stats.ring_slots = self.ring_slots;

        // Embed oversized: gather-only (no mantener embed.tensor mapeado).
        let embed_bytes = bytes_for_embed(index);
        let gather_threshold = GATHER_ONLY_TABLE_FLOOR_BYTES.max(self.weight_budget / 4);
        self.embed_gather_only = embed_bytes > gather_threshold;
        self.stats.embed_gather_only = u32::from(self.embed_gather_only);

        let (trunk_budget, moe_budget, pinned, resident) = compute_trunk_first_split(
            self.weight_budget,
            &self.weight_classes,
            self.avg_layer_bytes,
            manifest.num_layers,
            per_expert,
            top_k,
            trunk_frac,
            self.ring_slots,
        );

        self.trunk_budget = trunk_budget;
        self.moe_cache_budget = moe_budget;
        self.pinned_layers = pinned;
        self.resident_layers = resident;
        self.stats.pinned_layers = pinned;
        self.stats.trunk_budget_bytes = trunk_budget;
        self.stats.moe_cache_budget_bytes = moe_budget;
        self.stats.resident_layers = resident;

        // Si el modelo cabe entero, pin todas las capas (sin anillo).
        if model_fits && self.weight_classes.routed_expert_bytes == 0 {
            self.pinned_layers = manifest.num_layers;
            self.resident_layers = manifest.num_layers;
            self.stats.pinned_layers = manifest.num_layers;
            self.stats.resident_layers = manifest.num_layers;
        }

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
        self.stats.kv_window_tokens = self.kv_window_tokens as u32;
        self.stats.kv_dtype_i8 = u32::from(matches!(self.kv_dtype, KvDtype::I8));
        self.stats.h2o_enabled = u32::from(self.use_h2o);
        self.stats.sparse_attn = u32::from(self.use_sparse);
    }

    /// Registra acceso a capa de tronco (telemetría true-resident).
    pub fn note_trunk_layer(&mut self, layer: u32, index: &TensorIndex) {
        let pinned = layer < self.pinned_layers;
        let warm = self.ring_warm.iter().any(|&l| l == layer);
        if pinned || warm {
            self.stats.trunk_hits = self.stats.trunk_hits.saturating_add(1);
        } else {
            self.stats.trunk_misses = self.stats.trunk_misses.saturating_add(1);
            self.stats.trunk_bytes_read = self
                .stats
                .trunk_bytes_read
                .saturating_add(bytes_for_trunk_layer(layer, index));
        }
        if layer >= self.pinned_layers && !warm {
            self.ring_warm.push(layer);
            while self.ring_warm.len() > self.ring_slots as usize {
                self.ring_warm.remove(0);
            }
        }
    }

    pub fn begin_token(&mut self) {
        self.ring_warm.clear();
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
        if ms > 0 {
            ewma(&mut self.stage_wait_ms_ewma, ms as f64);
            self.stats.avg_stage_wait_ms = self.stage_wait_ms_ewma;
            self.recompute_io_bound();
        }
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
        self.recompute_io_bound();
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
        let mut cold = 0u32;
        for &(expert, _) in experts {
            #[cfg(feature = "std")]
            if std::env::var("SOSO_MOE_TRACE").is_ok() {
                self.moe_trace.push((layer, expert));
            }
            let shards: Vec<String> = expert_shard_names(layer, expert)
                .into_iter()
                .map(|n| format!("{n}.tensor"))
                .collect();
            let bytes = bytes_for_expert(layer, expert, index);
            let resident_hit = self
                .moe_cache
                .iter()
                .any(|(l, e, _)| *l == layer && *e == expert);
            if resident_hit {
                self.note_moe_hit();
                self.stats.moe_resident_hits = self.stats.moe_resident_hits.saturating_add(1);
            } else {
                self.note_moe_miss();
                cold = cold.saturating_add(1);
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
        self.last_moe_cold = cold;
        prefetch
    }

    pub fn last_moe_cold(&self) -> u32 {
        self.last_moe_cold
    }

    /// Traza MoE acumulada (solo host + `SOSO_MOE_TRACE=1`).
    #[cfg(feature = "std")]
    pub fn moe_trace(&self) -> &[(u32, u32)] {
        &self.moe_trace
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
        let Some(p) = self.layer_plans.get(layer as usize) else {
            return false;
        };
        if p.dest != ExecDest::Gpu {
            return false;
        }
        if is_expert_tensor(tensor) {
            return p.gpu_experts;
        }
        let prefix = layer_tensor_prefix(layer);
        p.gpu_tensors.iter().any(|suf| {
            tensor.strip_prefix(prefix.as_str()) == Some(suf.as_str()) || tensor == suf.as_str()
        })
    }

    /// Marca expertos servidos tras prefetch JIT (no estaban residentes al router).
    pub fn note_moe_jit_served(&mut self, n: u32) {
        self.stats.moe_jit_hits = self.stats.moe_jit_hits.saturating_add(n);
        self.stats.moe_hits = self.stats.moe_hits.saturating_add(n);
    }

    /// Shards a mantener tras terminar `layer` (pin prefix + anillo + prefetch).
    pub fn keep_shards_after(
        &self,
        layer: u32,
        layer_end: u32,
        manifest: &Manifest,
    ) -> Vec<String> {
        let mut keep = Vec::new();
        // Prefijo pinneado: capas 0..pinned nunca se sueltan.
        for l in 0..self.pinned_layers.min(manifest.num_layers) {
            if let Some(pf) = manifest.prefetch.get(l as usize) {
                keep.extend(pf.shards.iter().cloned());
            }
        }
        // Anillo: ventana sobre capas no pinneadas + prefetch adelantado.
        if self.pinned_layers < manifest.num_layers {
            let ring_start = if layer >= self.pinned_layers {
                layer.saturating_sub(self.ring_slots.saturating_sub(1))
            } else {
                self.pinned_layers
            };
            let ring_end = (layer + 2)
                .min(layer_end)
                .min(manifest.num_layers);
            for l in ring_start.max(self.pinned_layers)..ring_end {
                if let Some(pf) = manifest.prefetch.get(l as usize) {
                    keep.extend(pf.shards.iter().cloned());
                }
            }
        }
        // Siempre residentes (embed, lm_head, …).
        keep.extend(self.always_resident_shards());
        keep
    }

    fn always_resident_shards(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.embed_gather_only {
            out.push(String::from("embed.tensor"));
        }
        out.push(String::from("output_norm.tensor"));
        out.push(String::from("lm_head.tensor"));
        out
    }

    /// Recalcula `io_bound`: stage_wait domina matvec+attn cuando ambos EWMAs están calientes.
    fn recompute_io_bound(&mut self) {
        self.stats.avg_stage_wait_ms = self.stage_wait_ms_ewma;
        let compute_warm = self.matvec_ms_ewma > 0.0 && self.attn_ms_ewma > 0.0;
        self.io_bound = compute_warm
            && self.stage_wait_ms_ewma > 0.0
            && self.stage_wait_ms_ewma > (self.matvec_ms_ewma + self.attn_ms_ewma);
        self.stats.io_bound = u32::from(self.io_bound);
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
        self.begin_token();
        self.tokens_since_replan += 1;
        self.recompute_io_bound();
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

        // Pase 1: destino + tronco en VRAM (atención, router, FFN denso, Sxx).
        // El *8 a f32 estaba mal: Q4_K/Q8_0 se suben en crudo.
        for layer in 0..manifest.num_layers {
            let lb = bytes_for_layer(layer, index);
            let dest0 = choose_dest(
                lb,
                stream_budget,
                self.model_weight_bytes,
                vram_left,
                self.remote_available,
                self.remote_degraded,
                self.layer_ms_cpu.get(layer as usize).copied().unwrap_or(0.0),
                self.layer_ms_remote.get(layer as usize).copied().unwrap_or(0.0),
                self.remote_rtt_ms,
                self.io_bound,
            );
            let mut gpu_tensors = Vec::new();
            let dest = if dest0 == ExecDest::Gpu {
                gpu_tensors = pack_trunk_gpu(layer, manifest, index, &mut vram_left);
                if gpu_tensors.is_empty() {
                    cpu_layers += 1;
                    ExecDest::Cpu
                } else {
                    gpu_layers += 1;
                    ExecDest::Gpu
                }
            } else if dest0 == ExecDest::Remote {
                remote_layers += 1;
                dest0
            } else {
                cpu_layers += 1;
                dest0
            };
            self.layer_plans.push(LayerPlan {
                layer,
                dest,
                gpu_tensors,
                gpu_experts: false,
            });
        }

        // Pase 2: pool compartido de expertos. No se pinnea E00..E07 por capa
        // (Mixtral activa top-k): SysGpu ya hace LRU por nombre.
        let per_expert = vram_bytes_for_expert(0, 0, index);
        let slots = if per_expert == 0 {
            0
        } else {
            (vram_left / per_expert) as u32
        };
        let mut expert_layers = 0u32;
        if manifest.is_moe() && slots >= 1 {
            for p in &mut self.layer_plans {
                if p.dest == ExecDest::Gpu {
                    p.gpu_experts = true;
                    expert_layers += 1;
                }
            }
        }
        self.stats.cpu_layers = cpu_layers;
        self.stats.gpu_layers = gpu_layers;
        self.stats.remote_layers = remote_layers;
        self.stats.gpu_expert_slots = slots;
        self.stats.gpu_expert_layers = expert_layers;
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

/// Bytes que ocupará el tensor en VRAM: Q4_K/Q8_0 en crudo si `cols` es bloque;
/// si no, el plano f32 del fallback de subida.
fn vram_bytes_for_entry(e: &sosomodel::index::TensorEntry) -> u64 {
    let cols = e.shape.get(1).copied().unwrap_or(0) as usize;
    match e.dtype {
        sosomodel::layout::DTYPE_Q4_K | sosomodel::layout::DTYPE_Q8_0
            if sosomodel::dequant::row_bytes(e.dtype, cols).is_some() =>
        {
            e.byte_len
        }
        sosomodel::layout::DTYPE_F32 => e.byte_len,
        _ => (e.elems() as u64).saturating_mul(4),
    }
}

fn vram_bytes_for_named(index: &TensorIndex, name: &str) -> u64 {
    index.find(name).map(vram_bytes_for_entry).unwrap_or(0)
}

fn vram_bytes_for_expert(layer: u32, expert: u32, index: &TensorIndex) -> u64 {
    expert_shard_names(layer, expert)
        .iter()
        .map(|n| vram_bytes_for_named(index, n))
        .sum()
}

fn pack_trunk_gpu(
    layer: u32,
    manifest: &Manifest,
    index: &TensorIndex,
    vram_left: &mut u64,
) -> Vec<String> {
    let prefix = layer_tensor_prefix(layer);
    let mut out = Vec::new();
    for suffix in TRUNK_GPU_PROJ {
        let name = format!("{prefix}{suffix}");
        let Some(e) = index.find(&name) else {
            continue;
        };
        let need = vram_bytes_for_entry(e);
        if need == 0 || need > *vram_left {
            continue;
        }
        *vram_left -= need;
        out.push(String::from(suffix));
    }
    let n_shared = manifest
        .layer(layer)
        .map(|sp| sp.num_shared_experts)
        .unwrap_or(0);
    for shared in 0..n_shared {
        let names = shared_expert_shard_names(layer, shared);
        let need: u64 = names.iter().map(|n| vram_bytes_for_named(index, n)).sum();
        if need == 0 || need > *vram_left {
            continue;
        }
        *vram_left -= need;
        for n in &names {
            if let Some(suf) = n.strip_prefix(prefix.as_str()) {
                out.push(String::from(suf));
            }
        }
    }
    out
}

fn choose_dest(
    _layer_bytes: u64,
    weight_budget: u64,
    model_bytes: u64,
    vram_free: u64,
    remote_available: bool,
    remote_degraded: bool,
    cpu_ms: f64,
    remote_layer_ms: f64,
    remote_rtt_ms: f64,
    io_bound: bool,
) -> ExecDest {
    if remote_available && !remote_degraded {
        if remote_layer_ms > 0.0 && cpu_ms > 0.0 && remote_layer_ms + remote_rtt_ms < cpu_ms {
            return ExecDest::Remote;
        }
        if model_bytes > weight_budget.saturating_mul(2) && remote_rtt_ms < REMOTE_SLOW_MS {
            return ExecDest::Remote;
        }
        // I/O-bound: el peer remoto evita lecturas locales de disco.
        if io_bound && remote_rtt_ms < REMOTE_SLOW_MS {
            return ExecDest::Remote;
        }
    }
    if vram_free > 0 {
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
    fn trunk_first_split_prioritizes_trunk() {
        let classes = WeightClassBytes {
            trunk_bytes: 1000,
            routed_expert_bytes: 9000,
            always_resident_bytes: 100,
        };
        let (trunk, moe, pinned, resident) = compute_trunk_first_split(
            1000,
            &classes,
            100,
            10,
            50,
            2,
            90,
            2,
        );
        assert!(trunk >= moe || moe <= 100);
        assert!(pinned > 0);
        assert!(resident >= pinned);
    }

    #[test]
    fn classify_weights_splits_experts() {
        let mut index = TensorIndex::default();
        index.entries.push(make_f32_entry(
            0,
            "embed",
            "embed.tensor",
            0,
            &[4, 8],
        ));
        index.entries.push(make_f32_entry(
            1,
            "L00.attn_q",
            "L00.attn_q.tensor",
            0,
            &[8, 8],
        ));
        index.entries.push(make_f32_entry(
            2,
            "L00.E00.ffn_gate",
            "L00.E00.ffn_gate.tensor",
            0,
            &[16, 8],
        ));
        let c = classify_weight_bytes(&index);
        assert!(c.always_resident_bytes > 0);
        assert!(c.trunk_bytes > 0);
        assert!(c.routed_expert_bytes > 0);
    }

    #[test]
    fn trunk_first_pins_more_than_equal_split() {
        let weight_budget = 10_000_000u64;
        let classes = WeightClassBytes {
            trunk_bytes: 8_000_000,
            routed_expert_bytes: 4_000_000,
            always_resident_bytes: 500_000,
        };
        let avg_layer = 1_000_000;
        let num_layers = 8;
        let per_expert = 200_000;
        let top_k = 2;
        let (trunk_tight, _, pinned_tight, _) = compute_trunk_first_split(
            weight_budget,
            &classes,
            avg_layer,
            num_layers,
            per_expert,
            top_k,
            85,
            2,
        );
        let (trunk_equal, _, pinned_equal, _) = compute_trunk_first_split(
            weight_budget,
            &classes,
            avg_layer,
            num_layers,
            per_expert,
            top_k,
            50,
            2,
        );
        assert!(trunk_tight > trunk_equal);
        assert!(pinned_tight >= pinned_equal);
    }

    #[test]
    fn pinned_keep_includes_prefix() {
        let manifest = Manifest::tiny("t");
        let index = TensorIndex::default();
        let mem = MemSnapshot {
            total_frames: 100_000,
            free_frames: 50_000,
            reclaimable_frames: 0,
        };
        let cfg = MemoryPlanConfig {
            preset: MemoryPreset::MaxPin,
            trunk_frac_pct: Some(98),
            ring_slots: 2,
        };
        let planner = ResourcePlanner::with_config(&manifest, &index, mem, 0, false, cfg);
        let keep = planner.keep_shards_after(0, manifest.num_layers, &manifest);
        assert!(keep.iter().any(|s| s.contains("embed")));
    }

    fn moe_gpu_index(m: &Manifest) -> TensorIndex {
        let mut index = TensorIndex::default();
        let h = m.hidden_dim;
        let f = m.expert_ffn_dim();
        let mut push = |name: String, shape: &[u32]| {
            index.entries.push(make_f32_entry(
                index.entries.len() as u32,
                &name,
                &format!("{name}.tensor"),
                0,
                shape,
            ));
        };
        for layer in 0..m.num_layers {
            let p = format!("L{layer:02}");
            push(format!("{p}.attn_q"), &[h, h]);
            push(format!("{p}.attn_k"), &[h, h]);
            push(format!("{p}.attn_v"), &[h, h]);
            push(format!("{p}.attn_output"), &[h, h]);
            push(format!("{p}.ffn_gate_inp"), &[m.num_experts, h]);
            for e in 0..m.num_experts {
                let ep = format!("{p}.E{e:02}");
                push(format!("{ep}.ffn_gate"), &[f, h]);
                push(format!("{ep}.ffn_up"), &[f, h]);
                push(format!("{ep}.ffn_down"), &[h, f]);
            }
        }
        index
    }

    #[test]
    fn moe_experts_allowed_when_vram_has_pool() {
        let manifest = Manifest::tiny_moe("moe");
        let index = moe_gpu_index(&manifest);
        let mem = MemSnapshot {
            total_frames: 100_000,
            free_frames: 50_000,
            reclaimable_frames: 0,
        };
        let planner = ResourcePlanner::new(&manifest, &index, mem, 16 * 1024 * 1024, false);
        assert!(planner.stats().gpu_layers > 0);
        assert!(planner.stats().gpu_expert_slots >= 1);
        assert!(planner.stats().gpu_expert_layers > 0);
        assert!(planner.gpu_tensor_allowed(0, "L00.attn_q"));
        assert!(planner.gpu_tensor_allowed(0, "L00.ffn_gate_inp"));
        assert!(planner.gpu_tensor_allowed(0, "L00.E01.ffn_up"));
        assert!(
            !planner.layer_plans[0]
                .gpu_tensors
                .iter()
                .any(|s| s.starts_with('E')),
            "los expertos van al pool, no pinneados por capa: {:?}",
            planner.layer_plans[0].gpu_tensors
        );
    }

    #[test]
    fn moe_experts_cpu_when_vram_only_covers_trunk() {
        let manifest = Manifest::tiny_moe("moe");
        let index = moe_gpu_index(&manifest);
        let mut trunk = 0u64;
        for layer in 0..manifest.num_layers {
            for suf in ["attn_q", "attn_k", "attn_v", "attn_output", "ffn_gate_inp"] {
                trunk += vram_bytes_for_named(&index, &format!("L{layer:02}.{suf}"));
            }
        }
        let one_ex = vram_bytes_for_expert(0, 0, &index);
        assert!(one_ex > 0 && trunk > 0);
        let mem = MemSnapshot {
            total_frames: 100_000,
            free_frames: 50_000,
            reclaimable_frames: 0,
        };
        let planner = ResourcePlanner::new(&manifest, &index, mem, trunk + 1, false);
        assert!(planner.stats().gpu_layers > 0);
        assert_eq!(planner.stats().gpu_expert_slots, 0);
        assert!(!planner.gpu_tensor_allowed(0, "L00.E00.ffn_up"));
        assert!(planner.gpu_tensor_allowed(0, "L00.attn_q"));
    }

    #[test]
    fn q4k_vram_cost_is_raw_not_f32_plane() {
        let e = sosomodel::index::make_q4_k_entry(0, "w", "w.tensor", 0, &[256, 256]);
        let vram = vram_bytes_for_entry(&e);
        assert_eq!(vram, e.byte_len);
        assert!(vram < (e.elems() as u64) * 4);
        assert_ne!(vram, e.byte_len.saturating_mul(8));
    }

    #[test]
    fn dense_ffn_gate_is_packed() {
        let manifest = Manifest::tiny("t");
        let mut index = TensorIndex::default();
        let h = manifest.hidden_dim;
        let f = manifest.ffn_dim;
        for layer in 0..manifest.num_layers {
            let p = format!("L{layer:02}");
            for (name, shape) in [
                (format!("{p}.attn_q"), [h, h]),
                (format!("{p}.ffn_gate"), [f, h]),
                (format!("{p}.ffn_up"), [f, h]),
                (format!("{p}.ffn_down"), [h, f]),
            ] {
                index.entries.push(make_f32_entry(
                    index.entries.len() as u32,
                    &name,
                    &format!("{name}.tensor"),
                    0,
                    &shape,
                ));
            }
        }
        let mem = MemSnapshot {
            total_frames: 100_000,
            free_frames: 50_000,
            reclaimable_frames: 0,
        };
        let planner = ResourcePlanner::new(&manifest, &index, mem, 8 * 1024 * 1024, false);
        let t = &planner.layer_plans[0].gpu_tensors;
        assert!(t.iter().any(|s| s == "ffn_gate"), "{t:?}");
        assert!(t.iter().any(|s| s == "ffn_up"), "{t:?}");
        assert!(planner.gpu_tensor_allowed(0, "L00.ffn_gate"));
        assert!(!planner.gpu_tensor_allowed(0, "L00.E00.ffn_up"));
    }

    #[test]
    fn io_bound_flag_sets_when_stage_wait_dominates() {
        let manifest = Manifest::tiny("t");
        let index = TensorIndex::default();
        let mut planner =
            ResourcePlanner::new(&manifest, &index, MemSnapshot::default(), 0, false);
        planner.observe_hotpath(1, 1);
        planner.note_stage_wait_ms(100);
        planner.note_stage_wait_ms(100);
        assert_eq!(planner.stats().io_bound, 1);
        assert!(planner.stats().avg_stage_wait_ms > planner.stats().avg_matvec_ms);
    }

    #[test]
    fn io_bound_prefers_remote() {
        let manifest = Manifest::tiny("t");
        let index = TensorIndex::default();
        let mem = MemSnapshot {
            free_frames: 1000,
            ..Default::default()
        };
        let mut planner = ResourcePlanner::new(&manifest, &index, mem, 0, true);
        planner.observe_hotpath(1, 1);
        for _ in 0..4 {
            planner.note_stage_wait_ms(200);
        }
        assert_eq!(planner.stats().io_bound, 1);
        planner.remote_rtt_ms = 50.0;
        planner.rebuild_plan(&manifest, &index);
        assert!(planner.layer_plans.iter().any(|p| p.dest == ExecDest::Remote));
    }

    #[test]
    fn huge_layer_forces_single_buffer() {
        let manifest = Manifest::tiny("t");
        let mut index = TensorIndex::default();
        let h = manifest.hidden_dim;
        // Capas muy grandes (~1 MiB cada una).
        for layer in 0..manifest.num_layers {
            let p = format!("L{layer:02}");
            index.entries.push(make_f32_entry(
                index.entries.len() as u32,
                &format!("{p}.attn_q"),
                &format!("{p}.attn_q.tensor"),
                0,
                &[h, 8192],
            ));
        }
        // RAM libre: ~1 MiB — no caben dos capas (~2 MiB).
        let mem = MemSnapshot {
            total_frames: 256,
            free_frames: 256,
            reclaimable_frames: 0,
        };
        let planner = ResourcePlanner::new(&manifest, &index, mem, 0, false);
        assert_eq!(planner.ring_slots(), 1);
        assert_eq!(planner.stats().prefetch_single_buffered, 1);
    }

    #[test]
    fn oversized_embed_is_gather_only() {
        let manifest = Manifest::tiny("t");
        let h = manifest.hidden_dim;
        let mut big_index = TensorIndex::default();
        big_index.entries.push(make_f32_entry(
            0,
            "embed",
            "embed.tensor",
            0,
            &[manifest.vocab_size, h * 4096],
        ));
        let mem = MemSnapshot {
            total_frames: 256,
            free_frames: 64,
            reclaimable_frames: 0,
        };
        let planner = ResourcePlanner::new(&manifest, &big_index, mem, 0, false);
        assert_eq!(planner.stats().embed_gather_only, 1);
        let keep = planner.keep_shards_after(0, manifest.num_layers, &manifest);
        assert!(!keep.iter().any(|s| s.contains("embed")));

        let mut small_index = TensorIndex::default();
        small_index.entries.push(make_f32_entry(
            0,
            "embed",
            "embed.tensor",
            0,
            &[manifest.vocab_size, h],
        ));
        let mem_big = MemSnapshot {
            total_frames: 100_000,
            free_frames: 50_000,
            reclaimable_frames: 0,
        };
        let planner_small = ResourcePlanner::new(&manifest, &small_index, mem_big, 0, false);
        assert_eq!(planner_small.stats().embed_gather_only, 0);
        let keep_small = planner_small.keep_shards_after(0, manifest.num_layers, &manifest);
        assert!(keep_small.iter().any(|s| s.contains("embed")));
    }
}
