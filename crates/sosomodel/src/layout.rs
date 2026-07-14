//! Constantes y tipos on-disk del formato sosomodel.

pub const MAGIC: [u8; 8] = *b"SOMODL01";
pub const MANIFEST_FILE: &str = "manifest.som";
pub const INDEX_FILE: &str = "index.som";
pub const SHARDS_DIR: &str = "shards";

pub const DTYPE_F32: u8 = 1;
pub const DTYPE_Q8_0: u8 = 2;
pub const DTYPE_Q4_K: u8 = 3;

pub const QUANT_NONE: u8 = 0;
pub const QUANT_Q8_0: u8 = 1;
pub const QUANT_Q4_K: u8 = 2;

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SomHeader {
    pub magic: [u8; 8],
    pub crc: u32,
    pub version: u32,
    pub payload_len: u64,
}

pub const SOM_HEADER_SIZE: usize = core::mem::size_of::<SomHeader>();
