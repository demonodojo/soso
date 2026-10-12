//! index.som: tabla tensor_id → shard, offset, shape, dtype.

use crate::layout::{
    DTYPE_F32, DTYPE_MXFP4, DTYPE_Q4_K, DTYPE_Q8_0, MXFP4_BLOCK_BYTES, MXFP4_BLOCK_ELEMS,
    Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS, Q8_0_BLOCK_BYTES, Q8_0_BLOCK_ELEMS, QUANT_MXFP4,
    QUANT_NONE, QUANT_Q4_K, QUANT_Q8_0,
};
use crate::{pack_som, parse_som, Reader, BLOCK_ALIGN, CACHE_ALIGN};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
pub struct TensorEntry {
    pub id: u32,
    pub name: String,
    pub shard: String,
    pub offset: u64,
    pub byte_len: u64,
    pub shape: Vec<u32>,
    pub dtype: u8,
    pub quant: u8,
}

impl TensorEntry {
    /// Número de elementos f32 lógicos del tensor.
    pub fn elems(&self) -> usize {
        self.shape.iter().map(|&d| d as usize).product()
    }
}

#[derive(Clone, Debug, Default)]
pub struct TensorIndex {
    pub entries: Vec<TensorEntry>,
}

impl TensorIndex {
    pub fn serialize(&self) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for e in &self.entries {
            body.extend_from_slice(&e.id.to_le_bytes());
            body.extend_from_slice(e.name.as_bytes());
            body.push(0);
            body.extend_from_slice(e.shard.as_bytes());
            body.push(0);
            body.extend_from_slice(&e.offset.to_le_bytes());
            body.extend_from_slice(&e.byte_len.to_le_bytes());
            body.extend_from_slice(&(e.shape.len() as u32).to_le_bytes());
            for &d in &e.shape {
                body.extend_from_slice(&d.to_le_bytes());
            }
            body.push(e.dtype);
            body.push(e.quant);
            body.push(0);
            body.push(0);
        }
        pack_som(&body, 1, CACHE_ALIGN)
    }

    pub fn parse(data: &[u8]) -> Result<Self, ()> {
        let (_version, body) = parse_som(data)?;
        let mut r = Reader::new(body);
        let n = r.u32()? as usize;
        let mut entries = Vec::new();
        for _ in 0..n {
            let id = r.u32()?;
            let name = String::from(r.cstr()?);
            let shard = String::from(r.cstr()?);
            let offset = r.u64()?;
            let byte_len = r.u64()?;
            let ndim = r.u32()? as usize;
            let mut shape = Vec::new();
            for _ in 0..ndim {
                shape.push(r.u32()?);
            }
            let dtype = r.u8()?;
            let quant = r.u8()?;
            r.take(2)?;
            entries.push(TensorEntry {
                id,
                name,
                shard,
                offset,
                byte_len,
                shape,
                dtype,
                quant,
            });
        }
        Ok(Self { entries })
    }

    pub fn find(&self, name: &str) -> Option<&TensorEntry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

/// Offset del payload en un shard v2: el payload queda alineado a 64 B
/// dentro del fichero (y por tanto en el mmap, que es page-aligned), para
/// poder leer los pesos como vistas `&[f32]`/SIMD sin copiar.
pub const SHARD_PAYLOAD_OFF: usize = crate::CACHE_ALIGN;

/// Empaqueta el payload de un shard: cabecera SomHeader (CRC + longitud
/// explícita) + relleno hasta 64 B + payload + relleno hasta BLOCK_ALIGN.
pub fn pack_shard(data: &[u8]) -> Vec<u8> {
    use crate::layout::MAGIC;
    let crc = crate::crc32c(data);
    let mut out = Vec::with_capacity(SHARD_PAYLOAD_OFF + data.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.resize(SHARD_PAYLOAD_OFF, 0);
    out.extend_from_slice(data);
    let aligned = crate::align_up(out.len(), BLOCK_ALIGN);
    out.resize(aligned, 0);
    out
}

/// Verifica cabecera y CRC de un shard y devuelve el payload delimitado por
/// `payload_len`. v1: payload tras la cabecera (24 B); v2: en el offset 64.
pub fn verify_shard(data: &[u8]) -> Result<&[u8], ()> {
    use crate::layout::{MAGIC, SOM_HEADER_SIZE};
    if data.len() < SOM_HEADER_SIZE || data[..8] != MAGIC {
        return Err(());
    }
    let crc = u32::from_le_bytes(data[8..12].try_into().map_err(|_| ())?);
    let version = u32::from_le_bytes(data[12..16].try_into().map_err(|_| ())?);
    let payload_len = u64::from_le_bytes(data[16..24].try_into().map_err(|_| ())?);
    let payload_len = usize::try_from(payload_len).map_err(|_| ())?;
    let off = if version >= 2 { SHARD_PAYLOAD_OFF } else { SOM_HEADER_SIZE };
    let end = off.checked_add(payload_len).ok_or(())?;
    let payload = data.get(off..end).ok_or(())?;
    if crate::crc32c(payload) != crc {
        return Err(());
    }
    Ok(payload)
}

pub fn make_f32_entry(id: u32, name: &str, shard: &str, offset: u64, shape: &[u32]) -> TensorEntry {
    let elems: u64 = shape.iter().map(|&d| d as u64).product();
    TensorEntry {
        id,
        name: String::from(name),
        shard: String::from(shard),
        offset,
        byte_len: elems * 4,
        shape: shape.to_vec(),
        dtype: DTYPE_F32,
        quant: QUANT_NONE,
    }
}

/// Entrada Q4_K: superbloques GGML de 256 elementos (144 bytes). El número
/// de elementos debe ser múltiplo del superbloque.
pub fn make_q4_k_entry(
    id: u32,
    name: &str,
    shard: &str,
    offset: u64,
    shape: &[u32],
) -> TensorEntry {
    let elems: u64 = shape.iter().map(|&d| d as u64).product();
    let blocks = elems.div_ceil(Q4_K_BLOCK_ELEMS as u64);
    TensorEntry {
        id,
        name: String::from(name),
        shard: String::from(shard),
        offset,
        byte_len: blocks * Q4_K_BLOCK_BYTES as u64,
        shape: shape.to_vec(),
        dtype: DTYPE_Q4_K,
        quant: QUANT_Q4_K,
    }
}

/// Entrada Q8_0: bloques de 32 elementos (escala f32 + 32 i8 = 36 bytes).
/// El número de elementos debe ser múltiplo del tamaño de bloque.
pub fn make_q8_0_entry(
    id: u32,
    name: &str,
    shard: &str,
    offset: u64,
    shape: &[u32],
) -> TensorEntry {
    let elems: u64 = shape.iter().map(|&d| d as u64).product();
    let blocks = elems.div_ceil(Q8_0_BLOCK_ELEMS as u64);
    TensorEntry {
        id,
        name: String::from(name),
        shard: String::from(shard),
        offset,
        byte_len: blocks * Q8_0_BLOCK_BYTES as u64,
        shape: shape.to_vec(),
        dtype: DTYPE_Q8_0,
        quant: QUANT_Q8_0,
    }
}

/// Entrada MXFP4: bloques de 32 elementos (escala f32 + 16 bytes nibbles).
pub fn make_mxfp4_entry(
    id: u32,
    name: &str,
    shard: &str,
    offset: u64,
    shape: &[u32],
) -> TensorEntry {
    let elems: u64 = shape.iter().map(|&d| d as u64).product();
    let blocks = elems.div_ceil(MXFP4_BLOCK_ELEMS as u64);
    TensorEntry {
        id,
        name: String::from(name),
        shard: String::from(shard),
        offset,
        byte_len: blocks * MXFP4_BLOCK_BYTES as u64,
        shape: shape.to_vec(),
        dtype: DTYPE_MXFP4,
        quant: QUANT_MXFP4,
    }
}
