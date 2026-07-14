//! Runtime de inferencia LLM para soso (no_std + std).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod gemm;
pub mod layer;
pub mod quant;
pub mod optim;
pub mod runtime;
pub mod tier;
pub mod attn;

pub use runtime::Runtime;
