//! Formato nativo `.som` para modelos LLM en soso.
//!
//! Layout optimizado para mmap y caché:
//! - Bloques alineados a 4 KiB (sosofs)
//! - Datos row-major con padding a 64 bytes (línea de caché)
//! - Shards de 8 MiB por defecto

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod layout;
pub mod manifest;
pub mod index;
pub mod validate;

pub use layout::*;
pub use manifest::Manifest;
pub use index::{TensorEntry, TensorIndex};

use crc::{CRC_32_ISCSI, Crc};

const CRC32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

pub fn crc32c(data: &[u8]) -> u32 {
    CRC32C.checksum(data)
}

/// Tamaño por defecto de un shard (8 MiB).
pub const DEFAULT_SHARD_SIZE: usize = 8 * 1024 * 1024;
/// Alineación de caché dentro del shard.
pub const CACHE_ALIGN: usize = 64;
/// Alineación de bloque sosofs.
pub const BLOCK_ALIGN: usize = 4096;

pub fn align_up(v: usize, align: usize) -> usize {
    (v + align - 1) & !(align - 1)
}
