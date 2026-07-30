//! Montaje, lectura y verificación de sosomfs.

use crate::cache::BlockCache;
use crate::catalog::Catalog;
use crate::layout::*;
use crate::volume_set::{SingleDev, VolumeSet};
use alloc::string::String;
use alloc::vec::Vec;
use block_dev::{BlockDevice, BlockError, BLOCK_SIZE};
use crc::{CRC_32_ISCSI, Crc};

const CRC32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    Io,
    Corrupt,
    NotFound,
    NotADir,
    NotAFile,
}

pub struct Sosomfs<V: VolumeSet> {
    pub cache: BlockCache<V>,
    pub sb: Superblock,
    pub catalog: Catalog,
    pub catalog_blocks: u64,
}

fn crc32c(data: &[u8]) -> u32 {
    CRC32C.checksum(data)
}

fn read_super_raw<V: VolumeSet>(vol: &mut V, slot: u64) -> Result<Superblock, ()> {
    let mut buf = [0u8; BLOCK_SIZE];
    vol.read_lba(slot, &mut buf).map_err(|_| ())?;
    if buf[..8] != MAGIC {
        return Err(());
    }
    let crc_stored = u32::from_le_bytes(buf[8..12].try_into().unwrap());
    let body = &buf[16..];
    if crc32c(body) != crc_stored {
        return Err(());
    }
    let generation = u64::from_le_bytes(buf[16..24].try_into().unwrap());
    let block_size = u32::from_le_bytes(buf[24..28].try_into().unwrap());
    let total_blocks = u64::from_le_bytes(buf[28..36].try_into().unwrap());
    let catalog_bucket_count = u32::from_le_bytes(buf[36..40].try_into().unwrap());
    let catalog_bucket_root = u64::from_le_bytes(buf[40..48].try_into().unwrap());
    let data_start = u64::from_le_bytes(buf[48..56].try_into().unwrap());
    let volume_count = u32::from_le_bytes(buf[56..60].try_into().unwrap());
    let catalog_blocks = u64::from_le_bytes(buf[60..68].try_into().unwrap());
    let mut volumes = [VolumeDesc {
        volume_id: 0,
        device_slot: 0,
        start_lba: 0,
        block_count: 0,
    }; 16];
    let mut off = 68usize;
    for i in 0..volume_count.min(16) as usize {
        if off + 24 > BLOCK_SIZE {
            break;
        }
        volumes[i] = VolumeDesc {
            volume_id: u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()),
            device_slot: u32::from_le_bytes(buf[off + 4..off + 8].try_into().unwrap()),
            start_lba: u64::from_le_bytes(buf[off + 8..off + 16].try_into().unwrap()),
            block_count: u64::from_le_bytes(buf[off + 16..off + 24].try_into().unwrap()),
        };
        off += 24;
    }
    Ok(Superblock {
        generation,
        block_size,
        total_blocks,
        catalog_bucket_count,
        catalog_bucket_root,
        data_start,
        volume_count,
        catalog_blocks,
        volumes,
    })
}

fn write_super_raw<D: BlockDevice>(dev: &mut D, slot: u64, sb: &Superblock) -> Result<(), BlockError> {
    let mut buf = [0u8; BLOCK_SIZE];
    buf[..8].copy_from_slice(&MAGIC);
    buf[16..24].copy_from_slice(&sb.generation.to_le_bytes());
    buf[24..28].copy_from_slice(&sb.block_size.to_le_bytes());
    buf[28..36].copy_from_slice(&sb.total_blocks.to_le_bytes());
    buf[36..40].copy_from_slice(&sb.catalog_bucket_count.to_le_bytes());
    buf[40..48].copy_from_slice(&sb.catalog_bucket_root.to_le_bytes());
    buf[48..56].copy_from_slice(&sb.data_start.to_le_bytes());
    buf[56..60].copy_from_slice(&sb.volume_count.to_le_bytes());
    buf[60..68].copy_from_slice(&sb.catalog_blocks.to_le_bytes());
    let mut off = 68usize;
    for i in 0..sb.volume_count.min(16) as usize {
        let v = &sb.volumes[i];
        buf[off..off + 4].copy_from_slice(&v.volume_id.to_le_bytes());
        buf[off + 4..off + 8].copy_from_slice(&v.device_slot.to_le_bytes());
        buf[off + 8..off + 16].copy_from_slice(&v.start_lba.to_le_bytes());
        buf[off + 16..off + 24].copy_from_slice(&v.block_count.to_le_bytes());
        off += 24;
    }
    let crc = crc32c(&buf[16..]);
    buf[8..12].copy_from_slice(&crc.to_le_bytes());
    dev.write_block(slot, &buf)
}

impl<V: VolumeSet> Sosomfs<V> {
    pub fn mount_with_cache(mut vol: V, cache_cap: usize) -> Result<Self, FsError> {
        let mut best: Option<Superblock> = None;
        for slot in 0..SUPERBLOCK_SLOTS {
            if let Ok(sb) = read_super_raw(&mut vol, slot) {
                if best.as_ref().is_none_or(|b| sb.generation > b.generation) {
                    best = Some(sb);
                }
            }
        }
        let sb = best.ok_or(FsError::Corrupt)?;
        let catalog_bytes = read_catalog_bytes(&mut vol, sb.catalog_bucket_root, sb.catalog_blocks)
        .map_err(|_| FsError::Io)?;
        let catalog = Catalog::parse(&catalog_bytes).map_err(|_| FsError::Corrupt)?;
        Ok(Self {
            cache: BlockCache::new(vol, cache_cap),
            sb,
            catalog,
            catalog_blocks: sb.catalog_blocks,
        })
    }

    pub fn mount(vol: V) -> Result<Self, FsError> {
        Self::mount_with_cache(vol, 2048)
    }

    pub fn total_blocks(&self) -> u64 {
        self.sb.total_blocks
    }

    pub fn generation(&self) -> u64 {
        self.sb.generation
    }

    fn parse_models_path(path: &str) -> Result<(&str, &str), FsError> {
        let path = path.trim_start_matches('/');
        let rest = path
            .strip_prefix("models")
            .ok_or(FsError::NotFound)?
            .strip_prefix('/')
            .unwrap_or("");
        if rest.is_empty() {
            return Err(FsError::NotADir);
        }
        match rest.split_once('/') {
            Some((model, sub)) if !model.is_empty() => Ok((model, sub)),
            None => Ok((rest, "")),
            _ => Err(FsError::NotFound),
        }
    }

    pub fn resolve(&self, path: &str) -> Result<(&ModelEntry, Option<&ShardEntry>), FsError> {
        if path == "/models" || path == "/models/" {
            return Err(FsError::NotADir);
        }
        let (model, sub) = Self::parse_models_path(path)?;
        let m = self.catalog.find_model(model).ok_or(FsError::NotFound)?;
        if sub.is_empty() {
            return Ok((m, None));
        }
        if sub == "manifest.som" {
            return Ok((m, m.shards.iter().find(|s| s.rel_path == "manifest.som")));
        }
        if sub == "index.som" {
            return Ok((m, m.shards.iter().find(|s| s.rel_path == "index.som")));
        }
        let shard = self.catalog.find_shard(model, sub).ok_or(FsError::NotFound)?;
        Ok((m, Some(shard)))
    }

    pub fn stat(&self, path: &str) -> Result<(u64, u8), FsError> {
        if path == "/models" || path == "/models/" {
            return Ok((0, 2));
        }
        let (_model, sub) = Self::parse_models_path(path)?;
        if sub.is_empty() {
            return Ok((0, 2));
        }
        if sub == "shards" || sub.ends_with('/') {
            return Ok((0, 2));
        }
        let (_, shard) = self.resolve(path)?;
        let s = shard.ok_or(FsError::NotFound)?;
        Ok((s.byte_len, 1))
    }

    pub fn read_dir(&self, path: &str) -> Result<Vec<(String, u8)>, FsError> {
        if path == "/models" || path == "/models/" {
            return Ok(self
                .catalog
                .models
                .iter()
                .map(|m| (m.name.clone(), 2))
                .collect());
        }
        let (model, sub) = Self::parse_models_path(path)?;
        if sub.is_empty() {
            let mut entries = Vec::new();
            entries.push((String::from("manifest.som"), 1));
            entries.push((String::from("index.som"), 1));
            if self.catalog.find_shard(model, "tokenizer.som").is_some() {
                entries.push((String::from("tokenizer.som"), 1));
            }
            entries.push((String::from("shards"), 2));
            return Ok(entries);
        }
        if sub == "shards" || sub == "shards/" {
            let m = self.catalog.find_model(model).ok_or(FsError::NotFound)?;
            return Ok(m
                .shards
                .iter()
                .filter(|s| s.rel_path.starts_with("shards/"))
                .map(|s| {
                    let name = s.rel_path.strip_prefix("shards/").unwrap_or(&s.rel_path);
                    (String::from(name), 1)
                })
                .collect());
        }
        Err(FsError::NotADir)
    }

    pub fn read_range(
        &mut self,
        shard: &ShardEntry,
        offset: usize,
        len: usize,
        out: &mut [u8],
    ) -> Result<(), FsError> {
        if offset + len > shard.byte_len as usize {
            return Err(FsError::Corrupt);
        }
        if shard.prefetch_next_lba != 0 {
            self.cache
                .prefetch(shard.prefetch_next_lba, shard.prefetch_bytes, CACHE_STREAM);
        }
        let mut remaining = len;
        let mut out_off = 0usize;
        let mut file_pos = 0usize;
        for ext in &shard.extents {
            let ext_bytes = (ext.block_count as usize) * BLOCK_SIZE;
            if offset >= file_pos + ext_bytes {
                file_pos += ext_bytes;
                continue;
            }
            let start_in_ext = offset.saturating_sub(file_pos);
            let mut lba_off = start_in_ext / BLOCK_SIZE;
            let mut in_block = start_in_ext % BLOCK_SIZE;
            while lba_off < ext.block_count as usize && remaining > 0 {
                let lba = ext.start_lba + lba_off as u64;
                let mut block = [0u8; BLOCK_SIZE];
                self.cache
                    .read_lba(lba, shard.cache_policy, &mut block)
                    .map_err(|_| FsError::Io)?;
                if ext.segment_crc32c != 0 && (lba_off * BLOCK_SIZE) % SEGMENT_SIZE == 0 {
                    let seg_off = lba_off * BLOCK_SIZE;
                    let seg_len = SEGMENT_SIZE.min(ext_bytes - seg_off);
                    let mut seg_buf = alloc::vec::Vec::with_capacity(seg_len);
                    seg_buf.extend_from_slice(&block);
                    let mut seg_lba = lba_off + 1;
                    while seg_buf.len() < seg_len && seg_lba < ext.block_count as usize {
                        let mut b2 = [0u8; BLOCK_SIZE];
                        self.cache
                            .read_lba(ext.start_lba + seg_lba as u64, shard.cache_policy, &mut b2)
                            .map_err(|_| FsError::Io)?;
                        let need = seg_len - seg_buf.len();
                        seg_buf.extend_from_slice(&b2[..need.min(BLOCK_SIZE)]);
                        seg_lba += 1;
                    }
                    if crc32c(&seg_buf[..seg_len]) != ext.segment_crc32c {
                        return Err(FsError::Corrupt);
                    }
                }
                let n = (BLOCK_SIZE - in_block).min(remaining);
                out[out_off..out_off + n].copy_from_slice(&block[in_block..in_block + n]);
                out_off += n;
                remaining -= n;
                in_block = 0;
                lba_off += 1;
            }
            if remaining == 0 {
                return Ok(());
            }
            file_pos += ext_bytes;
        }
        Err(FsError::Corrupt)
    }

    /// Lectura directa del volumen, sin pasar por la caché de bloques: para
    /// rangos grandes alineados a bloque (los faults de 2 MiB del kernel).
    /// No verifica el CRC de segmento — la integridad del payload la valida
    /// `verify_shard` en el consumidor.
    pub fn read_range_direct(
        &mut self,
        shard: &ShardEntry,
        offset: usize,
        len: usize,
        out: &mut [u8],
    ) -> Result<(), FsError> {
        if offset % BLOCK_SIZE != 0 || len % BLOCK_SIZE != 0 || out.len() < len {
            return Err(FsError::Corrupt);
        }
        let end = offset + len;
        let mut cur = offset;
        let mut out_off = 0usize;
        let mut file_pos = 0usize;
        for ext in &shard.extents {
            let ext_end = file_pos + (ext.block_count as usize) * BLOCK_SIZE;
            while cur < end && cur >= file_pos && cur < ext_end {
                let lba = ext.start_lba + ((cur - file_pos) / BLOCK_SIZE) as u64;
                let dst: &mut [u8; BLOCK_SIZE] = (&mut out[out_off..out_off + BLOCK_SIZE])
                    .try_into()
                    .map_err(|_| FsError::Io)?;
                self.cache
                    .volume_mut()
                    .read_lba(lba, dst)
                    .map_err(|_| FsError::Io)?;
                cur += BLOCK_SIZE;
                out_off += BLOCK_SIZE;
            }
            file_pos = ext_end;
            if cur >= end {
                return Ok(());
            }
        }
        Err(FsError::Corrupt)
    }

    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>, FsError> {
        let shard = self.shard_for_path(path)?;
        let mut out = alloc::vec![0u8; shard.byte_len as usize];
        self.read_range(&shard, 0, shard.byte_len as usize, &mut out)?;
        Ok(out)
    }

    pub fn shard_for_path(&self, path: &str) -> Result<ShardEntry, FsError> {
        let (_, shard) = self.resolve(path)?;
        shard.cloned().ok_or(FsError::NotFound)
    }

    pub fn cache_policy_for(&self, path: &str) -> u8 {
        if let Ok((_, Some(s))) = self.resolve(path) {
            s.cache_policy
        } else if path.ends_with("manifest.som") || path.ends_with("index.som") {
            CACHE_PIN
        } else {
            CACHE_NORMAL
        }
    }
}

pub fn mount<D: BlockDevice>(dev: D) -> Result<Sosomfs<SingleDev<D>>, FsError> {
    Sosomfs::mount(SingleDev::new(dev))
}


fn read_catalog_bytes<V: VolumeSet>(
    vol: &mut V,
    root: u64,
    blocks: u64,
) -> Result<Vec<u8>, BlockError> {
    let mut data = Vec::with_capacity((blocks as usize) * BLOCK_SIZE);
    for i in 0..blocks {
        let mut buf = [0u8; BLOCK_SIZE];
        vol.read_lba(root + i, &mut buf)?;
        data.extend_from_slice(&buf);
    }
    Ok(data)
}

pub fn write_image<D: BlockDevice>(dev: &mut D, catalog: &Catalog, sb: &Superblock) -> Result<(), BlockError> {
    let cat_bytes = catalog.serialize();
    let cat_blocks = (cat_bytes.len() + BLOCK_SIZE - 1) / BLOCK_SIZE;
    let mut sb = *sb;
    sb.catalog_blocks = cat_blocks as u64;
    crate::catalog::write_catalog_blocks(dev, sb.catalog_bucket_root, &cat_bytes)?;
    let slot = sb.generation % SUPERBLOCK_SLOTS;
    write_super_raw(dev, slot, &sb)?;
    let other = 1 - slot;
    write_super_raw(dev, other, &sb)?;
    dev.flush()
}
