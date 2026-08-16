//! Importación atómica de modelos en un volumen sosomfs montado (no_std).

use crate::catalog::Catalog;
use crate::fs::{write_image, write_super_raw, FsError};
use crate::layout::*;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use block_dev::{BlockDevice, BlockError, BLOCK_SIZE};
use crc::{CRC_32_ISCSI, Crc};
use sosomodel::index::TensorIndex;
use sosomodel::layout::{INDEX_FILE, MANIFEST_FILE, SHARDS_DIR};
use sosomodel::manifest::Manifest;

const CRC32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

/// Bloques reservados para el catálogo (LBA 2..=65).
pub const CATALOG_RESERVED_BLOCKS: u64 = 64;

fn crc32c(data: &[u8]) -> u32 {
    CRC32C.checksum(data)
}

fn align_bytes(len: usize, align: u8) -> usize {
    let a = match align {
        ALIGN_HUGE_2M => 2 * 1024 * 1024,
        ALIGN_GPU_DMA_64K => 64 * 1024,
        ALIGN_PAGE_4K => 4096,
        _ => 64,
    };
    (len + a - 1) & !(a - 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportError {
    Io,
    Exists,
    InProgress,
    NoSession,
    NoSpace,
    CatalogTooLarge,
    InvalidName,
    InvalidPath,
    MissingManifest,
    MissingIndex,
    InvalidManifest,
}

impl ImportError {
    pub fn to_errno(self) -> i64 {
        match self {
            ImportError::Io => -5, // EIO
            ImportError::Exists => -17, // EEXIST
            ImportError::InProgress => -16, // EBUSY
            ImportError::NoSession => -22, // EINVAL
            ImportError::NoSpace => -28, // ENOSPC
            ImportError::CatalogTooLarge => -27, // EFBIG
            ImportError::InvalidName | ImportError::InvalidPath | ImportError::MissingManifest
            | ImportError::MissingIndex | ImportError::InvalidManifest => -22,
        }
    }
}

/// Primer LBA libre tras los datos ya asignados.
pub fn next_free_lba(sb: &Superblock, catalog: &Catalog) -> u64 {
    let mut free = sb.data_start;
    for m in &catalog.models {
        for s in &m.shards {
            for e in &s.extents {
                let end = e.start_lba.saturating_add(e.block_count);
                if end > free {
                    free = end;
                }
            }
        }
    }
    free
}

/// Amplía `total_blocks` si la partición es mayor (sin subir generación).
pub fn grow_superblock(sb: &mut Superblock, partition_blocks: u64) {
    if partition_blocks > sb.total_blocks {
        sb.total_blocks = partition_blocks;
        if sb.volume_count > 0 {
            sb.volumes[0].block_count = partition_blocks;
        }
    }
}

/// Persiste el superbloque ampliado (misma generación).
pub fn commit_grow<D: BlockDevice>(dev: &mut D, sb: &Superblock) -> Result<(), BlockError> {
    let slot = sb.generation % SUPERBLOCK_SLOTS;
    write_super_raw(dev, slot, sb)?;
    write_super_raw(dev, 1 - slot, sb)?;
    dev.flush()
}

fn cache_policy_for(rel: &str) -> u8 {
    if rel == MANIFEST_FILE || rel == INDEX_FILE {
        CACHE_PIN
    } else if rel.starts_with(SHARDS_DIR) && rel.ends_with(".tensor") {
        CACHE_STREAM
    } else {
        CACHE_NORMAL
    }
}

fn align_for(len: usize, cache_policy: u8) -> u8 {
    if len >= 2 * 1024 * 1024 {
        ALIGN_HUGE_2M
    } else if cache_policy == CACHE_STREAM {
        ALIGN_GPU_DMA_64K
    } else {
        ALIGN_PAGE_4K
    }
}

fn flags_for_shard(index: &TensorIndex, rel: &str) -> u16 {
    let shard_name = rel.strip_prefix(&format!("{SHARDS_DIR}/")).unwrap_or(rel);
    let mut flags = 0u16;
    if let Some(ent) = index.entries.iter().find(|e| e.shard == shard_name) {
        if ent.quant == sosomodel::layout::QUANT_Q4_K {
            flags |= FLAG_TILE_Q4;
        } else if ent.quant == sosomodel::layout::QUANT_Q8_0 {
            flags |= FLAG_TILE_Q8;
        }
    }
    flags
}

fn write_payload<D: BlockDevice>(dev: &mut D, start: u64, data: &[u8]) -> Result<(), BlockError> {
    for (i, chunk) in data.chunks(BLOCK_SIZE).enumerate() {
        let mut buf = [0u8; BLOCK_SIZE];
        buf[..chunk.len()].copy_from_slice(chunk);
        dev.write_block(start + i as u64, &buf)?;
    }
    Ok(())
}

fn write_shard_file<D: BlockDevice>(
    dev: &mut D,
    rel_path: &str,
    data: &[u8],
    cache_policy: u8,
    align_requirement: u8,
    flags: u16,
    prefetch_bytes: u32,
    start_lba: u64,
) -> Result<ShardEntry, BlockError> {
    let padded_len = align_bytes(data.len(), align_requirement);
    let mut seg_buf = vec![0u8; SEGMENT_SIZE];
    let mut shard_crc = sosomodel::Crc32cDigest::new();
    let mut extents = Vec::new();
    let mut off = 0usize;
    while off < padded_len {
        let seg_len = SEGMENT_SIZE.min(padded_len - off);
        let buf = &mut seg_buf[..seg_len];
        buf.fill(0);
        let avail = data.len().saturating_sub(off).min(seg_len);
        if avail > 0 {
            buf[..avail].copy_from_slice(&data[off..off + avail]);
        }
        let seg_crc = crc32c(buf);
        shard_crc.update(buf);
        write_payload(dev, start_lba + (off / BLOCK_SIZE) as u64, buf)?;
        extents.push(Extent {
            volume_id: 0,
            start_lba: start_lba + (off / BLOCK_SIZE) as u64,
            block_count: ((seg_len + BLOCK_SIZE - 1) / BLOCK_SIZE) as u64,
            segment_crc32c: seg_crc,
            stripe_width: 0,
            stripe_index: 0,
        });
        off += seg_len;
    }
    Ok(ShardEntry {
        rel_path: String::from(rel_path),
        byte_len: padded_len as u64,
        extents,
        prefetch_next_lba: 0,
        prefetch_bytes,
        cache_policy,
        align_requirement,
        flags,
        shard_crc32c: shard_crc.finalize(),
    })
}

/// Reserva bloques al final del volumen (crece hacia abajo).
#[derive(Clone, Copy, Debug)]
pub struct ScratchRegion {
    pub start_lba: u64,
    pub blocks: u64,
}

pub fn scratch_alloc(
    scratch_floor: &mut u64,
    size_bytes: u64,
) -> Result<ScratchRegion, ImportError> {
    if size_bytes == 0 {
        return Err(ImportError::InvalidPath);
    }
    let blocks = size_bytes.div_ceil(BLOCK_SIZE as u64);
    if blocks > *scratch_floor {
        return Err(ImportError::NoSpace);
    }
    *scratch_floor -= blocks;
    Ok(ScratchRegion {
        start_lba: *scratch_floor,
        blocks,
    })
}

pub fn scratch_write<D: BlockDevice>(
    dev: &mut D,
    region: ScratchRegion,
    offset: u64,
    data: &[u8],
) -> Result<(), ImportError> {
    if offset + data.len() as u64 > region.blocks * BLOCK_SIZE as u64 {
        return Err(ImportError::InvalidPath);
    }
    let start = region.start_lba + offset / BLOCK_SIZE as u64;
    let skip = (offset % BLOCK_SIZE as u64) as usize;
    if skip != 0 {
        return Err(ImportError::InvalidPath);
    }
    write_payload(dev, start, data).map_err(|_| ImportError::Io)
}

pub fn scratch_read<D: BlockDevice>(
    dev: &mut D,
    region: ScratchRegion,
    offset: u64,
    out: &mut [u8],
) -> Result<(), ImportError> {
    if offset + out.len() as u64 > region.blocks * BLOCK_SIZE as u64 {
        return Err(ImportError::InvalidPath);
    }
    if offset % BLOCK_SIZE as u64 != 0 || out.len() % BLOCK_SIZE != 0 {
        return Err(ImportError::InvalidPath);
    }
    let mut lba = region.start_lba + offset / BLOCK_SIZE as u64;
    for chunk in out.chunks_mut(BLOCK_SIZE) {
        let mut block = [0u8; BLOCK_SIZE];
        dev.read_block(lba, &mut block).map_err(|_| ImportError::Io)?;
        chunk.copy_from_slice(&block);
        lba += 1;
    }
    Ok(())
}

/// Sesión de import: escribe shards y commitea catálogo + generación.
pub struct ImportSession {
    model_name: String,
    sb: Superblock,
    catalog: Catalog,
    next_lba: u64,
    scratch_floor: u64,
    shards: Vec<ShardEntry>,
    lba_by_path: BTreeMap<String, u64>,
    manifest_bytes: Option<Vec<u8>>,
    index_bytes: Option<Vec<u8>>,
    scratch: Option<ScratchRegion>,
}

impl ImportSession {
    pub fn begin(
        sb: Superblock,
        catalog: Catalog,
        name: &str,
    ) -> Result<Self, ImportError> {
        if name.is_empty() || name.len() > NAME_MAX {
            return Err(ImportError::InvalidName);
        }
        if name.contains('/') {
            return Err(ImportError::InvalidName);
        }
        if catalog.find_model(name).is_some() {
            return Err(ImportError::Exists);
        }
        let next_lba = next_free_lba(&sb, &catalog);
        let scratch_floor = sb.total_blocks;
        Ok(Self {
            model_name: String::from(name),
            sb,
            catalog,
            next_lba,
            scratch_floor,
            shards: Vec::new(),
            lba_by_path: BTreeMap::new(),
            manifest_bytes: None,
            index_bytes: None,
            scratch: None,
        })
    }

    pub fn scratch_floor(&self) -> u64 {
        self.scratch_floor
    }

    pub fn scratch_alloc(&mut self, size_bytes: u64) -> Result<ScratchRegion, ImportError> {
        if self.scratch.is_some() {
            return Err(ImportError::InProgress);
        }
        let region = scratch_alloc(&mut self.scratch_floor, size_bytes)?;
        if self.next_lba > self.scratch_floor {
            return Err(ImportError::NoSpace);
        }
        self.scratch = Some(region);
        Ok(region)
    }

    pub fn scratch_free(&mut self) {
        if let Some(r) = self.scratch.take() {
            self.scratch_floor = r.start_lba + r.blocks;
        }
    }

    pub fn scratch_region(&self) -> Option<ScratchRegion> {
        self.scratch
    }

    pub fn put_file<D: BlockDevice>(
        &mut self,
        dev: &mut D,
        rel_path: &str,
        data: &[u8],
    ) -> Result<(), ImportError> {
        if rel_path.is_empty() || rel_path.contains("..") {
            return Err(ImportError::InvalidPath);
        }
        let cache_policy = cache_policy_for(rel_path);
        let align_requirement = align_for(data.len(), cache_policy);
        let padded_len = align_bytes(data.len(), align_requirement);
        let blocks = (padded_len + BLOCK_SIZE - 1) / BLOCK_SIZE;
        if self.next_lba + blocks as u64 > self.scratch_floor {
            return Err(ImportError::NoSpace);
        }
        let start_lba = self.next_lba;
        let flags = if let Some(ref idx) = self.index_bytes {
            if let Ok(index) = TensorIndex::parse(idx) {
                flags_for_shard(&index, rel_path)
            } else {
                0
            }
        } else {
            0
        };
        let prefetch_bytes = 0u32;
        let shard = write_shard_file(
            dev,
            rel_path,
            data,
            cache_policy,
            align_requirement,
            flags,
            prefetch_bytes,
            start_lba,
        )
        .map_err(|_| ImportError::Io)?;
        self.lba_by_path.insert(String::from(rel_path), start_lba);
        if rel_path == MANIFEST_FILE {
            self.manifest_bytes = Some(data.to_vec());
        } else if rel_path == INDEX_FILE {
            self.index_bytes = Some(data.to_vec());
        }
        self.next_lba += blocks as u64;
        self.shards.push(shard);
        Ok(())
    }

    fn apply_prefetch(&mut self, manifest: &Manifest) {
        let tam = |rel: &str| -> u32 {
            self.shards
                .iter()
                .find(|s| s.rel_path == rel)
                .map(|s| s.byte_len.min(u32::MAX as u64) as u32)
                .unwrap_or(0)
        };
        let mut prefetch_map: BTreeMap<String, (String, u32)> = BTreeMap::new();
        for pf in &manifest.prefetch {
            let chain: Vec<String> = pf
                .shards
                .iter()
                .map(|s| format!("{SHARDS_DIR}/{s}"))
                .collect();
            for i in 0..chain.len().saturating_sub(1) {
                let bytes = tam(&chain[i + 1]);
                if bytes == 0 {
                    continue;
                }
                prefetch_map.insert(chain[i].clone(), (chain[i + 1].clone(), bytes));
            }
        }
        for shard in &mut self.shards {
            if let Some((next_path, bytes)) = prefetch_map.get(&shard.rel_path) {
                shard.prefetch_next_lba = self.lba_by_path.get(next_path).copied().unwrap_or(0);
                shard.prefetch_bytes = *bytes;
            }
        }
    }

    fn refresh_flags_from_index(&mut self) {
        let Some(ref idx) = self.index_bytes else {
            return;
        };
        let Ok(index) = TensorIndex::parse(idx) else {
            return;
        };
        for shard in &mut self.shards {
            shard.flags = flags_for_shard(&index, &shard.rel_path);
        }
    }

    pub fn commit<D: BlockDevice>(mut self, dev: &mut D) -> Result<(Superblock, Catalog), ImportError> {
        let manifest_bytes = self
            .manifest_bytes
            .as_ref()
            .ok_or(ImportError::MissingManifest)?;
        let manifest = Manifest::parse(manifest_bytes).map_err(|_| ImportError::InvalidManifest)?;
        if manifest.name != self.model_name {
            return Err(ImportError::InvalidManifest);
        }
        if self.index_bytes.is_none() {
            return Err(ImportError::MissingIndex);
        }
        self.refresh_flags_from_index();
        self.apply_prefetch(&manifest);

        let manifest_lba = self
            .lba_by_path
            .get(MANIFEST_FILE)
            .copied()
            .unwrap_or(0);
        let index_lba = self.lba_by_path.get(INDEX_FILE).copied().unwrap_or(0);
        let manifest_blocks = self
            .shards
            .iter()
            .find(|s| s.rel_path == MANIFEST_FILE)
            .map(|s| s.extents.iter().map(|e| e.block_count).sum())
            .unwrap_or(0);
        let index_blocks = self
            .shards
            .iter()
            .find(|s| s.rel_path == INDEX_FILE)
            .map(|s| s.extents.iter().map(|e| e.block_count).sum())
            .unwrap_or(0);

        self.catalog.models.push(ModelEntry {
            name: self.model_name.clone(),
            manifest_lba,
            manifest_blocks,
            index_lba,
            index_blocks,
            shards: core::mem::take(&mut self.shards),
        });

        let cat_bytes = self.catalog.serialize();
        let cat_blocks = (cat_bytes.len() + BLOCK_SIZE - 1) / BLOCK_SIZE;
        if cat_blocks as u64 > CATALOG_RESERVED_BLOCKS {
            return Err(ImportError::CatalogTooLarge);
        }

        self.sb.generation = self.sb.generation.saturating_add(1);
        write_image(dev, &self.catalog, &self.sb).map_err(|_| ImportError::Io)?;
        Ok((self.sb, self.catalog))
    }

    pub fn abort(self) {}
}

/// Recarga superbloque y catálogo desde disco (tras commit o grow).
pub fn reload_mounted<V: crate::volume_set::VolumeSet>(
    vol: &mut V,
    sb_out: &mut Superblock,
    catalog_out: &mut Catalog,
    catalog_blocks_out: &mut u64,
) -> Result<(), FsError> {
    let mut best: Option<Superblock> = None;
    for slot in 0..SUPERBLOCK_SLOTS {
        if let Ok(sb) = crate::fs::read_super_raw(vol, slot) {
            if best.as_ref().is_none_or(|b| sb.generation > b.generation) {
                best = Some(sb);
            }
        }
    }
    let sb = best.ok_or(FsError::Corrupt)?;
    let mut catalog_bytes = Vec::with_capacity((sb.catalog_blocks as usize) * BLOCK_SIZE);
    for i in 0..sb.catalog_blocks {
        let mut buf = [0u8; BLOCK_SIZE];
        vol.read_lba(sb.catalog_bucket_root + i, &mut buf)
            .map_err(|_| FsError::Io)?;
        catalog_bytes.extend_from_slice(&buf);
    }
    *catalog_out = Catalog::parse(&catalog_bytes).map_err(|_| FsError::Corrupt)?;
    *sb_out = sb;
    *catalog_blocks_out = sb.catalog_blocks;
    Ok(())
}
