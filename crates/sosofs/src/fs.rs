//! Montaje y camino de lectura de sosofs. Nada se devuelve sin verificar
//! su checksum.

use crate::layout::*;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use block_dev::{BLOCK_SIZE, Block, BlockDevice};
use zerocopy::FromBytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    Io,
    NoValidSuperblock,
    /// Checksum inválido en un nodo del árbol o un extent de datos.
    BadChecksum { block: u64 },
    /// Un nodo no está en el bloque en el que dice vivir.
    MisplacedNode { block: u64 },
    NotFound,
    NotADir,
    NotAFile,
    NameTooLong,
    NoSpace,
    Exists,
    DirNotEmpty,
    Corrupt,
}

pub struct Sosofs<D: BlockDevice> {
    pub(crate) dev: D,
    /// Último superbloque comprometido.
    pub(crate) sb: Superblock,
    /// Raíz de trabajo: puede ir por delante de `sb` dentro de una transacción.
    pub(crate) tree_root: u64,
    pub(crate) next_inode: u64,
    /// Asignación de la generación en construcción (se muta al asignar/liberar).
    pub(crate) bitmap: alloc::vec::Vec<u8>,
    /// Asignación de la generación comprometida: nada marcado aquí puede
    /// reutilizarse hasta después del próximo commit (regla de oro CoW).
    pub(crate) frozen: alloc::vec::Vec<u8>,
    pub(crate) dirty: bool,
    pub(crate) alloc_hint: u64,
}

impl<D: BlockDevice> Sosofs<D> {
    pub fn mount(mut dev: D) -> Result<Self, FsError> {
        let mut best: Option<Superblock> = None;
        for slot in 0..2 {
            let mut buf: Box<Block> = Box::new([0; BLOCK_SIZE]);
            if dev.read_block(slot, &mut buf).is_err() {
                continue;
            }
            if let Some(sb) = Superblock::parse(&buf)
                && best.is_none_or(|b| sb.generation.get() > b.generation.get())
            {
                best = Some(sb);
            }
        }
        let sb = best.ok_or(FsError::NoValidSuperblock)?;

        // Cargar el bitmap de la generación comprometida.
        let mut bitmap = alloc::vec![0u8; (sb.bitmap_blocks.get() as usize) * BLOCK_SIZE];
        for i in 0..sb.bitmap_blocks.get() as u64 {
            let buf: &mut Block = (&mut bitmap
                [i as usize * BLOCK_SIZE..(i as usize + 1) * BLOCK_SIZE])
                .try_into()
                .unwrap();
            dev.read_block(sb.bitmap_start.get() + i, buf).map_err(|_| FsError::Io)?;
        }

        Ok(Self {
            tree_root: sb.tree_root.get(),
            next_inode: sb.next_inode.get(),
            frozen: bitmap.clone(),
            bitmap,
            dirty: false,
            alloc_hint: 2,
            dev,
            sb,
        })
    }

    pub fn generation(&self) -> u64 {
        self.sb.generation.get()
    }

    /// Desmonta y devuelve el dispositivo (tests).
    pub fn into_device(self) -> D {
        self.dev
    }

    pub fn block_count(&self) -> u64 {
        self.sb.block_count.get()
    }

    /// Bloques libres en la generación en construcción.
    pub fn free_blocks(&self) -> u64 {
        let total = self.sb.block_count.get();
        (0..total).filter(|&b| self.bitmap[(b / 8) as usize] & (1 << (b % 8)) == 0).count()
            as u64
    }

    // ---- árbol ----

    pub(crate) fn read_node(&mut self, block: u64) -> Result<Box<Block>, FsError> {
        let mut buf: Box<Block> = Box::new([0; BLOCK_SIZE]);
        self.dev.read_block(block, &mut buf).map_err(|_| FsError::Io)?;
        let (hdr, _) = NodeHeader::read_from_prefix(&buf[..]).map_err(|_| FsError::Corrupt)?;
        if crate::crc32c(&buf[4..]) != hdr.crc.get() {
            return Err(FsError::BadChecksum { block });
        }
        if hdr.expected_block.get() != block {
            return Err(FsError::MisplacedNode { block });
        }
        Ok(buf)
    }

    /// Recorre el árbol y entrega los items con clave en `[min, max]`,
    /// en orden.
    pub fn scan_range(
        &mut self,
        min: Key,
        max: Key,
        out: &mut Vec<(Key, [u8; ITEM_PAYLOAD])>,
    ) -> Result<(), FsError> {
        let root = self.tree_root;
        self.scan_node(root, min, max, out)
    }

    fn scan_node(
        &mut self,
        block: u64,
        min: Key,
        max: Key,
        out: &mut Vec<(Key, [u8; ITEM_PAYLOAD])>,
    ) -> Result<(), FsError> {
        let node = self.read_node(block)?;
        let (hdr, _) = NodeHeader::read_from_prefix(&node[..]).map_err(|_| FsError::Corrupt)?;
        let n = hdr.nkeys.get() as usize;

        if hdr.level == 0 {
            if n > LEAF_CAP {
                return Err(FsError::Corrupt);
            }
            for i in 0..n {
                let off = NODE_HEADER_SIZE + i * LEAF_ENTRY;
                let (dk, _) = DiskKey::read_from_prefix(&node[off..]).map_err(|_| FsError::Corrupt)?;
                let key = dk.key();
                if key >= min && key <= max {
                    let mut payload = [0u8; ITEM_PAYLOAD];
                    payload.copy_from_slice(&node[off + DiskKey::SIZE..off + LEAF_ENTRY]);
                    out.push((key, payload));
                }
            }
        } else {
            if n > INT_CAP {
                return Err(FsError::Corrupt);
            }
            // Invariante: la clave de la entrada i es la mínima de su
            // subárbol; el hijo i cubre [clave_i, clave_{i+1}).
            for i in 0..n {
                let off = NODE_HEADER_SIZE + i * INT_ENTRY;
                let (dk, _) = DiskKey::read_from_prefix(&node[off..]).map_err(|_| FsError::Corrupt)?;
                let low = dk.key();
                let high = if i + 1 < n {
                    let noff = NODE_HEADER_SIZE + (i + 1) * INT_ENTRY;
                    let (nk, _) =
                        DiskKey::read_from_prefix(&node[noff..]).map_err(|_| FsError::Corrupt)?;
                    Some(nk.key())
                } else {
                    None
                };
                if low <= max && high.is_none_or(|h| h > min) {
                    let child = u64::from_le_bytes(
                        node[off + DiskKey::SIZE..off + INT_ENTRY].try_into().unwrap(),
                    );
                    self.scan_node(child, min, max, out)?;
                }
            }
        }
        Ok(())
    }

    // ---- API de ficheros ----

    pub fn stat_inode(&mut self, ino: u64) -> Result<InodeItem, FsError> {
        let k = Key::inode(ino);
        let mut out = Vec::new();
        self.scan_range(k, k, &mut out)?;
        let (_, payload) = out.first().ok_or(FsError::NotFound)?;
        let (item, _) = InodeItem::read_from_prefix(payload).map_err(|_| FsError::Corrupt)?;
        Ok(item)
    }

    pub fn lookup(&mut self, dir: u64, name: &str) -> Result<u64, FsError> {
        if name.len() > NAME_MAX {
            return Err(FsError::NameTooLong);
        }
        let (min, max) = Key::range(dir, KIND_DIRENT);
        let mut out = Vec::new();
        self.scan_range(min, max, &mut out)?;
        for (_, payload) in &out {
            let (de, _) = DirentItem::read_from_prefix(payload).map_err(|_| FsError::Corrupt)?;
            if de.name_bytes() == name.as_bytes() {
                return Ok(de.child.get());
            }
        }
        Err(FsError::NotFound)
    }

    /// Resuelve una ruta absoluta (o relativa a la raíz) a un inode.
    pub fn resolve(&mut self, path: &str) -> Result<u64, FsError> {
        let mut ino = ROOT_INODE;
        for comp in path.split('/').filter(|c| !c.is_empty() && *c != ".") {
            let st = self.stat_inode(ino)?;
            if st.file_type != FT_DIR {
                return Err(FsError::NotADir);
            }
            ino = self.lookup(ino, comp)?;
        }
        Ok(ino)
    }

    pub fn read_dir(&mut self, dir: u64) -> Result<Vec<(String, u64)>, FsError> {
        let st = self.stat_inode(dir)?;
        if st.file_type != FT_DIR {
            return Err(FsError::NotADir);
        }
        let (min, max) = Key::range(dir, KIND_DIRENT);
        let mut out = Vec::new();
        self.scan_range(min, max, &mut out)?;
        let mut entries = Vec::with_capacity(out.len());
        for (_, payload) in &out {
            let (de, _) = DirentItem::read_from_prefix(payload).map_err(|_| FsError::Corrupt)?;
            let name = String::from_utf8_lossy(de.name_bytes()).into_owned();
            entries.push((name, de.child.get()));
        }
        entries.sort();
        Ok(entries)
    }

    /// Lee un fichero completo, verificando el crc de cada extent.
    pub fn read_file(&mut self, ino: u64) -> Result<Vec<u8>, FsError> {
        let st = self.stat_inode(ino)?;
        if st.file_type != FT_FILE {
            return Err(FsError::NotAFile);
        }
        let size = st.size.get() as usize;

        let (min, max) = Key::range(ino, KIND_EXTENT);
        let mut extents = Vec::new();
        self.scan_range(min, max, &mut extents)?;

        let mut data = Vec::with_capacity(size);
        for (key, payload) in &extents {
            if key.offset != data.len() as u64 {
                return Err(FsError::Corrupt); // hueco o solape entre extents
            }
            let (ext, _) = ExtentItem::read_from_prefix(payload).map_err(|_| FsError::Corrupt)?;
            let nblocks = ext.block_count.get() as usize;
            if nblocks == 0 || nblocks as u64 > EXTENT_MAX_BLOCKS {
                return Err(FsError::Corrupt);
            }
            let mut chunk = vec![0u8; nblocks * BLOCK_SIZE];
            for i in 0..nblocks {
                let block = ext.start_block.get() + i as u64;
                let buf: &mut Block = (&mut chunk[i * BLOCK_SIZE..(i + 1) * BLOCK_SIZE])
                    .try_into()
                    .unwrap();
                self.dev.read_block(block, buf).map_err(|_| FsError::Io)?;
            }
            if crate::crc32c(&chunk) != ext.crc.get() {
                return Err(FsError::BadChecksum { block: ext.start_block.get() });
            }
            let restante = size - data.len();
            chunk.truncate(restante.min(chunk.len()));
            data.extend_from_slice(&chunk);
        }
        if data.len() != size {
            return Err(FsError::Corrupt);
        }
        Ok(data)
    }
}
