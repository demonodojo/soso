//! index.som: tabla tensor_id → shard, offset, shape, dtype.

use crate::layout::{DTYPE_F32, MAGIC, QUANT_NONE, SOM_HEADER_SIZE};
use crate::{align_up, crc32c, CACHE_ALIGN};
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
        let crc = crc32c(&body);
        let mut out = Vec::with_capacity(SOM_HEADER_SIZE + body.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(body.len() as u64).to_le_bytes());
        out.extend_from_slice(&body);
        let pad = align_up(out.len(), CACHE_ALIGN) - out.len();
        out.resize(out.len() + pad, 0);
        out
    }

    pub fn parse(data: &[u8]) -> Result<Self, ()> {
        if data.len() < SOM_HEADER_SIZE || data[..8] != MAGIC {
            return Err(());
        }
        let payload_len = u64::from_le_bytes(data[16..24].try_into().unwrap()) as usize;
        let body = &data[SOM_HEADER_SIZE..SOM_HEADER_SIZE + payload_len];
        if crc32c(body) != u32::from_le_bytes(data[8..12].try_into().unwrap()) {
            return Err(());
        }
        let n = u32::from_le_bytes(body[0..4].try_into().unwrap()) as usize;
        let mut off = 4usize;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let id = u32::from_le_bytes(body[off..off + 4].try_into().unwrap());
            off += 4;
            let nul = body[off..].iter().position(|&b| b == 0).ok_or(())?;
            let name = core::str::from_utf8(&body[off..off + nul]).map_err(|_| ())?;
            off += nul + 1;
            let nul2 = body[off..].iter().position(|&b| b == 0).ok_or(())?;
            let shard = core::str::from_utf8(&body[off..off + nul2]).map_err(|_| ())?;
            off += nul2 + 1;
            let offset = u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
            off += 8;
            let byte_len = u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
            off += 8;
            let ndim = u32::from_le_bytes(body[off..off + 4].try_into().unwrap()) as usize;
            off += 4;
            let mut shape = Vec::with_capacity(ndim);
            for _ in 0..ndim {
                shape.push(u32::from_le_bytes(body[off..off + 4].try_into().unwrap()));
                off += 4;
            }
            let dtype = body[off];
            let quant = body[off + 1];
            off += 4;
            entries.push(TensorEntry {
                id,
                name: String::from(name),
                shard: String::from(shard),
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

pub fn pack_shard(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::from(data);
    let crc = crc32c(data);
    out.extend_from_slice(&crc.to_le_bytes());
    let aligned = crate::align_up(out.len(), crate::BLOCK_ALIGN);
    out.resize(aligned, 0);
    out
}

pub fn verify_shard(data: &[u8]) -> Result<&[u8], ()> {
    if data.len() < 8 {
        return Err(());
    }
    let payload_end = data.len() - 4;
    let stored_crc = u32::from_le_bytes(data[payload_end..].try_into().unwrap());
    let payload = &data[..payload_end];
    if crc32c(payload) != stored_crc {
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
