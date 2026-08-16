//! Pool lógico de volúmenes (un disco en v1, N en el futuro).

use block_dev::{Block, BlockDevice, BlockError, BLOCK_SIZE};

pub trait VolumeSet {
    fn volume_count(&self) -> u32;
    fn total_blocks(&self) -> u64;
    fn read_lba(&mut self, lba: u64, buf: &mut Block) -> Result<(), BlockError>;

    /// Máximo de bloques por petición del dispositivo de abajo.
    fn max_blocks_per_request(&self) -> usize {
        1
    }

    /// Lee un rango consecutivo de LBAs de una vez. Por defecto, el bucle.
    fn read_range_lba(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        for (i, trozo) in buf.chunks_exact_mut(BLOCK_SIZE).enumerate() {
            let b: &mut Block = trozo.try_into().map_err(|_| BlockError::OutOfRange)?;
            self.read_lba(start + i as u64, b)?;
        }
        Ok(())
    }

    /// Tras grow del superbloque, alinear tope lógico con la partición (solo `SingleDev`).
    fn refresh_total(&mut self) {}
}

pub struct SingleDev<D: BlockDevice> {
    dev: D,
    total: u64,
}

impl<D: BlockDevice> SingleDev<D> {
    pub fn new(dev: D) -> Self {
        let total = dev.block_count();
        Self { dev, total }
    }

    pub fn inner(&self) -> &D {
        &self.dev
    }

    pub fn inner_mut(&mut self) -> &mut D {
        &mut self.dev
    }

    /// Tras grow del superbloque, alinear tope lógico con la partición.
    pub fn refresh_total(&mut self) {
        self.total = self.dev.block_count();
    }
}

impl<D: BlockDevice> VolumeSet for SingleDev<D> {
    fn volume_count(&self) -> u32 {
        1
    }

    fn total_blocks(&self) -> u64 {
        self.total
    }

    fn read_lba(&mut self, lba: u64, buf: &mut Block) -> Result<(), BlockError> {
        if lba >= self.total {
            return Err(BlockError::OutOfRange);
        }
        self.dev.read_block(lba, buf)
    }

    fn max_blocks_per_request(&self) -> usize {
        self.dev.max_blocks_per_request()
    }

    /// El rango se valida **una vez**, no bloque a bloque.
    fn read_range_lba(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let bloques = (buf.len() / BLOCK_SIZE) as u64;
        if start.saturating_add(bloques) > self.total {
            return Err(BlockError::OutOfRange);
        }
        self.dev.read_blocks(start, buf)
    }

    fn refresh_total(&mut self) {
        self.total = self.dev.block_count();
    }
}
