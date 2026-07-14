//! Pool lógico de volúmenes (un disco en v1, N en el futuro).

use block_dev::{Block, BlockDevice, BlockError};

pub trait VolumeSet {
    fn volume_count(&self) -> u32;
    fn total_blocks(&self) -> u64;
    fn read_lba(&mut self, lba: u64, buf: &mut Block) -> Result<(), BlockError>;
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
}
