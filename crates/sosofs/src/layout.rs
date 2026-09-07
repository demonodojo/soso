//! Estructuras on-disk de sosofs v1. Todas little-endian y sin requisitos
//! de alineación (tipos de zerocopy), para leerlas de buffers arbitrarios.

use block_dev::BLOCK_SIZE;
use zerocopy::little_endian::{U16, U32, U64};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

pub const MAGIC: [u8; 8] = *b"SOSOFS11";
pub const ROOT_INODE: u64 = 1;
pub const NAME_MAX: usize = 255;
/// Tamaño máximo de un extent en bloques (128 KiB): acota la memoria
/// necesaria para verificar su checksum de una pieza.
pub const EXTENT_MAX_BLOCKS: u64 = 32;

// Tipos de item (campo `kind` de la clave).
pub const KIND_INODE: u8 = 1;
pub const KIND_DIRENT: u8 = 2;
pub const KIND_EXTENT: u8 = 3;

// Tipos de inode.
pub const FT_FILE: u8 = 1;
pub const FT_DIR: u8 = 2;

/// Clave lógica del árbol. El orden derivado (inode, kind, offset) ES el
/// orden on-disk: no reordenar los campos.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Key {
    pub inode: u64,
    pub kind: u8,
    pub offset: u64,
}

impl Key {
    pub const MIN: Key = Key { inode: 0, kind: 0, offset: 0 };
    pub const MAX: Key = Key { inode: u64::MAX, kind: u8::MAX, offset: u64::MAX };

    pub fn inode(ino: u64) -> Key {
        Key { inode: ino, kind: KIND_INODE, offset: 0 }
    }

    /// Rango completo de un tipo de item dentro de un inode.
    pub fn range(ino: u64, kind: u8) -> (Key, Key) {
        (
            Key { inode: ino, kind, offset: 0 },
            Key { inode: ino, kind, offset: u64::MAX },
        )
    }
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone, Copy)]
#[repr(C)]
pub struct DiskKey {
    pub inode: U64,
    pub offset: U64,
    pub kind: u8,
    pub _pad: [u8; 7],
}

impl DiskKey {
    pub const SIZE: usize = 24;

    pub fn from_key(k: Key) -> Self {
        Self { inode: k.inode.into(), offset: k.offset.into(), kind: k.kind, _pad: [0; 7] }
    }

    pub fn key(&self) -> Key {
        Key { inode: self.inode.get(), kind: self.kind, offset: self.offset.get() }
    }
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone, Copy)]
#[repr(C)]
pub struct Superblock {
    pub magic: [u8; 8],
    /// crc32c de los bytes del superbloque a partir de `generation`.
    pub crc: U32,
    pub generation: U64,
    pub block_count: U64,
    pub tree_root: U64,
    pub bitmap_start: U64,
    pub bitmap_blocks: U32,
    pub next_inode: U64,
}

impl Superblock {
    pub const SIZE: usize = 56;
    /// Offset del primer byte cubierto por el crc (tras magic + crc).
    pub const CRC_FROM: usize = 12;

    pub fn parse(block: &[u8; BLOCK_SIZE]) -> Option<Superblock> {
        let (sb, _) = Superblock::read_from_prefix(block).ok()?;
        if sb.magic != MAGIC {
            return None;
        }
        if crate::crc32c(&block[Self::CRC_FROM..Self::SIZE]) != sb.crc.get() {
            return None;
        }
        Some(sb)
    }

    /// Serializa al principio de un bloque, sellando el crc.
    pub fn write_to_block(&self, block: &mut [u8; BLOCK_SIZE]) {
        block[..Self::SIZE].copy_from_slice(self.as_bytes());
        let crc = crate::crc32c(&block[Self::CRC_FROM..Self::SIZE]);
        block[8..12].copy_from_slice(&crc.to_le_bytes());
    }
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone, Copy)]
#[repr(C)]
pub struct NodeHeader {
    /// crc32c de los bytes [4..BLOCK_SIZE] del nodo.
    pub crc: U32,
    pub generation: U64,
    /// Bloque donde este nodo espera vivir: detecta punteros desviados.
    pub expected_block: U64,
    /// 0 = hoja; >0 = interno.
    pub level: u8,
    pub _pad: u8,
    pub nkeys: U16,
    pub _pad2: [u8; 8],
}

pub const NODE_HEADER_SIZE: usize = 32;
/// Hoja: DiskKey (24) + payload (64).
pub const ITEM_PAYLOAD: usize = 272;
pub const LEAF_ENTRY: usize = DiskKey::SIZE + ITEM_PAYLOAD; // 296
pub const LEAF_CAP: usize = (BLOCK_SIZE - NODE_HEADER_SIZE) / LEAF_ENTRY; // 13
/// Interno: DiskKey (24) + hijo (8).
pub const INT_ENTRY: usize = DiskKey::SIZE + 8; // 32
pub const INT_CAP: usize = (BLOCK_SIZE - NODE_HEADER_SIZE) / INT_ENTRY; // 127

// ---- payloads (64 bytes, rellenos con ceros) ----

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone, Copy, Debug)]
#[repr(C)]
pub struct InodeItem {
    pub file_type: u8,
    pub _pad: [u8; 7],
    pub size: U64,
    /// Segundos desde epoch.
    pub mtime: U64,
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone, Copy)]
#[repr(C)]
pub struct DirentItem {
    pub child: U64,
    pub name_len: u8,
    pub _pad: [u8; 7],
    pub name: [u8; NAME_MAX],
}

impl DirentItem {
    pub fn name_bytes(&self) -> &[u8] {
        &self.name[..(self.name_len as usize).min(NAME_MAX)]
    }
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone, Copy, Debug)]
#[repr(C)]
pub struct ExtentItem {
    pub start_block: U64,
    pub block_count: U32,
    /// crc32c de los datos del extent, incluido el relleno del último bloque.
    pub crc: U32,
}

/// Hash FNV-1a de 64 bits para el offset de los dirents.
pub fn name_hash(name: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in name {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
