//! Montaje y camino de lectura de sosofs. Nada se devuelve sin verificar
//! su checksum.

use crate::layout::*;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use block_dev::{BLOCK_SIZE, Block, BlockDevice};
use zerocopy::FromBytes;

/// Cuántos búferes de nodo se guardan para reutilizar.
///
/// La profundidad del árbol acota cuántos hacen falta a la vez, así que más
/// que esto sería retener memoria por si acaso.
const NODOS_POOL: usize = 8;

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
    /// Cuántos nodos del árbol se han leído desde que se montó.
    ///
    /// Diagnóstico, no contabilidad: es el equivalente un nivel por encima de
    /// lo que `SYS_IOSTAT` cuenta en el dispositivo, y existe porque
    /// [N-012](../../../docs/self-improvement/native/N-012.md) necesitaba
    /// saber **cuántas** lecturas hace un `stat` antes de decidir si el coste
    /// está en cada lectura o en el número de ellas. Adivinarlo desde fuera ya
    /// costó tres pasos.
    nodos_leidos: u64,
    /// Búferes de nodo reutilizables.
    ///
    /// `read_node` pedía un `Box<Block>` nuevo —4 KiB— en cada lectura, y en
    /// soso eso no es barato: el asignador del kernel audita cada reserva
    /// recorriendo su tabla de bloques vivos, así que **cada `alloc` cuesta
    /// O(reservas vivas)**. Medido en el guest: quitar esa auditoría hacía un
    /// `stat` 4× más rápido, y este pool ataca el mismo coste desde el lado
    /// que sí es nuestro — pidiendo menos.
    ///
    /// Devolver un búfer al pool es **opcional**: quien no lo haga simplemente
    /// lo libera como antes. Por eso no hay forma de corromper nada
    /// olvidándose, sólo de no ganar.
    nodos_libres: Vec<Box<Block>>,
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
            nodos_leidos: 0,
            nodos_libres: Vec::new(),
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
        self.nodos_leidos = self.nodos_leidos.wrapping_add(1);
        let mut buf: Box<Block> = self.tomar_nodo();
        self.dev.read_block(block, &mut buf).map_err(|_| FsError::Io)?;
        let (hdr, _) = NodeHeader::read_from_prefix(&buf[..]).map_err(|_| FsError::Corrupt)?;
        // El CRC se comprueba cuando el bloque **llega del dispositivo**, no
        // cada vez que se lee de RAM. Un bloque cacheado ya pasó por aquí, y
        // la caché se invalida al escribir, así que un acierto es exactamente
        // lo último que se verificó.
        //
        // Lo que se ahorra no es una micro-optimización: N-012 midió que un
        // `stat` de cinco componentes equivalía a ~49 CRC de bloque de 4 KiB,
        // y casi todos eran del mismo puñado de nodos del árbol.
        //
        // Lo que **no** se ahorra: si el bloque viene corrupto, se saca de la
        // caché antes de devolver el error. Si no, la siguiente lectura sería
        // un acierto y se daría por buena sin mirar.
        if !self.dev.last_read_cached() {
            if crate::crc32c(&buf[4..]) != hdr.crc.get() {
                self.dev.invalidate_block(block);
                return Err(FsError::BadChecksum { block });
            }
            if hdr.expected_block.get() != block {
                self.dev.invalidate_block(block);
                return Err(FsError::MisplacedNode { block });
            }
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
            self.soltar_nodo(node);
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
            self.soltar_nodo(node);
        }
        Ok(())
    }

    /// Un búfer de nodo: del pool si hay, recién pedido si no.
    fn tomar_nodo(&mut self) -> Box<Block> {
        self.nodos_libres.pop().unwrap_or_else(|| Box::new([0; BLOCK_SIZE]))
    }

    /// Devuelve un búfer al pool. Si está lleno, se libera como cualquier otro.
    ///
    /// El tope es bajo a propósito: la profundidad del árbol acota cuántos se
    /// usan a la vez, y guardar más sería retener memoria por si acaso.
    fn soltar_nodo(&mut self, buf: Box<Block>) {
        if self.nodos_libres.len() < NODOS_POOL {
            self.nodos_libres.push(buf);
        }
    }

    /// Nodos del árbol leídos desde el montaje. Ver [`Self::nodos_leidos`].
    pub fn nodos_leidos(&self) -> u64 {
        self.nodos_leidos
    }

    /// Como [`Self::scan_range`], pero **para en cuanto `pred` acepta uno**.
    ///
    /// Existe porque `lookup` no necesita el directorio entero: necesita **un**
    /// nombre. Recogerlos todos en un `Vec` hacía que resolver una ruta
    /// costara O(entradas del directorio) — medido en
    /// [N-012](../../../docs/self-improvement/native/N-012.md): 13 µs con 8
    /// entradas, 39 µs con 64 y 135 µs con 256, en RAM y sin tocar el disco.
    ///
    /// El recorrido es **en orden**, así que «el primero que acepta» es el
    /// primero por clave, que es lo que el sondeo lineal de los dirents
    /// necesita.
    pub(crate) fn find_first(
        &mut self,
        min: Key,
        max: Key,
        pred: &mut dyn FnMut(&[u8; ITEM_PAYLOAD]) -> bool,
    ) -> Result<Option<(Key, [u8; ITEM_PAYLOAD])>, FsError> {
        let root = self.tree_root;
        self.find_node(root, min, max, pred)
    }

    fn find_node(
        &mut self,
        block: u64,
        min: Key,
        max: Key,
        pred: &mut dyn FnMut(&[u8; ITEM_PAYLOAD]) -> bool,
    ) -> Result<Option<(Key, [u8; ITEM_PAYLOAD])>, FsError> {
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
                    if pred(&payload) {
                        self.soltar_nodo(node);
                        return Ok(Some((key, payload)));
                    }
                }
            }
            self.soltar_nodo(node);
        } else {
            if n > INT_CAP {
                return Err(FsError::Corrupt);
            }
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
                    if let Some(hit) = self.find_node(child, min, max, pred)? {
                        self.soltar_nodo(node);
                        return Ok(Some(hit));
                    }
                }
            }
            self.soltar_nodo(node);
        }
        Ok(None)
    }

    /// Busca el dirent de `name` en `dir` sin recorrer el directorio entero.
    ///
    /// La clave del dirent es `name_hash(name)` con **sondeo lineal hacia
    /// delante** si colisiona — lo hacen igual el builder
    /// (`builder.rs`) y la escritura en caliente (`write.rs`). Así que la
    /// entrada, si existe, está en `[hash, MAX]`; y la vuelta `[0, hash)` se
    /// mira sólo si no apareció, por si el sondeo dio la vuelta al contador.
    ///
    /// Buscar sin encontrar sigue costando lo de antes. Encontrar, que es el
    /// caso de todas las rutas que se resuelven, cuesta la profundidad del
    /// árbol.
    pub(crate) fn dirent_buscar(
        &mut self,
        dir: u64,
        name: &str,
    ) -> Result<Option<(Key, DirentItem)>, FsError> {
        let objetivo = name.as_bytes();
        let mut encontrado: Option<DirentItem> = None;
        let mut pred = |payload: &[u8; ITEM_PAYLOAD]| match DirentItem::read_from_prefix(payload) {
            Ok((de, _)) if de.name_bytes() == objetivo => {
                encontrado = Some(de);
                true
            }
            _ => false,
        };
        let h = name_hash(objetivo);
        let hit = self.find_first(
            Key { inode: dir, kind: KIND_DIRENT, offset: h },
            Key { inode: dir, kind: KIND_DIRENT, offset: u64::MAX },
            &mut pred,
        )?;
        if let Some((key, _)) = hit {
            return Ok(encontrado.map(|de| (key, de)));
        }
        let Some(antes) = h.checked_sub(1) else {
            return Ok(None);
        };
        let hit = self.find_first(
            Key { inode: dir, kind: KIND_DIRENT, offset: 0 },
            Key { inode: dir, kind: KIND_DIRENT, offset: antes },
            &mut pred,
        )?;
        Ok(hit.and_then(|(key, _)| encontrado.map(|de| (key, de))))
    }

    // ---- API de ficheros ----

    pub fn stat_inode(&mut self, ino: u64) -> Result<InodeItem, FsError> {
        // Una consulta puntual no necesita un `Vec`: pedía uno y empujaba un
        // elemento, y eso es **una reserva de montón** por cada `stat`. En
        // soso cada reserva cuesta O(reservas vivas) porque el asignador del
        // kernel audita la tabla entera ([N-013](../../../docs/self-improvement/native/N-013.md)),
        // así que quitar reservas del camino caliente vale más que abaratar lo
        // que hacen.
        let k = Key::inode(ino);
        let mut encontrado: Option<InodeItem> = None;
        let mut pred = |payload: &[u8; ITEM_PAYLOAD]| match InodeItem::read_from_prefix(payload) {
            Ok((item, _)) => {
                encontrado = Some(item);
                true
            }
            Err(_) => false,
        };
        self.find_first(k, k, &mut pred)?;
        encontrado.ok_or(FsError::NotFound)
    }

    pub fn lookup(&mut self, dir: u64, name: &str) -> Result<u64, FsError> {
        if name.len() > NAME_MAX {
            return Err(FsError::NameTooLong);
        }
        match self.dirent_buscar(dir, name)? {
            Some((_, de)) => Ok(de.child.get()),
            None => Err(FsError::NotFound),
        }
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
        let size = st.size.get() as usize;
        let mut data = vec![0u8; size];
        self.read_file_range(ino, 0, size, &mut data)?;
        Ok(data)
    }

    /// Lee un rango de bytes sin cargar el fichero entero.
    pub fn read_file_range(
        &mut self,
        ino: u64,
        offset: usize,
        len: usize,
        out: &mut [u8],
    ) -> Result<(), FsError> {
        if out.len() < len {
            return Err(FsError::Corrupt);
        }
        let st = self.stat_inode(ino)?;
        if st.file_type != FT_FILE {
            return Err(FsError::NotAFile);
        }
        let size = st.size.get() as usize;
        if offset > size || offset + len > size {
            return Err(FsError::Corrupt);
        }
        if len == 0 {
            return Ok(());
        }

        let (min, max) = Key::range(ino, KIND_EXTENT);
        let mut extents = Vec::new();
        self.scan_range(min, max, &mut extents)?;

        let mut written = 0usize;
        let end = offset + len;
        for (key, payload) in &extents {
            let (ext, _) = ExtentItem::read_from_prefix(payload).map_err(|_| FsError::Corrupt)?;
            let nblocks = ext.block_count.get() as usize;
            if nblocks == 0 || nblocks as u64 > EXTENT_MAX_BLOCKS {
                return Err(FsError::Corrupt);
            }
            let ext_start = key.offset as usize;
            let ext_end = ext_start + nblocks * BLOCK_SIZE;
            if ext_end <= offset || ext_start >= end {
                continue;
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
            let ext_size = (size - ext_start).min(chunk.len());
            chunk.truncate(ext_size);
            let copy_start = offset.saturating_sub(ext_start);
            let copy_end = (end - ext_start).min(chunk.len());
            if copy_start < copy_end {
                let n = copy_end - copy_start;
                // Extents solapados (o un `out` más corto de lo que dice `len`)
                // no deben matar al kernel: es un FS malformado, no un bug del
                // llamante. Sin esto, un append que insertaba extents en claves
                // no alineadas panicaba aquí con «range end index N out of range».
                if written + n > out.len() {
                    return Err(FsError::Corrupt);
                }
                out[written..written + n].copy_from_slice(&chunk[copy_start..copy_end]);
                written += n;
            }
        }
        if written != len {
            return Err(FsError::Corrupt);
        }
        Ok(())
    }
}
