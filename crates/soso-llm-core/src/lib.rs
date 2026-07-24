//! Runtime de inferencia LLM para soso (no_std + std).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod f16;
pub mod gemm;
pub mod gpu;
pub mod layer;
pub mod parallel;
pub mod quant;
pub mod optim;
pub mod pipeline;
pub mod runtime;
pub mod sample;
pub mod tier;
pub mod attn;
pub mod source;
pub mod tokenizer;

pub use runtime::Runtime;
pub use source::{FileMapper, MappedShard, MmapTensorSource};
