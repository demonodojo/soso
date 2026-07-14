//! Construcción de imágenes sosofs desde un directorio del host (mkfs).
//! Solo con feature `std`.
//!
//! Estrategia: asignación lineal de bloques (bump), items ordenados en un
//! BTreeMap y árbol construido de abajo arriba con hojas al ~85 % para
//! dejar hueco a la fase de escritura.

use crate::layout::*;
use block_dev::{BLOCK_SIZE, Block, BlockDevice};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;
use zerocopy::IntoBytes;

#[derive(Debug)]
pub enum BuildError {
    Io(std::io::Error),
    Device,
    DiskFull,
    NameTooLong(String),
}

impl From<std::io::Error> for BuildError {
    fn from(e: std::io::Error) -> Self {
        BuildError::Io(e)
    }
}

pub struct BuildStats {
    pub inodes: u64,
    pub blocks_used: u64,
    pub block_count: u64,
}

struct Builder<'d, D: BlockDevice> {
    dev: &'d mut D,
    next_block: u64,
    next_inode: u64,
    items: BTreeMap<Key, [u8; ITEM_PAYLOAD]>,
}

pub fn build_image<D: BlockDevice>(src: &Path, dev: &mut D) -> Result<BuildStats, BuildError> {
    let mut b = Builder {
        dev,
        next_block: 2, // tras los superbloques A/B
        next_inode: ROOT_INODE,
        items: BTreeMap::new(),
    };

    let root = b.alloc_inode();
    b.add_dir_tree(src, root)?;

    let tree_root = b.write_tree()?;
    let (bitmap_start, bitmap_blocks) = b.write_bitmap()?;

    let sb = Superblock {
        magic: MAGIC,
        crc: 0.into(),
        generation: 1.into(),
        block_count: b.dev.block_count().into(),
        tree_root: tree_root.into(),
        bitmap_start: bitmap_start.into(),
        bitmap_blocks: (bitmap_blocks as u32).into(),
        next_inode: b.next_inode.into(),
    };
    let mut block: Block = [0; BLOCK_SIZE];
    sb.write_to_block(&mut block);
    b.dev.write_block(0, &block).map_err(|_| BuildError::Device)?;
    b.dev.write_block(1, &block).map_err(|_| BuildError::Device)?;
    b.dev.flush().map_err(|_| BuildError::Device)?;

    Ok(BuildStats {
        inodes: b.next_inode - 1,
        blocks_used: b.next_block,
        block_count: b.dev.block_count(),
    })
}

impl<'d, D: BlockDevice> Builder<'d, D> {
    fn alloc_inode(&mut self) -> u64 {
        let ino = self.next_inode;
        self.next_inode += 1;
        ino
    }

    fn alloc_blocks(&mut self, n: u64) -> Result<u64, BuildError> {
        if self.next_block + n > self.dev.block_count() {
            return Err(BuildError::DiskFull);
        }
        let start = self.next_block;
        self.next_block += n;
        Ok(start)
    }

    fn put(&mut self, key: Key, payload: &[u8]) {
        let mut buf = [0u8; ITEM_PAYLOAD];
        buf[..payload.len()].copy_from_slice(payload);
        self.items.insert(key, buf);
    }

    fn put_dirent(&mut self, dir: u64, name: &str, child: u64) -> Result<(), BuildError> {
        if name.len() > NAME_MAX {
            return Err(BuildError::NameTooLong(name.into()));
        }
        let mut de = DirentItem { child: child.into(), name_len: name.len() as u8, name: [0; NAME_MAX] };
        de.name[..name.len()].copy_from_slice(name.as_bytes());
        // Sondeo lineal por si dos nombres colisionan en el hash.
        let mut off = name_hash(name.as_bytes());
        while self.items.contains_key(&Key { inode: dir, kind: KIND_DIRENT, offset: off }) {
            off = off.wrapping_add(1);
        }
        self.put(Key { inode: dir, kind: KIND_DIRENT, offset: off }, de.as_bytes());
        Ok(())
    }

    fn mtime_of(md: &std::fs::Metadata) -> u64 {
        md.modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs())
    }

    fn add_dir_tree(&mut self, dir: &Path, ino: u64) -> Result<(), BuildError> {
        let md = std::fs::metadata(dir)?;
        let inode = InodeItem {
            file_type: FT_DIR,
            _pad: [0; 7],
            size: 0.into(),
            mtime: Self::mtime_of(&md).into(),
        };
        self.put(Key::inode(ino), inode.as_bytes());

        let mut entradas: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
        entradas.sort_by_key(|e| e.file_name());
        for entrada in entradas {
            let nombre = entrada.file_name().into_string().map_err(|n| {
                BuildError::NameTooLong(n.to_string_lossy().into_owned())
            })?;
            let tipo = entrada.file_type()?;
            let child = self.alloc_inode();
            self.put_dirent(ino, &nombre, child)?;
            if tipo.is_dir() {
                self.add_dir_tree(&entrada.path(), child)?;
            } else if tipo.is_file() {
                self.add_file(&entrada.path(), child)?;
            } else {
                // symlinks y demás: fuera del alcance de v1
                self.next_inode -= 1;
            }
        }
        Ok(())
    }

    fn add_file(&mut self, path: &Path, ino: u64) -> Result<(), BuildError> {
        let md = std::fs::metadata(path)?;
        let mut file = std::fs::File::open(path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        let inode = InodeItem {
            file_type: FT_FILE,
            _pad: [0; 7],
            size: (data.len() as u64).into(),
            mtime: Self::mtime_of(&md).into(),
        };
        self.put(Key::inode(ino), inode.as_bytes());

        let max = EXTENT_MAX_BLOCKS as usize * BLOCK_SIZE;
        for (i, trozo) in data.chunks(max).enumerate() {
            let nblocks = trozo.len().div_ceil(BLOCK_SIZE) as u64;
            let start = self.alloc_blocks(nblocks)?;
            let mut padded = vec![0u8; nblocks as usize * BLOCK_SIZE];
            padded[..trozo.len()].copy_from_slice(trozo);
            for (j, bloque) in padded.chunks(BLOCK_SIZE).enumerate() {
                self.dev
                    .write_block(start + j as u64, bloque.try_into().unwrap())
                    .map_err(|_| BuildError::Device)?;
            }
            let ext = ExtentItem {
                start_block: start.into(),
                block_count: (nblocks as u32).into(),
                crc: crate::crc32c(&padded).into(),
            };
            self.put(
                Key { inode: ino, kind: KIND_EXTENT, offset: (i * max) as u64 },
                ext.as_bytes(),
            );
        }
        Ok(())
    }

    // ---- construcción del árbol, de hojas a raíz ----

    fn write_node(
        &mut self,
        level: u8,
        entries: &[(Key, Vec<u8>)],
        entry_size: usize,
    ) -> Result<u64, BuildError> {
        let block = self.alloc_blocks(1)?;
        let mut buf: Block = [0; BLOCK_SIZE];
        let hdr = NodeHeader {
            crc: 0.into(),
            generation: 1.into(),
            expected_block: block.into(),
            level,
            _pad: 0,
            nkeys: (entries.len() as u16).into(),
            _pad2: [0; 8],
        };
        buf[..NODE_HEADER_SIZE].copy_from_slice(hdr.as_bytes());
        for (i, (key, payload)) in entries.iter().enumerate() {
            let off = NODE_HEADER_SIZE + i * entry_size;
            buf[off..off + DiskKey::SIZE].copy_from_slice(DiskKey::from_key(*key).as_bytes());
            buf[off + DiskKey::SIZE..off + DiskKey::SIZE + payload.len()]
                .copy_from_slice(payload);
        }
        let crc = crate::crc32c(&buf[4..]);
        buf[0..4].copy_from_slice(&crc.to_le_bytes());
        self.dev.write_block(block, &buf).map_err(|_| BuildError::Device)?;
        Ok(block)
    }

    fn write_tree(&mut self) -> Result<u64, BuildError> {
        // Hojas al ~85 % de capacidad: hueco para inserciones futuras.
        let leaf_fill = LEAF_CAP * 85 / 100;
        let int_fill = INT_CAP * 85 / 100;

        let items: Vec<(Key, Vec<u8>)> = self
            .items
            .iter()
            .map(|(k, v)| (*k, v.to_vec()))
            .collect();

        // Nivel 0.
        let mut nivel: Vec<(Key, u64)> = Vec::new(); // (clave mínima, bloque)
        for grupo in items.chunks(leaf_fill.max(1)) {
            let min_key = grupo[0].0;
            let block = self.write_node(0, grupo, LEAF_ENTRY)?;
            nivel.push((min_key, block));
        }
        if nivel.is_empty() {
            let block = self.write_node(0, &[], LEAF_ENTRY)?;
            return Ok(block);
        }

        // Niveles internos hasta quedar una sola raíz.
        let mut level = 1u8;
        while nivel.len() > 1 {
            let mut siguiente = Vec::new();
            let entradas: Vec<(Key, Vec<u8>)> = nivel
                .iter()
                .map(|(k, b)| (*k, b.to_le_bytes().to_vec()))
                .collect();
            for grupo in entradas.chunks(int_fill.max(2)) {
                let min_key = grupo[0].0;
                let block = self.write_node(level, grupo, INT_ENTRY)?;
                siguiente.push((min_key, block));
            }
            nivel = siguiente;
            level += 1;
        }
        Ok(nivel[0].1)
    }

    fn write_bitmap(&mut self) -> Result<(u64, u64), BuildError> {
        let block_count = self.dev.block_count();
        let bitmap_blocks = block_count.div_ceil(BLOCK_SIZE as u64 * 8);
        let start = self.alloc_blocks(bitmap_blocks)?;
        // Todo lo asignado hasta ahora (incluido el propio bitmap) está usado.
        let usados = self.next_block;
        for b in 0..bitmap_blocks {
            let mut buf: Block = [0; BLOCK_SIZE];
            for bit in 0..(BLOCK_SIZE * 8) {
                let bloque = b * BLOCK_SIZE as u64 * 8 + bit as u64;
                if bloque < usados {
                    buf[bit / 8] |= 1 << (bit % 8);
                }
            }
            self.dev.write_block(start + b, &buf).map_err(|_| BuildError::Device)?;
        }
        Ok((start, bitmap_blocks))
    }
}
