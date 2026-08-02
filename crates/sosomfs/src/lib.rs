//! sosomfs: sistema de ficheros de solo lectura para modelos LLM.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod cache;
pub mod catalog;
pub mod fs;
pub mod layout;
pub mod volume_set;

#[cfg(feature = "std")]
pub mod builder;

pub use cache::BlockCache;
pub use catalog::Catalog;
pub use fs::{mount, FsError, Sosomfs, MAX_REQ_BLOCKS};
pub use layout::*;
pub use volume_set::{SingleDev, VolumeSet};

#[cfg(feature = "std")]
pub use builder::{build_from_dir, build_from_dirs, parse_size, BuildReport};
