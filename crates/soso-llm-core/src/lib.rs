//! Runtime de inferencia LLM para soso (no_std + std).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod chat;
pub mod f16;
pub mod gemm;
pub mod gpu;
pub mod sched;
pub mod kv;
pub mod layer;
pub mod parallel;
pub mod quant;
pub mod optim;
pub mod pipeline;
pub mod plan;
pub mod runtime;
pub mod sample;
pub mod tier;
pub mod stage;
pub mod arch;
pub mod asr;
pub mod attn;
pub mod source;
pub mod tokenizer;

pub use runtime::Runtime;
pub use asr::{AsrPhase, AsrProfile, AsrTokenProfile};
pub use sched::{CostModel, Dest, OpDesc, OpSched};
pub use attn::{prompt_lookup_draft, prompt_lookup_draft_hinted};
pub use kv::{KvDtype, LayerKv};
pub use plan::{
    classify_weight_bytes, compute_trunk_first_split, is_shared_expert_tensor,
    shared_expert_shard_names, ExecDest, MemoryPlanConfig, MemoryPreset,
    MemSnapshot, PlannerStats, ResourcePlanner, WeightClassBytes,
};
pub use source::{FileMapper, MappedShard, MmapTensorSource};
#[cfg(feature = "std")]
pub use source::host::ThreadStagedSource;
pub use stage::{PrefetchSink, SyncStager};
