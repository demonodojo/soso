//! Camino de escritura de sosofs: asignador con bitmap dual, mutación CoW
//! del árbol y commit atómico.
//!
//! Invariantes:
//! - Nunca se sobreescribe un bloque marcado en `frozen` (la generación
//!   comprometida): un corte de luz en cualquier punto deja la generación
//!   anterior intacta.
//! - Cada nodo modificado se reescribe en un bloque nuevo, y el camino
//!   hasta la raíz con él (CoW clásico); el commit publica la nueva raíz
//!   escribiendo el superbloque en el slot alterno tras dos flushes.

use crate::fs::{FsError, Sosofs};
use crate::layout::*;
use alloc::vec;
use alloc::vec::Vec;
use block_dev::{BLOCK_SIZE, Block, BlockDevice};
use zerocopy::{FromBytes, IntoBytes};

enum NodeData {
    Leaf(Vec<(Key, [u8; ITEM_PAYLOAD])>),
    Internal(Vec<(Key, u64)>),
}

struct Cow {
    block: u64,
    min_key: Key,
    /// Hermano derecho creado por un split: (su clave mínima, su bloque).
    split: Option<(Key, u64)>,
}

impl<D: BlockDevice> Sosofs<D> {
    // ---- asignador (bitmap dual) ----

    fn bit(map: &[u8], b: u64) -> bool {
        map[(b / 8) as usize] & (1 << (b % 8)) != 0
    }

    fn set_bit(map: &mut [u8], b: u64, v: bool) {
        if v {
            map[(b / 8) as usize] |= 1 << (b % 8);
        } else {
            map[(b / 8) as usize] &= !(1 << (b % 8));
        }
    }

    /// Asigna `n` bloques consecutivos libres en AMBOS bitmaps.
    fn alloc_run(&mut self, n: u64) -> Result<u64, FsError> {
        let total = self.sb.block_count.get();
        for base in [self.alloc_hint.max(2), 2] {
            let mut b = base;
            'candidato: while b + n <= total {
                for i in 0..n {
                    if Self::bit(&self.bitmap, b + i) || Self::bit(&self.frozen, b + i) {
                        b = b + i + 1;
                        continue 'candidato;
                    }
                }
                for i in 0..n {
                    Self::set_bit(&mut self.bitmap, b + i, true);
                }
                self.alloc_hint = b + n;
                self.dirty = true;
                return Ok(b);
            }
        }
        Err(FsError::NoSpace)
    }

    /// Intenta `want` bloques consecutivos, reduciendo a la mitad si no hay
    /// hueco contiguo. Devuelve (inicio, conseguidos).
    fn alloc_extent(&mut self, want: u64) -> Result<(u64, u64), FsError> {
        let mut n = want.max(1);
        loop {
            match self.alloc_run(n) {
                Ok(start) => return Ok((start, n)),
                Err(FsError::NoSpace) if n > 1 => n /= 2,
                Err(e) => return Err(e),
            }
        }
    }

    fn free_block(&mut self, b: u64) {
        Self::set_bit(&mut self.bitmap, b, false);
        self.dirty = true;
    }

    // ---- nodos ----

    fn parse_node(&mut self, block: u64) -> Result<NodeData, FsError> {
        let node = self.read_node(block)?;
        let (hdr, _) = NodeHeader::read_from_prefix(&node[..]).map_err(|_| FsError::Corrupt)?;
        let n = hdr.nkeys.get() as usize;
        if hdr.level == 0 {
            if n > LEAF_CAP {
                return Err(FsError::Corrupt);
            }
            let mut v = Vec::with_capacity(n);
            for i in 0..n {
                let off = NODE_HEADER_SIZE + i * LEAF_ENTRY;
                let (dk, _) =
                    DiskKey::read_from_prefix(&node[off..]).map_err(|_| FsError::Corrupt)?;
                let mut payload = [0u8; ITEM_PAYLOAD];
                payload.copy_from_slice(&node[off + DiskKey::SIZE..off + LEAF_ENTRY]);
                v.push((dk.key(), payload));
            }
            Ok(NodeData::Leaf(v))
        } else {
            if n > INT_CAP {
                return Err(FsError::Corrupt);
            }
            let mut v = Vec::with_capacity(n);
            for i in 0..n {
                let off = NODE_HEADER_SIZE + i * INT_ENTRY;
                let (dk, _) =
                    DiskKey::read_from_prefix(&node[off..]).map_err(|_| FsError::Corrupt)?;
                let child =
                    u64::from_le_bytes(node[off + DiskKey::SIZE..off + INT_ENTRY].try_into().unwrap());
                v.push((dk.key(), child));
            }
            Ok(NodeData::Internal(v))
        }
    }

    fn node_level(&mut self, block: u64) -> Result<u8, FsError> {
        let node = self.read_node(block)?;
        let (hdr, _) = NodeHeader::read_from_prefix(&node[..]).map_err(|_| FsError::Corrupt)?;
        Ok(hdr.level)
    }

    /// Escribe un nodo en un bloque recién asignado. Devuelve (bloque, clave mínima).
    fn write_node(&mut self, level: u8, data: &NodeData) -> Result<(u64, Key), FsError> {
        let block = self.alloc_run(1)?;
        let mut buf: Block = [0; BLOCK_SIZE];
        let (nkeys, min_key) = match data {
            NodeData::Leaf(v) => {
                for (i, (key, payload)) in v.iter().enumerate() {
                    let off = NODE_HEADER_SIZE + i * LEAF_ENTRY;
                    buf[off..off + DiskKey::SIZE]
                        .copy_from_slice(DiskKey::from_key(*key).as_bytes());
                    buf[off + DiskKey::SIZE..off + LEAF_ENTRY].copy_from_slice(payload);
                }
                (v.len(), v.first().map_or(Key::MIN, |e| e.0))
            }
            NodeData::Internal(v) => {
                for (i, (key, child)) in v.iter().enumerate() {
                    let off = NODE_HEADER_SIZE + i * INT_ENTRY;
                    buf[off..off + DiskKey::SIZE]
                        .copy_from_slice(DiskKey::from_key(*key).as_bytes());
                    buf[off + DiskKey::SIZE..off + INT_ENTRY]
                        .copy_from_slice(&child.to_le_bytes());
                }
                (v.len(), v.first().map_or(Key::MIN, |e| e.0))
            }
        };
        let hdr = NodeHeader {
            crc: 0.into(),
            generation: (self.sb.generation.get() + 1).into(),
            expected_block: block.into(),
            level,
            _pad: 0,
            nkeys: (nkeys as u16).into(),
            _pad2: [0; 8],
        };
        buf[..NODE_HEADER_SIZE].copy_from_slice(hdr.as_bytes());
        let crc = crate::crc32c(&buf[4..]);
        buf[0..4].copy_from_slice(&crc.to_le_bytes());
        self.dev.write_block(block, &buf).map_err(|_| FsError::Io)?;
        Ok((block, min_key))
    }

    // ---- inserción / borrado CoW ----

    /// Inserta o reemplaza (upsert) un item.
    pub(crate) fn tree_insert(
        &mut self,
        key: Key,
        payload: &[u8; ITEM_PAYLOAD],
    ) -> Result<(), FsError> {
        let root = self.tree_root;
        let level = self.node_level(root)?;
        let cow = self.insert_rec(root, level, key, payload)?;
        self.tree_root = match cow.split {
            None => cow.block,
            Some((sk, sblk)) => {
                let (nb, _) = self.write_node(
                    level + 1,
                    &NodeData::Internal(vec![(cow.min_key, cow.block), (sk, sblk)]),
                )?;
                nb
            }
        };
        Ok(())
    }

    fn insert_rec(
        &mut self,
        block: u64,
        level: u8,
        key: Key,
        payload: &[u8; ITEM_PAYLOAD],
    ) -> Result<Cow, FsError> {
        match self.parse_node(block)? {
            NodeData::Leaf(mut v) => {
                match v.binary_search_by_key(&key, |e| e.0) {
                    Ok(i) => v[i].1 = *payload,
                    Err(i) => v.insert(i, (key, *payload)),
                }
                self.free_block(block);
                self.write_split(0, v.len() > LEAF_CAP, NodeData::Leaf(v))
            }
            NodeData::Internal(mut v) => {
                let idx = match v.binary_search_by_key(&key, |e| e.0) {
                    Ok(i) => i,
                    Err(0) => 0,
                    Err(i) => i - 1,
                };
                let cow = self.insert_rec(v[idx].1, level - 1, key, payload)?;
                v[idx] = (cow.min_key, cow.block);
                if let Some((sk, sblk)) = cow.split {
                    v.insert(idx + 1, (sk, sblk));
                }
                self.free_block(block);
                self.write_split(level, v.len() > INT_CAP, NodeData::Internal(v))
            }
        }
    }

    /// Escribe el nodo, partiéndolo en dos si `overflow`.
    fn write_split(&mut self, level: u8, overflow: bool, data: NodeData) -> Result<Cow, FsError> {
        if !overflow {
            let (block, min_key) = self.write_node(level, &data)?;
            return Ok(Cow { block, min_key, split: None });
        }
        let (izq, der) = match data {
            NodeData::Leaf(mut v) => {
                let d = v.split_off(v.len() / 2);
                (NodeData::Leaf(v), NodeData::Leaf(d))
            }
            NodeData::Internal(mut v) => {
                let d = v.split_off(v.len() / 2);
                (NodeData::Internal(v), NodeData::Internal(d))
            }
        };
        let (lb, lmin) = self.write_node(level, &izq)?;
        let (rb, rmin) = self.write_node(level, &der)?;
        Ok(Cow { block: lb, min_key: lmin, split: Some((rmin, rb)) })
    }

    pub(crate) fn tree_delete(&mut self, key: Key) -> Result<(), FsError> {
        let root = self.tree_root;
        let level = self.node_level(root)?;
        match self.delete_rec(root, level, key)? {
            Some((nb, _)) => self.tree_root = nb,
            None => {
                // Árbol vacío: raíz = hoja vacía.
                let (nb, _) = self.write_node(0, &NodeData::Leaf(Vec::new()))?;
                self.tree_root = nb;
            }
        }
        // Encoger la altura mientras la raíz interna tenga un solo hijo.
        loop {
            match self.parse_node(self.tree_root)? {
                NodeData::Internal(v) if v.len() == 1 => {
                    self.free_block(self.tree_root);
                    self.tree_root = v[0].1;
                }
                _ => break,
            }
        }
        Ok(())
    }

    /// None: el nodo quedó vacío (y su bloque liberado).
    fn delete_rec(
        &mut self,
        block: u64,
        level: u8,
        key: Key,
    ) -> Result<Option<(u64, Key)>, FsError> {
        match self.parse_node(block)? {
            NodeData::Leaf(mut v) => {
                let i = v.binary_search_by_key(&key, |e| e.0).map_err(|_| FsError::NotFound)?;
                v.remove(i);
                self.free_block(block);
                if v.is_empty() {
                    return Ok(None);
                }
                let (nb, min) = self.write_node(0, &NodeData::Leaf(v))?;
                Ok(Some((nb, min)))
            }
            NodeData::Internal(mut v) => {
                let idx = match v.binary_search_by_key(&key, |e| e.0) {
                    Ok(i) => i,
                    Err(0) => 0,
                    Err(i) => i - 1,
                };
                match self.delete_rec(v[idx].1, level - 1, key)? {
                    Some((nb, nmin)) => v[idx] = (nmin, nb),
                    None => {
                        v.remove(idx);
                    }
                }
                self.free_block(block);
                if v.is_empty() {
                    return Ok(None);
                }
                let (nb, min) = self.write_node(level, &NodeData::Internal(v))?;
                Ok(Some((nb, min)))
            }
        }
    }

    // ---- transacciones ----

    /// Publica la transacción: bitmap CoW + superbloque en el slot alterno.
    pub fn commit(&mut self) -> Result<(), FsError> {
        if !self.dirty {
            return Ok(());
        }
        // Bitmap CoW: liberar los bloques del bitmap viejo y asignar nuevos.
        let old_start = self.sb.bitmap_start.get();
        let nblocks = self.sb.bitmap_blocks.get() as u64;
        for i in 0..nblocks {
            self.free_block(old_start + i);
        }
        let new_start = self.alloc_run(nblocks)?;
        for i in 0..nblocks {
            let buf: &Block = (&self.bitmap
                [i as usize * BLOCK_SIZE..(i as usize + 1) * BLOCK_SIZE])
                .try_into()
                .unwrap();
            let buf = *buf; // copia: no se puede prestar self.bitmap y self.dev a la vez
            self.dev.write_block(new_start + i, &buf).map_err(|_| FsError::Io)?;
        }
        self.dev.flush().map_err(|_| FsError::Io)?;

        let new_gen = self.sb.generation.get() + 1;
        let sb = Superblock {
            magic: MAGIC,
            crc: 0.into(),
            generation: new_gen.into(),
            block_count: self.sb.block_count,
            tree_root: self.tree_root.into(),
            bitmap_start: new_start.into(),
            bitmap_blocks: (nblocks as u32).into(),
            next_inode: self.next_inode.into(),
        };
        let mut block: Block = [0; BLOCK_SIZE];
        sb.write_to_block(&mut block);
        self.dev.write_block(new_gen % 2, &block).map_err(|_| FsError::Io)?;
        self.dev.flush().map_err(|_| FsError::Io)?;

        self.sb = sb;
        self.frozen.copy_from_slice(&self.bitmap);
        self.dirty = false;
        Ok(())
    }

    /// Descarta la transacción en curso: vuelve al estado comprometido.
    pub fn rollback(&mut self) {
        self.bitmap.copy_from_slice(&self.frozen);
        self.tree_root = self.sb.tree_root.get();
        self.next_inode = self.sb.next_inode.get();
        self.dirty = false;
        self.alloc_hint = 2;
    }

    fn finish<T>(&mut self, r: Result<T, FsError>) -> Result<T, FsError> {
        match r {
            Ok(v) => match self.commit() {
                Ok(()) => Ok(v),
                Err(e) => {
                    self.rollback();
                    Err(e)
                }
            },
            Err(e) => {
                self.rollback();
                Err(e)
            }
        }
    }

    // ---- operaciones de alto nivel (una transacción cada una) ----

    pub fn mkdir(&mut self, dir: u64, name: &str, mtime: u64) -> Result<u64, FsError> {
        let r = self.mkdir_inner(dir, name, mtime);
        self.finish(r)
    }

    pub fn create_file(
        &mut self,
        dir: u64,
        name: &str,
        data: &[u8],
        mtime: u64,
    ) -> Result<u64, FsError> {
        let r = self.create_file_inner(dir, name, data, mtime);
        self.finish(r)
    }

    /// Añade bytes al final de un fichero existente (escritura streaming).
    pub fn append_file(&mut self, ino: u64, data: &[u8], mtime: u64) -> Result<(), FsError> {
        let r = self.append_file_inner(ino, data, mtime);
        self.finish(r).map(|_| ())
    }

    pub fn unlink(&mut self, dir: u64, name: &str) -> Result<(), FsError> {
        let r = self.unlink_inner(dir, name);
        self.finish(r)
    }

    /// Renombra o mueve un dirent (mismo volumen).
    pub fn rename(
        &mut self,
        old_dir: u64,
        old_name: &str,
        new_dir: u64,
        new_name: &str,
        mtime: u64,
    ) -> Result<(), FsError> {
        let r = self.rename_inner(old_dir, old_name, new_dir, new_name, mtime);
        self.finish(r)
    }

    /// Trunca un fichero a `new_size` bytes.
    pub fn truncate_file(&mut self, ino: u64, new_size: u64, mtime: u64) -> Result<(), FsError> {
        let r = self.truncate_inner(ino, new_size, mtime);
        self.finish(r).map(|_| ())
    }

    /// Actualiza solo el mtime de un inode.
    pub fn set_mtime(&mut self, ino: u64, mtime: u64) -> Result<(), FsError> {
        let r = self.set_mtime_inner(ino, mtime);
        self.finish(r)
    }

    fn check_dir(&mut self, dir: u64) -> Result<(), FsError> {
        if self.stat_inode(dir)?.file_type != FT_DIR {
            return Err(FsError::NotADir);
        }
        Ok(())
    }

    fn insert_dirent(&mut self, dir: u64, name: &str, child: u64) -> Result<(), FsError> {
        if name.len() > NAME_MAX || name.is_empty() {
            return Err(FsError::NameTooLong);
        }
        let mut de =
            DirentItem {
                child: child.into(),
                name_len: name.len() as u8,
                _pad: [0; 7],
                name: [0; NAME_MAX],
            };
        de.name[..name.len()].copy_from_slice(name.as_bytes());
        // Sondeo lineal si el hash colisiona con otro nombre.
        let mut off = name_hash(name.as_bytes());
        loop {
            let k = Key { inode: dir, kind: KIND_DIRENT, offset: off };
            let mut out = Vec::new();
            self.scan_range(k, k, &mut out)?;
            if out.is_empty() {
                let mut payload = [0u8; ITEM_PAYLOAD];
                payload[..core::mem::size_of::<DirentItem>()].copy_from_slice(de.as_bytes());
                return self.tree_insert(k, &payload);
            }
            off = off.wrapping_add(1);
        }
    }

    /// Clave exacta del dirent de `name` en `dir` (el offset pudo sondearse).
    fn dirent_key(&mut self, dir: u64, name: &str) -> Result<Key, FsError> {
        let (min, max) = Key::range(dir, KIND_DIRENT);
        let mut out = Vec::new();
        self.scan_range(min, max, &mut out)?;
        for (key, payload) in &out {
            let (de, _) = DirentItem::read_from_prefix(payload).map_err(|_| FsError::Corrupt)?;
            if de.name_bytes() == name.as_bytes() {
                return Ok(*key);
            }
        }
        Err(FsError::NotFound)
    }

    fn mkdir_inner(&mut self, dir: u64, name: &str, mtime: u64) -> Result<u64, FsError> {
        self.check_dir(dir)?;
        if self.lookup(dir, name).is_ok() {
            return Err(FsError::Exists);
        }
        let ino = self.next_inode;
        self.next_inode += 1;
        let inode =
            InodeItem { file_type: FT_DIR, _pad: [0; 7], size: 0.into(), mtime: mtime.into() };
        let mut payload = [0u8; ITEM_PAYLOAD];
        payload[..core::mem::size_of::<InodeItem>()].copy_from_slice(inode.as_bytes());
        self.tree_insert(Key::inode(ino), &payload)?;
        self.insert_dirent(dir, name, ino)?;
        Ok(ino)
    }

    fn create_file_inner(
        &mut self,
        dir: u64,
        name: &str,
        data: &[u8],
        mtime: u64,
    ) -> Result<u64, FsError> {
        self.check_dir(dir)?;
        if let Ok(existing) = self.lookup(dir, name) {
            if self.stat_inode(existing)?.file_type == FT_DIR {
                return Err(FsError::NotAFile);
            }
            self.unlink_inner(dir, name)?; // sobreescritura
        }
        let ino = self.next_inode;
        self.next_inode += 1;
        let inode = InodeItem {
            file_type: FT_FILE,
            _pad: [0; 7],
            size: (data.len() as u64).into(),
            mtime: mtime.into(),
        };
        let mut payload = [0u8; ITEM_PAYLOAD];
        payload[..core::mem::size_of::<InodeItem>()].copy_from_slice(inode.as_bytes());
        self.tree_insert(Key::inode(ino), &payload)?;

        let mut off = 0usize;
        while off < data.len() {
            let restante = data.len() - off;
            let want = (restante.div_ceil(BLOCK_SIZE) as u64).min(EXTENT_MAX_BLOCKS);
            let (start, got) = self.alloc_extent(want)?;
            let chunk_len = restante.min(got as usize * BLOCK_SIZE);
            let mut padded = vec![0u8; got as usize * BLOCK_SIZE];
            padded[..chunk_len].copy_from_slice(&data[off..off + chunk_len]);
            for (j, bloque) in padded.chunks(BLOCK_SIZE).enumerate() {
                self.dev
                    .write_block(start + j as u64, bloque.try_into().unwrap())
                    .map_err(|_| FsError::Io)?;
            }
            let ext = ExtentItem {
                start_block: start.into(),
                block_count: (got as u32).into(),
                crc: crate::crc32c(&padded).into(),
            };
            let mut payload = [0u8; ITEM_PAYLOAD];
            payload[..core::mem::size_of::<ExtentItem>()].copy_from_slice(ext.as_bytes());
            self.tree_insert(Key { inode: ino, kind: KIND_EXTENT, offset: off as u64 }, &payload)?;
            off += chunk_len;
        }

        self.insert_dirent(dir, name, ino)?;
        Ok(ino)
    }

    fn append_file_inner(&mut self, ino: u64, data: &[u8], mtime: u64) -> Result<u64, FsError> {
        if data.is_empty() {
            return Ok(ino);
        }
        let st = self.stat_inode(ino)?;
        if st.file_type != FT_FILE {
            return Err(FsError::NotAFile);
        }
        let size = st.size.get() as usize;
        let new_size = size + data.len();

        // La clave de un extent es un offset de fichero **alineado a bloque**:
        // así los emite `create_file_inner` y así los interpreta
        // `read_file_range`, que calcula `ext_end = key.offset + bloques*4096`.
        //
        // AVERÍA: esto arrancaba en `off = st.size`, el tamaño actual a pelo. Al
        // añadir a un fichero de 6 bytes, el extent nuevo se indexaba en el
        // offset 6 y se solapaba con el que cubre [0, 4096): al leer, los rangos
        // se contaban dos veces, `written` pasaba de `len` y el kernel moría en
        // `fs.rs` con «range end index 9 out of range for slice of length 6».
        // Y como `sys_write` pasa por aquí, le ocurría a **cualquier** escritura
        // de tamaño no múltiplo de 4096.
        //
        // No basta con alinear al bloque que contiene `size`: ese bloque puede
        // estar en medio de un extent de varios bloques, y un extent nuevo a
        // mitad de otro vuelve a solaparse. Hay que reescribir desde el
        // principio del **último extent**, que sí es una clave existente y
        // alineada; así el `tree_insert` lo reemplaza en vez de añadir uno que
        // pise.
        let (min, max) = Key::range(ino, KIND_EXTENT);
        let mut extents = Vec::new();
        self.scan_range(min, max, &mut extents)?;
        let base = extents.last().map(|(k, _)| k.offset as usize).unwrap_or(0);
        if base > size {
            return Err(FsError::Corrupt);
        }

        // Lo que ya había en ese último extent se reescribe junto con lo nuevo.
        let previo = size - base;
        let mut buf = Vec::new();
        buf.try_reserve(previo + data.len()).map_err(|_| FsError::NoSpace)?;
        if previo > 0 {
            buf.resize(previo, 0);
            self.read_file_range(ino, base, previo, &mut buf)?;
        }
        buf.extend_from_slice(data);

        let mut payload_inode = [0u8; ITEM_PAYLOAD];
        let inode = InodeItem {
            file_type: FT_FILE,
            _pad: [0; 7],
            size: (new_size as u64).into(),
            mtime: mtime.into(),
        };
        payload_inode[..core::mem::size_of::<InodeItem>()].copy_from_slice(inode.as_bytes());
        self.tree_insert(Key::inode(ino), &payload_inode)?;

        let mut off = base;
        let mut chunk_off = 0usize;
        while chunk_off < buf.len() {
            let restante = buf.len() - chunk_off;
            let want = (restante.div_ceil(BLOCK_SIZE) as u64).min(EXTENT_MAX_BLOCKS);
            let (start, got) = self.alloc_extent(want)?;
            let chunk_len = restante.min(got as usize * BLOCK_SIZE);
            let mut padded = vec![0u8; got as usize * BLOCK_SIZE];
            padded[..chunk_len].copy_from_slice(&buf[chunk_off..chunk_off + chunk_len]);
            for (j, bloque) in padded.chunks(BLOCK_SIZE).enumerate() {
                self.dev
                    .write_block(start + j as u64, bloque.try_into().unwrap())
                    .map_err(|_| FsError::Io)?;
            }
            let ext = ExtentItem {
                start_block: start.into(),
                block_count: (got as u32).into(),
                crc: crate::crc32c(&padded).into(),
            };
            let mut payload = [0u8; ITEM_PAYLOAD];
            payload[..core::mem::size_of::<ExtentItem>()].copy_from_slice(ext.as_bytes());
            self.tree_insert(
                Key {
                    inode: ino,
                    kind: KIND_EXTENT,
                    offset: off as u64,
                },
                &payload,
            )?;
            off += chunk_len;
            chunk_off += chunk_len;
        }
        Ok(ino)
    }

    fn unlink_inner(&mut self, dir: u64, name: &str) -> Result<(), FsError> {
        let dirent_key = self.dirent_key(dir, name)?;
        let ino = self.lookup(dir, name)?;
        let st = self.stat_inode(ino)?;

        if st.file_type == FT_DIR {
            let (min, max) = Key::range(ino, KIND_DIRENT);
            let mut out = Vec::new();
            self.scan_range(min, max, &mut out)?;
            if !out.is_empty() {
                return Err(FsError::DirNotEmpty);
            }
        } else {
            // Liberar los datos y borrar los items EXTENT.
            let (min, max) = Key::range(ino, KIND_EXTENT);
            let mut extents = Vec::new();
            self.scan_range(min, max, &mut extents)?;
            for (key, payload) in &extents {
                let (ext, _) =
                    ExtentItem::read_from_prefix(payload).map_err(|_| FsError::Corrupt)?;
                for i in 0..ext.block_count.get() as u64 {
                    self.free_block(ext.start_block.get() + i);
                }
                self.tree_delete(*key)?;
            }
        }
        self.tree_delete(Key::inode(ino))?;
        self.tree_delete(dirent_key)?;
        Ok(())
    }

    fn rename_inner(
        &mut self,
        old_dir: u64,
        old_name: &str,
        new_dir: u64,
        new_name: &str,
        mtime: u64,
    ) -> Result<(), FsError> {
        self.check_dir(old_dir)?;
        self.check_dir(new_dir)?;
        if new_name.is_empty() || old_name.is_empty() {
            return Err(FsError::NameTooLong);
        }
        if self.lookup(new_dir, new_name).is_ok() {
            return Err(FsError::Exists);
        }
        let ino = self.lookup(old_dir, old_name)?;
        let old_key = self.dirent_key(old_dir, old_name)?;
        self.tree_delete(old_key)?;
        self.insert_dirent(new_dir, new_name, ino)?;
        let st = self.stat_inode(ino)?;
        let inode = InodeItem {
            file_type: st.file_type,
            _pad: [0; 7],
            size: st.size,
            mtime: mtime.into(),
        };
        let mut payload = [0u8; ITEM_PAYLOAD];
        payload[..core::mem::size_of::<InodeItem>()].copy_from_slice(inode.as_bytes());
        self.tree_insert(Key::inode(ino), &payload)?;
        Ok(())
    }

    fn truncate_inner(&mut self, ino: u64, new_size: u64, mtime: u64) -> Result<(), FsError> {
        let st = self.stat_inode(ino)?;
        if st.file_type != FT_FILE {
            return Err(FsError::NotAFile);
        }
        let old_size = st.size.get();
        if new_size >= old_size {
            let inode = InodeItem {
                file_type: FT_FILE,
                _pad: [0; 7],
                size: new_size.into(),
                mtime: mtime.into(),
            };
            let mut payload = [0u8; ITEM_PAYLOAD];
            payload[..core::mem::size_of::<InodeItem>()].copy_from_slice(inode.as_bytes());
            self.tree_insert(Key::inode(ino), &payload)?;
            return Ok(());
        }
        let new_size_usize = new_size as usize;
        let (min, max) = Key::range(ino, KIND_EXTENT);
        let mut extents = Vec::new();
        self.scan_range(min, max, &mut extents)?;
        for (key, payload) in &extents {
            let off = key.offset as usize;
            let (ext, _) = ExtentItem::read_from_prefix(payload).map_err(|_| FsError::Corrupt)?;
            let ext_end = off + ext.block_count.get() as usize * BLOCK_SIZE;
            if off >= new_size_usize {
                for i in 0..ext.block_count.get() as u64 {
                    self.free_block(ext.start_block.get() + i);
                }
                self.tree_delete(*key)?;
            } else if ext_end > new_size_usize {
                let keep = new_size_usize - off;
                let want = (keep.div_ceil(BLOCK_SIZE) as u64).min(EXTENT_MAX_BLOCKS);
                let (start, got) = self.alloc_extent(want)?;
                let chunk_len = keep.min(got as usize * BLOCK_SIZE);
                let mut padded = vec![0u8; got as usize * BLOCK_SIZE];
                if chunk_len > 0 {
                    self.read_file_range(ino, off, chunk_len, &mut padded[..chunk_len])?;
                }
                for (j, bloque) in padded.chunks(BLOCK_SIZE).enumerate() {
                    self.dev
                        .write_block(start + j as u64, bloque.try_into().unwrap())
                        .map_err(|_| FsError::Io)?;
                }
                for i in 0..ext.block_count.get() as u64 {
                    self.free_block(ext.start_block.get() + i);
                }
                self.tree_delete(*key)?;
                let ext_new = ExtentItem {
                    start_block: start.into(),
                    block_count: (got as u32).into(),
                    crc: crate::crc32c(&padded).into(),
                };
                let mut pl = [0u8; ITEM_PAYLOAD];
                pl[..core::mem::size_of::<ExtentItem>()].copy_from_slice(ext_new.as_bytes());
                self.tree_insert(
                    Key {
                        inode: ino,
                        kind: KIND_EXTENT,
                        offset: off as u64,
                    },
                    &pl,
                )?;
            }
        }
        let inode = InodeItem {
            file_type: FT_FILE,
            _pad: [0; 7],
            size: new_size.into(),
            mtime: mtime.into(),
        };
        let mut payload = [0u8; ITEM_PAYLOAD];
        payload[..core::mem::size_of::<InodeItem>()].copy_from_slice(inode.as_bytes());
        self.tree_insert(Key::inode(ino), &payload)?;
        Ok(())
    }

    fn set_mtime_inner(&mut self, ino: u64, mtime: u64) -> Result<(), FsError> {
        let st = self.stat_inode(ino)?;
        let inode = InodeItem {
            file_type: st.file_type,
            _pad: [0; 7],
            size: st.size,
            mtime: mtime.into(),
        };
        let mut payload = [0u8; ITEM_PAYLOAD];
        payload[..core::mem::size_of::<InodeItem>()].copy_from_slice(inode.as_bytes());
        self.tree_insert(Key::inode(ino), &payload)?;
        Ok(())
    }
}
