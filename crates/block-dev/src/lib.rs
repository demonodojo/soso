//! Abstracción de dispositivo de bloques compartida entre el kernel
//! (virtio-blk) y las herramientas de host (fichero imagen, memoria).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub const BLOCK_SIZE: usize = 4096;
pub type Block = [u8; BLOCK_SIZE];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    Io,
    OutOfRange,
}

pub trait BlockDevice {
    fn block_count(&self) -> u64;
    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError>;
    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError>;
    fn flush(&mut self) -> Result<(), BlockError>;

    /// Cuántos bloques admite el dispositivo en **una sola** petición. 1 = no
    /// sabe agrupar, y el llamante no debe molestarse en juntar rangos.
    ///
    /// Existe porque el tope no es del formato sino del transporte: virtio
    /// rebota por un buffer DMA físicamente contiguo y NVMe por una lista PRP,
    /// y cada uno se rinde a un tamaño distinto.
    fn max_blocks_per_request(&self) -> usize {
        1
    }

    /// Lee `buf.len() / BLOCK_SIZE` bloques consecutivos desde `start`.
    ///
    /// El método por defecto es el bucle de toda la vida, para que los
    /// dispositivos de host (imagen en fichero, memoria) sigan funcionando sin
    /// tocarlos. Los backends que sepan servirlo en una petición lo
    /// sobrescriben — y ahí está toda la ganancia: hasta ahora **cada bloque de
    /// 4 KiB costaba un viaje completo al dispositivo** (medido: 178 us, o sea
    /// 22 MB/s, en un disco que no existe).
    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if buf.len() % BLOCK_SIZE != 0 {
            return Err(BlockError::OutOfRange);
        }
        for (i, trozo) in buf.chunks_exact_mut(BLOCK_SIZE).enumerate() {
            let b: &mut Block = trozo.try_into().map_err(|_| BlockError::OutOfRange)?;
            self.read_block(start + i as u64, b)?;
        }
        Ok(())
    }
}

/// Dispositivo en memoria: tests y construcción de imágenes pequeñas.
#[derive(Clone)]
pub struct MemBlockDevice {
    data: alloc::vec::Vec<u8>,
    #[cfg(feature = "std")]
    reads: std::cell::Cell<u64>,
}

impl MemBlockDevice {
    pub fn new(blocks: u64) -> Self {
        Self {
            data: alloc::vec![0; blocks as usize * BLOCK_SIZE],
            #[cfg(feature = "std")]
            reads: std::cell::Cell::new(0),
        }
    }

    #[cfg(feature = "std")]
    pub fn read_count(&self) -> u64 {
        self.reads.get()
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Amplía la imagen en memoria (tests de grow).
    pub fn grow_to_blocks(&mut self, blocks: u64) {
        self.data.resize(blocks as usize * BLOCK_SIZE, 0);
    }
}

impl BlockDevice for MemBlockDevice {
    fn block_count(&self) -> u64 {
        (self.data.len() / BLOCK_SIZE) as u64
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        #[cfg(feature = "std")]
        self.reads.set(self.reads.get() + 1);
        let off = block as usize * BLOCK_SIZE;
        let src = self.data.get(off..off + BLOCK_SIZE).ok_or(BlockError::OutOfRange)?;
        buf.copy_from_slice(src);
        Ok(())
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        let off = block as usize * BLOCK_SIZE;
        let dst = self.data.get_mut(off..off + BLOCK_SIZE).ok_or(BlockError::OutOfRange)?;
        dst.copy_from_slice(buf);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }

    fn max_blocks_per_request(&self) -> usize {
        usize::MAX
    }

    /// Una petición, una copia. Es lo que hace que `read_count()` cuente
    /// **peticiones** y no bloques, y por tanto lo que permite a los tests de
    /// host afirmar que la agrupación ocurre de verdad.
    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if buf.len() % BLOCK_SIZE != 0 {
            return Err(BlockError::OutOfRange);
        }
        #[cfg(feature = "std")]
        self.reads.set(self.reads.get() + 1);
        let off = start as usize * BLOCK_SIZE;
        let src = self
            .data
            .get(off..off + buf.len())
            .ok_or(BlockError::OutOfRange)?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

#[cfg(feature = "std")]
mod sparse_dev {
    use super::*;
    use std::collections::HashMap;

    /// Dispositivo sparse: `block_count` grande sin reservar toda la RAM.
    pub struct SparseBlockDevice {
        blocks: u64,
        data: HashMap<u64, Block>,
    }

    impl SparseBlockDevice {
        pub fn new(blocks: u64) -> Self {
            Self {
                blocks,
                data: HashMap::new(),
            }
        }
    }

    impl BlockDevice for SparseBlockDevice {
        fn block_count(&self) -> u64 {
            self.blocks
        }

        fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
            if block >= self.blocks {
                return Err(BlockError::OutOfRange);
            }
            if let Some(b) = self.data.get(&block) {
                buf.copy_from_slice(b);
            } else {
                buf.fill(0);
            }
            Ok(())
        }

        fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
            if block >= self.blocks {
                return Err(BlockError::OutOfRange);
            }
            self.data.insert(block, *buf);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            Ok(())
        }
    }
}

#[cfg(feature = "std")]
pub use sparse_dev::SparseBlockDevice;

#[cfg(feature = "std")]
mod file_dev {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::path::Path;

    /// Imagen de disco en un fichero del host (mkfs, tests de integración).
    pub struct FileBlockDevice {
        file: File,
        blocks: u64,
    }

    impl FileBlockDevice {
        pub fn create(path: &Path, blocks: u64) -> std::io::Result<Self> {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)?;
            file.set_len(blocks * BLOCK_SIZE as u64)?;
            Ok(Self { file, blocks })
        }

        pub fn open(path: &Path) -> std::io::Result<Self> {
            let file = OpenOptions::new().read(true).write(true).open(path)?;
            let blocks = file.metadata()?.len() / BLOCK_SIZE as u64;
            Ok(Self { file, blocks })
        }
    }

    impl BlockDevice for FileBlockDevice {
        fn block_count(&self) -> u64 {
            self.blocks
        }

        fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
            if block >= self.blocks {
                return Err(BlockError::OutOfRange);
            }
            self.file
                .seek(SeekFrom::Start(block * BLOCK_SIZE as u64))
                .and_then(|_| self.file.read_exact(buf))
                .map_err(|_| BlockError::Io)
        }

        fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
            if block >= self.blocks {
                return Err(BlockError::OutOfRange);
            }
            self.file
                .seek(SeekFrom::Start(block * BLOCK_SIZE as u64))
                .and_then(|_| self.file.write_all(buf))
                .map_err(|_| BlockError::Io)
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            self.file.sync_all().map_err(|_| BlockError::Io)
        }
    }
}

#[cfg(feature = "std")]
pub use file_dev::FileBlockDevice;
