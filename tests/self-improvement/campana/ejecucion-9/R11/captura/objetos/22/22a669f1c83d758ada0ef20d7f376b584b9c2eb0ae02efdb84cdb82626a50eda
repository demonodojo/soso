//! Serialización y lookup del catálogo paginado.

use crate::layout::*;
use alloc::string::String;
use alloc::vec::Vec;
use block_dev::{BlockDevice, BlockError, BLOCK_SIZE};

pub fn hash_path(model: &str, path: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in model.bytes().chain(path.bytes()) {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

pub fn bucket_index(model: &str, path: &str, bucket_count: u32) -> u32 {
    hash_path(model, path) % bucket_count.max(1)
}

#[derive(Clone)]
pub struct Catalog {
    pub models: Vec<ModelEntry>,
}

impl Catalog {
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.models.len() as u32).to_le_bytes());
        for m in &self.models {
            let name = m.name.as_bytes();
            let nlen = name.len().min(NAME_MAX) as u8;
            out.push(nlen);
            out.extend_from_slice(&name[..nlen as usize]);
            out.resize(out.len() + NAME_MAX - nlen as usize, 0);
            out.extend_from_slice(&m.manifest_lba.to_le_bytes());
            out.extend_from_slice(&m.manifest_blocks.to_le_bytes());
            out.extend_from_slice(&m.index_lba.to_le_bytes());
            out.extend_from_slice(&m.index_blocks.to_le_bytes());
            out.extend_from_slice(&(m.shards.len() as u32).to_le_bytes());
            for s in &m.shards {
                write_shard(&mut out, s);
            }
        }
        out
    }

    pub fn parse(data: &[u8]) -> Result<Self, ()> {
        if data.len() < 4 {
            return Err(());
        }
        let nmodels = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let mut off = 4usize;
        let mut models = Vec::with_capacity(nmodels);
        for _ in 0..nmodels {
            let (m, used) = read_model(&data[off..])?;
            off += used;
            models.push(m);
        }
        Ok(Self { models })
    }

    pub fn find_model(&self, name: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.name == name)
    }

    pub fn find_shard<'a>(&'a self, model: &str, rel_path: &str) -> Option<&'a ShardEntry> {
        let m = self.find_model(model)?;
        m.shards.iter().find(|s| s.rel_path == rel_path)
    }
}

fn write_shard(out: &mut Vec<u8>, s: &ShardEntry) {
    let path = s.rel_path.as_bytes();
    let plen = path.len().min(PATH_MAX) as u8;
    out.push(plen);
    out.extend_from_slice(&path[..plen as usize]);
    out.resize(out.len() + PATH_MAX - plen as usize, 0);
    out.extend_from_slice(&s.byte_len.to_le_bytes());
    out.extend_from_slice(&(s.extents.len() as u16).to_le_bytes());
    for e in &s.extents {
        write_extent(out, e);
    }
    out.extend_from_slice(&s.prefetch_next_lba.to_le_bytes());
    out.extend_from_slice(&s.prefetch_bytes.to_le_bytes());
    out.push(s.cache_policy);
    out.push(s.align_requirement);
    out.extend_from_slice(&s.flags.to_le_bytes());
    out.extend_from_slice(&s.shard_crc32c.to_le_bytes());
}

fn write_extent(out: &mut Vec<u8>, e: &Extent) {
    out.extend_from_slice(&e.volume_id.to_le_bytes());
    out.extend_from_slice(&e.start_lba.to_le_bytes());
    out.extend_from_slice(&e.block_count.to_le_bytes());
    out.extend_from_slice(&e.segment_crc32c.to_le_bytes());
    out.extend_from_slice(&e.stripe_width.to_le_bytes());
    out.extend_from_slice(&e.stripe_index.to_le_bytes());
}

fn read_model(data: &[u8]) -> Result<(ModelEntry, usize), ()> {
    if data.len() < NAME_MAX + 32 {
        return Err(());
    }
    let nlen = data[0] as usize;
    let name = core::str::from_utf8(&data[1..1 + nlen.min(NAME_MAX)]).map_err(|_| ())?;
    let mut off = NAME_MAX + 1;
    let manifest_lba = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
    off += 8;
    let manifest_blocks = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
    off += 8;
    let index_lba = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
    off += 8;
    let index_blocks = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
    off += 8;
    let nshards = u32::from_le_bytes(data[off..off + 4].try_into().unwrap()) as usize;
    off += 4;
    let mut shards = Vec::with_capacity(nshards);
    for _ in 0..nshards {
        let (s, used) = read_shard(&data[off..])?;
        off += used;
        shards.push(s);
    }
    Ok((
        ModelEntry {
            name: String::from(name),
            manifest_lba,
            manifest_blocks,
            index_lba,
            index_blocks,
            shards,
        },
        off,
    ))
}

fn read_shard(data: &[u8]) -> Result<(ShardEntry, usize), ()> {
    if data.len() < PATH_MAX + 20 {
        return Err(());
    }
    let plen = data[0] as usize;
    let rel_path = core::str::from_utf8(&data[1..1 + plen.min(PATH_MAX)]).map_err(|_| ())?;
    let mut off = PATH_MAX + 1;
    let byte_len = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
    off += 8;
    let nextents = u16::from_le_bytes(data[off..off + 2].try_into().unwrap()) as usize;
    off += 2;
    let mut extents = Vec::with_capacity(nextents);
    for _ in 0..nextents {
        let e = read_extent(&data[off..])?;
        off += 28;
        extents.push(e);
    }
    let prefetch_next_lba = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
    off += 8;
    let prefetch_bytes = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
    off += 4;
    let cache_policy = data[off];
    let align_requirement = data[off + 1];
    let flags = u16::from_le_bytes(data[off + 2..off + 4].try_into().unwrap());
    off += 4;
    let shard_crc32c = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
    off += 4;
    Ok((
        ShardEntry {
            rel_path: String::from(rel_path),
            byte_len,
            extents,
            prefetch_next_lba,
            prefetch_bytes,
            cache_policy,
            align_requirement,
            flags,
            shard_crc32c,
        },
        off,
    ))
}

fn read_extent(data: &[u8]) -> Result<Extent, ()> {
    if data.len() < 28 {
        return Err(());
    }
    Ok(Extent {
        volume_id: u32::from_le_bytes(data[0..4].try_into().unwrap()),
        start_lba: u64::from_le_bytes(data[4..12].try_into().unwrap()),
        block_count: u64::from_le_bytes(data[12..20].try_into().unwrap()),
        segment_crc32c: u32::from_le_bytes(data[20..24].try_into().unwrap()),
        stripe_width: u16::from_le_bytes(data[24..26].try_into().unwrap()),
        stripe_index: u16::from_le_bytes(data[26..28].try_into().unwrap()),
    })
}

pub fn write_catalog_blocks<D: BlockDevice>(
    dev: &mut D,
    block: u64,
    data: &[u8],
) -> Result<u64, BlockError> {
    let nblocks = (data.len() + BLOCK_SIZE - 1) / BLOCK_SIZE;
    for (i, chunk) in data.chunks(BLOCK_SIZE).enumerate() {
        let mut buf = [0u8; BLOCK_SIZE];
        buf[..chunk.len()].copy_from_slice(chunk);
        dev.write_block(block + i as u64, &buf)?;
    }
    Ok(nblocks as u64)
}

pub fn read_catalog_bytes<D: BlockDevice>(
    dev: &mut D,
    root: u64,
    blocks: u64,
) -> Result<Vec<u8>, BlockError> {
    let mut data = Vec::with_capacity((blocks as usize) * BLOCK_SIZE);
    for i in 0..blocks {
        let mut buf = [0u8; BLOCK_SIZE];
        dev.read_block(root + i, &mut buf)?;
        data.extend_from_slice(&buf);
    }
    Ok(data)
}
