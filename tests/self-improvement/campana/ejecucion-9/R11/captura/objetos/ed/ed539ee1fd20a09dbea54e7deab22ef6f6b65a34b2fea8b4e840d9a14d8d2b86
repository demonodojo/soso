//! Estructuras on-disk de sosomfs v1.

use block_dev::BLOCK_SIZE;

pub const MAGIC: [u8; 8] = *b"SOSOMFS1";
pub const NAME_MAX: usize = 64;
pub const PATH_MAX: usize = 96;
pub const INLINE_EXTENTS: usize = 4;
pub const SEGMENT_SIZE: usize = 8 * 1024 * 1024;
pub const SEGMENT_BLOCKS: u64 = (SEGMENT_SIZE / BLOCK_SIZE) as u64;

pub const CACHE_NORMAL: u8 = 0;
pub const CACHE_STREAM: u8 = 1;
pub const CACHE_PIN: u8 = 2;

pub const ALIGN_CACHE_64: u8 = 0;
pub const ALIGN_PAGE_4K: u8 = 1;
pub const ALIGN_HUGE_2M: u8 = 2;
pub const ALIGN_GPU_DMA_64K: u8 = 3;

pub const FLAG_SHARED_EXTENT: u16 = 1;
pub const FLAG_TILE_Q4: u16 = 2;
pub const FLAG_TILE_Q8: u16 = 4;

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct VolumeDesc {
    pub volume_id: u32,
    pub device_slot: u32,
    pub start_lba: u64,
    pub block_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Extent {
    pub volume_id: u32,
    pub start_lba: u64,
    pub block_count: u64,
    pub segment_crc32c: u32,
    pub stripe_width: u16,
    pub stripe_index: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardEntry {
    pub rel_path: alloc::string::String,
    pub byte_len: u64,
    pub extents: alloc::vec::Vec<Extent>,
    pub prefetch_next_lba: u64,
    pub prefetch_bytes: u32,
    pub cache_policy: u8,
    pub align_requirement: u8,
    pub flags: u16,
    pub shard_crc32c: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelEntry {
    pub name: alloc::string::String,
    pub manifest_lba: u64,
    pub manifest_blocks: u64,
    pub index_lba: u64,
    pub index_blocks: u64,
    pub shards: alloc::vec::Vec<ShardEntry>,
}

#[derive(Clone, Copy, Debug)]
pub struct Superblock {
    pub generation: u64,
    pub block_size: u32,
    pub total_blocks: u64,
    pub catalog_bucket_count: u32,
    pub catalog_bucket_root: u64,
    pub data_start: u64,
    pub volume_count: u32,
    pub catalog_blocks: u64,
    pub volumes: [VolumeDesc; 16],
}

pub const SUPERBLOCK_SLOTS: u64 = 2;
