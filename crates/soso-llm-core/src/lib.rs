//! Runtime de inferencia LLM para soso (no_std + std).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod f16;
pub mod gemm;
pub mod gpu;
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
pub mod attn;
pub mod source;
pub mod tokenizer;

pub use runtime::Runtime;
pub use attn::{prompt_lookup_draft, prompt_lookup_draft_hinted};
pub use kv::{KvDtype, LayerKv};
pub use plan::{ExecDest, MemSnapshot, PlannerStats, ResourcePlanner};
pub use source::{FileMapper, MappedShard, MmapTensorSource};
