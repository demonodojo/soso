//! Formato nativo `.som` para modelos LLM en soso.
//!
//! Layout optimizado para mmap y caché:
//! - Bloques alineados a 4 KiB (sosofs)
//! - Datos row-major con padding a 64 bytes (línea de caché)
//! - Shards de 8 MiB por defecto

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod dequant;
pub mod layout;
pub mod manifest;
pub mod index;
pub mod validate;

pub use layout::*;
pub use manifest::{AttnKind, FfnKind, LayerSpec, Manifest, UnsupportedLayer};
pub use index::{TensorEntry, TensorIndex};

use crc::{CRC_32_ISCSI, Crc};

const CRC32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

pub fn crc32c(data: &[u8]) -> u32 {
    CRC32C.checksum(data)
}

/// CRC32C incremental: para empaquetar shards grandes en streaming sin
/// materializar el payload completo en memoria.
pub struct Crc32cDigest(crc::Digest<'static, u32>);

impl Crc32cDigest {
    pub fn new() -> Self {
        Self(CRC32C.digest())
    }

    pub fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    pub fn finalize(self) -> u32 {
        self.0.finalize()
    }
}

impl Default for Crc32cDigest {
    fn default() -> Self {
        Self::new()
    }
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

/// Envuelve `body` en una cabecera SomHeader (magic, crc, versión, longitud)
/// y rellena con ceros hasta `align`.
pub fn pack_som(body: &[u8], version: u32, align: usize) -> alloc::vec::Vec<u8> {
    use layout::{MAGIC, SOM_HEADER_SIZE};
    let crc = crc32c(body);
    let mut out = alloc::vec::Vec::with_capacity(SOM_HEADER_SIZE + body.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&(body.len() as u64).to_le_bytes());
    out.extend_from_slice(body);
    let padded = align_up(out.len(), align);
    out.resize(padded, 0);
    out
}

/// Valida cabecera y CRC de un fichero .som y devuelve (versión, payload).
/// Nunca hace panic con entrada malformada o truncada.
pub fn parse_som(data: &[u8]) -> Result<(u32, &[u8]), ()> {
    use layout::{MAGIC, SOM_HEADER_SIZE};
    if data.len() < SOM_HEADER_SIZE || data[..8] != MAGIC {
        return Err(());
    }
    let crc = u32::from_le_bytes(data[8..12].try_into().map_err(|_| ())?);
    let version = u32::from_le_bytes(data[12..16].try_into().map_err(|_| ())?);
    let payload_len = u64::from_le_bytes(data[16..24].try_into().map_err(|_| ())?);
    let payload_len = usize::try_from(payload_len).map_err(|_| ())?;
    let end = SOM_HEADER_SIZE.checked_add(payload_len).ok_or(())?;
    let body = data.get(SOM_HEADER_SIZE..end).ok_or(())?;
    if crc32c(body) != crc {
        return Err(());
    }
    Ok((version, body))
}

/// Lector secuencial con bounds-check para los cuerpos .som.
pub struct Reader<'a> {
    data: &'a [u8],
    off: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, off: 0 }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], ()> {
        let end = self.off.checked_add(n).ok_or(())?;
        let s = self.data.get(self.off..end).ok_or(())?;
        self.off = end;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8, ()> {
        Ok(self.take(1)?[0])
    }

    pub fn u32(&mut self) -> Result<u32, ()> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| ())?))
    }

    pub fn u64(&mut self) -> Result<u64, ()> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| ())?))
    }

    pub fn f32(&mut self) -> Result<f32, ()> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().map_err(|_| ())?))
    }

    /// Cadena UTF-8 terminada en NUL.
    pub fn cstr(&mut self) -> Result<&'a str, ()> {
        let rest = self.data.get(self.off..).ok_or(())?;
        let nul = rest.iter().position(|&b| b == 0).ok_or(())?;
        let s = core::str::from_utf8(&rest[..nul]).map_err(|_| ())?;
        self.off += nul + 1;
        Ok(s)
    }
}
