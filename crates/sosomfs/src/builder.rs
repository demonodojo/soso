//! Construcción de imágenes sosomfs desde un directorio sosomodel.

use crate::catalog::Catalog;
use crate::fs::write_image;
use crate::layout::*;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use block_dev::{BlockDevice, BlockError, BLOCK_SIZE};
use crc::{CRC_32_ISCSI, Crc};
use sosomodel::index::TensorIndex;
use sosomodel::layout::{INDEX_FILE, MANIFEST_FILE, SHARDS_DIR};
use sosomodel::manifest::Manifest;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const CRC32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

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

struct WritePlan {
    rel_path: String,
    data: Vec<u8>,
    cache_policy: u8,
    align_requirement: u8,
    flags: u16,
    prefetch_next: Option<String>,
    prefetch_bytes: u32,
}

pub struct BuildReport {
    pub total_blocks: u64,
    pub models: usize,
    pub shards: usize,
}

pub fn build_from_dir<D: BlockDevice>(
    dev: &mut D,
    model_root: &Path,
    generation: u64,
) -> Result<BuildReport, String> {
    let manifest_data = fs::read(model_root.join(MANIFEST_FILE))
        .map_err(|e| format!("leer manifest: {e}"))?;
    let manifest = Manifest::parse(&manifest_data).map_err(|_| "manifest inválido".to_string())?;
    let index_data = fs::read(model_root.join(INDEX_FILE))
        .map_err(|e| format!("leer index: {e}"))?;
    let index = TensorIndex::parse(&index_data).map_err(|_| "index inválido".to_string())?;

    let mut order: Vec<String> = Vec::new();
    order.push(MANIFEST_FILE.to_string());
    order.push(INDEX_FILE.to_string());
    for pf in &manifest.prefetch {
        for s in &pf.shards {
            order.push(format!("{SHARDS_DIR}/{s}"));
        }
    }
    let shards_dir = model_root.join(SHARDS_DIR);
    if shards_dir.is_dir() {
        for ent in fs::read_dir(&shards_dir).map_err(|e| e.to_string())? {
            let ent = ent.map_err(|e| e.to_string())?;
            let name = ent.file_name().to_string_lossy().into_owned();
            let rel = format!("{SHARDS_DIR}/{name}");
            if !order.contains(&rel) {
                order.push(rel);
            }
        }
    }

    let mut prefetch_map: BTreeMap<String, (String, u32)> = BTreeMap::new();
    for pf in &manifest.prefetch {
        let chain: Vec<String> = pf
            .shards
            .iter()
            .map(|s| format!("{SHARDS_DIR}/{s}"))
            .collect();
        for i in 0..chain.len().saturating_sub(1) {
            prefetch_map.insert(chain[i].clone(), (chain[i + 1].clone(), 8 * 1024 * 1024));
        }
    }

    let mut plans: Vec<WritePlan> = Vec::new();
    for rel in &order {
        let path = model_root.join(rel);
        let data = if path.exists() {
            fs::read(&path).map_err(|e| format!("leer {}: {e}", path.display()))?
        } else if *rel == MANIFEST_FILE {
            manifest_data.clone()
        } else if *rel == INDEX_FILE {
            index_data.clone()
        } else {
            continue;
        };
        let cache_policy = if *rel == MANIFEST_FILE || *rel == INDEX_FILE {
            CACHE_PIN
        } else if rel.starts_with(SHARDS_DIR) && rel.ends_with(".tensor") {
            CACHE_STREAM
        } else {
            CACHE_NORMAL
        };
        let align_requirement = if data.len() >= 2 * 1024 * 1024 {
            ALIGN_HUGE_2M
        } else if cache_policy == CACHE_STREAM {
            ALIGN_GPU_DMA_64K
        } else {
            ALIGN_PAGE_4K
        };
        let mut flags = 0u16;
        let shard_name = rel.strip_prefix(&format!("{SHARDS_DIR}/")).unwrap_or(rel);
        if let Some(ent) = index.entries.iter().find(|e| e.shard == shard_name) {
            if ent.quant == sosomodel::layout::QUANT_Q4_K {
                flags |= FLAG_TILE_Q4;
            } else if ent.quant == sosomodel::layout::QUANT_Q8_0 {
                flags |= FLAG_TILE_Q8;
            }
        }
        let (prefetch_next, prefetch_bytes) = prefetch_map
            .get(rel)
            .map(|(n, b)| (Some(n.clone()), *b))
            .unwrap_or((None, 0));
        plans.push(WritePlan {
            rel_path: rel.clone(),
            data,
            cache_policy,
            align_requirement,
            flags,
            prefetch_next,
            prefetch_bytes,
        });
    }

    let catalog_root = SUPERBLOCK_SLOTS;
    let mut next_lba = catalog_root + 64;
    let mut shard_entries: Vec<ShardEntry> = Vec::new();
    let mut lba_by_path: BTreeMap<String, u64> = BTreeMap::new();

    for plan in &plans {
        let padded_len = align_bytes(plan.data.len(), plan.align_requirement);
        let mut padded = plan.data.clone();
        padded.resize(padded_len, 0);
        let blocks = (padded_len + BLOCK_SIZE - 1) / BLOCK_SIZE;
        let start_lba = next_lba;
        write_payload(dev, start_lba, &padded).map_err(|_| "escribir shard".to_string())?;
        lba_by_path.insert(plan.rel_path.clone(), start_lba);

        let mut extents = Vec::new();
        let mut off = 0usize;
        while off < padded_len {
            let seg_len = SEGMENT_SIZE.min(padded_len - off);
            let seg_blocks = (seg_len + BLOCK_SIZE - 1) / BLOCK_SIZE;
            let seg_crc = crc32c(&padded[off..off + seg_len]);
            extents.push(Extent {
                volume_id: 0,
                start_lba: start_lba + (off / BLOCK_SIZE) as u64,
                block_count: seg_blocks as u64,
                segment_crc32c: seg_crc,
                stripe_width: 0,
                stripe_index: 0,
            });
            off += seg_len;
        }
        let prefetch_next_lba = 0u64;
        shard_entries.push(ShardEntry {
            rel_path: plan.rel_path.clone(),
            byte_len: padded_len as u64,
            extents,
            prefetch_next_lba,
            prefetch_bytes: plan.prefetch_bytes,
            cache_policy: plan.cache_policy,
            align_requirement: plan.align_requirement,
            flags: plan.flags,
            shard_crc32c: crc32c(&padded),
        });
        next_lba += blocks as u64;
    }

    for shard in &mut shard_entries {
        if let Some((next_path, _)) = prefetch_map.get(&shard.rel_path) {
            shard.prefetch_next_lba = lba_by_path.get(next_path).copied().unwrap_or(0);
        }
    }

    let manifest_lba = lba_by_path.get(MANIFEST_FILE).copied().unwrap_or(0);
    let index_lba = lba_by_path.get(INDEX_FILE).copied().unwrap_or(0);
    let manifest_blocks = shard_entries
        .iter()
        .find(|s| s.rel_path == MANIFEST_FILE)
        .map(|s| s.extents.iter().map(|e| e.block_count).sum())
        .unwrap_or(0);
    let index_blocks = shard_entries
        .iter()
        .find(|s| s.rel_path == INDEX_FILE)
        .map(|s| s.extents.iter().map(|e| e.block_count).sum())
        .unwrap_or(0);

    let catalog = Catalog {
        models: vec![ModelEntry {
            name: manifest.name.clone(),
            manifest_lba,
            manifest_blocks,
            index_lba,
            index_blocks,
            shards: shard_entries,
        }],
    };

    let sb = Superblock {
        generation,
        block_size: BLOCK_SIZE as u32,
        total_blocks: dev.block_count(),
        catalog_bucket_count: 1,
        catalog_bucket_root: catalog_root,
        data_start: catalog_root + 64,
        volume_count: 1,
        catalog_blocks: 0,
        volumes: [{
            VolumeDesc {
                volume_id: 0,
                device_slot: 0,
                start_lba: 0,
                block_count: dev.block_count(),
            }
        }; 16],
    };

    write_image(dev, &catalog, &sb).map_err(|_| "escribir imagen".to_string())?;
    Ok(BuildReport {
        total_blocks: sb.total_blocks,
        models: 1,
        shards: catalog.models[0].shards.len(),
    })
}

fn write_payload<D: BlockDevice>(dev: &mut D, start: u64, data: &[u8]) -> Result<(), BlockError> {
    for (i, chunk) in data.chunks(BLOCK_SIZE).enumerate() {
        let mut buf = [0u8; BLOCK_SIZE];
        buf[..chunk.len()].copy_from_slice(chunk);
        dev.write_block(start + i as u64, &buf)?;
    }
    Ok(())
}

pub fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix("GiB").or_else(|| s.strip_suffix('G')) {
        let n: u64 = n.parse().map_err(|_| format!("tamaño inválido: {s}"))?;
        return Ok(n * 1024 * 1024 * 1024 / BLOCK_SIZE as u64);
    }
    if let Some(n) = s.strip_suffix("MiB").or_else(|| s.strip_suffix('M')) {
        let n: u64 = n.parse().map_err(|_| format!("tamaño inválido: {s}"))?;
        return Ok(n * 1024 * 1024 / BLOCK_SIZE as u64);
    }
    if let Some(n) = s.strip_suffix("TiB").or_else(|| s.strip_suffix('T')) {
        let n: u64 = n.parse().map_err(|_| format!("tamaño inválido: {s}"))?;
        return Ok(n * 1024 * 1024 * 1024 * 1024 / BLOCK_SIZE as u64);
    }
    let n: u64 = s.parse().map_err(|_| format!("tamaño inválido: {s}"))?;
    Ok(n)
}
