//! sosofs v1: filesystem copy-on-write con checksums.
//!
//! Formato on-disk (bloques de 4 KiB, little-endian):
//! - Bloques 0 y 1: superbloques A/B. Lo único que se sobreescribe in situ,
//!   alternando; al montar gana el válido de mayor `generation`.
//! - Resto: área general CoW (nodos del árbol, extents de datos, bitmap).
//!
//! Toda la metainformación vive en UN árbol B+ de items de tamaño fijo,
//! con clave `(inode, kind, offset)`:
//! - `(ino, INODE, 0)`   -> tipo, tamaño, mtime
//! - `(dir, DIRENT, hash(nombre))` -> nombre e inode hijo
//! - `(ino, EXTENT, offset_en_bytes)` -> bloques de datos + crc32c
//!
//! Cada nodo del árbol lleva crc32c y el bloque donde espera estar; los
//! datos se verifican contra el crc de su extent. Nada se lee sin verificar.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod layout;

mod cache;
mod fs;
mod write;
pub use cache::CachedBlockDevice;
pub use fs::{FsError, Sosofs};

#[cfg(feature = "std")]
pub mod builder;

use crc::{CRC_32_ISCSI, Crc};

const CRC32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

pub fn crc32c(data: &[u8]) -> u32 {
    CRC32C.checksum(data)
}
