//! Runtime de inferencia LLM para soso (no_std + std).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod chat;
/// Tipos de conversación compartidos (T04): dominio sin HTTP ni syscalls.
pub mod conversation;
pub mod especulativo;
pub mod generation;
pub mod f16;
pub use sosomodel::iq;
pub use sosomodel::iq_tablas;
pub mod iq_dot;
pub mod kev;
pub mod gemm;
pub mod gemma;
pub mod gpu;
pub mod moe_gpu;
pub mod sched;
pub mod kv;
pub mod layer;
pub mod parallel;
pub mod quant;
pub mod restriccion;
pub mod q8k;
pub mod optim;
pub mod pipeline;
pub mod cluster;
pub mod remoto;
pub mod plan;
pub mod runtime;
pub mod sample;
pub mod tier;
pub mod stage;
pub mod arch;
pub mod bloque35;
pub mod asr;
pub mod attn;
pub mod attn_gemma;
pub mod attn_grupo;
pub(crate) mod simd_math;
pub mod source;
pub mod tokenizer;

pub use generation::{
    GenCancel, GenCheckpoint, GenerationLedger, GenerationObserver, GenerationOptions,
    GenerationReport, LegacyStreamObserver, NoCancelObserver, StopReason, stops_from_eos,
};
pub use restriccion::{Restriccion, TablaBytes};
pub use remoto::{prefix_hash_tokens, ClusterGen, JournalActivaciones, TramoRemoto};
pub use runtime::Runtime;
pub use asr::{AsrPhase, AsrProfile, AsrTokenProfile};
pub use sched::{CostModel, Dest, OpDesc, OpSched};
pub use attn::{prompt_lookup_draft, prompt_lookup_draft_hinted};
pub use kv::{KvDtype, LayerKv};
pub use layer::{clear_infer_op, last_infer_op, note_infer_op};
pub use plan::{
    classify_weight_bytes, compute_trunk_first_split, is_shared_expert_tensor,
    shared_expert_shard_names, ExecDest, MemoryPlanConfig, MemoryPreset,
    MemSnapshot, PlannerStats, ResourcePlanner, WeightClassBytes,
};
pub use source::{FileMapper, MappedShard, MmapTensorSource};
#[cfg(feature = "std")]
pub use source::host::ThreadStagedSource;
pub use stage::{PrefetchSink, SyncStager};
