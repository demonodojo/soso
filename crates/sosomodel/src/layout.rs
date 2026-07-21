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

/// Bloque Q8_0 on-disk: escala f32 LE + 32 cuantizados i8.
pub const Q8_0_BLOCK_ELEMS: usize = 32;
pub const Q8_0_BLOCK_BYTES: usize = 4 + Q8_0_BLOCK_ELEMS;

/// Superbloque Q4_K on-disk: layout GGML exacto (passthrough desde GGUF):
/// d f16 + dmin f16 + 12 B de escalas/mins 6-bit + 128 B de nibbles.
pub const Q4_K_BLOCK_ELEMS: usize = 256;
pub const Q4_K_BLOCK_BYTES: usize = 2 + 2 + 12 + 128;

/// Fichero de vocabulario del tokenizer dentro del modelo.
pub const TOKENIZER_FILE: &str = "tokenizer.som";

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SomHeader {
    pub magic: [u8; 8],
    pub crc: u32,
    pub version: u32,
    pub payload_len: u64,
}

pub const SOM_HEADER_SIZE: usize = core::mem::size_of::<SomHeader>();
