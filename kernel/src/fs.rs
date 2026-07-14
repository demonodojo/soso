//! Montaje de sosofs sobre el disco virtio.

use crate::println;
use block_dev::{BLOCK_SIZE, Block, BlockDevice, BlockError};
use spin::{Mutex, Once};
use sosofs::Sosofs;

/// Adaptador BlockDevice -> virtio-blk (bloques de 4 KiB = 8 sectores).
pub struct VirtioDev;

const SECTORS_PER_BLOCK: u64 = (BLOCK_SIZE / 512) as u64;

impl BlockDevice for VirtioDev {
    fn block_count(&self) -> u64 {
        crate::drivers::virtio_blk::capacity_sectors().unwrap_or(0) / SECTORS_PER_BLOCK
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        let blk = crate::drivers::virtio_blk::BLK.get().ok_or(BlockError::Io)?;
        blk.lock()
            .read_blocks((block * SECTORS_PER_BLOCK) as usize, buf)
            .map_err(|_| BlockError::Io)
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        let blk = crate::drivers::virtio_blk::BLK.get().ok_or(BlockError::Io)?;
        blk.lock()
            .write_blocks((block * SECTORS_PER_BLOCK) as usize, buf)
            .map_err(|_| BlockError::Io)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        let blk = crate::drivers::virtio_blk::BLK.get().ok_or(BlockError::Io)?;
        blk.lock().flush().map_err(|_| BlockError::Io)
    }
}

pub static FS: Once<Mutex<Sosofs<VirtioDev>>> = Once::new();

pub fn init() {
    match Sosofs::mount(VirtioDev) {
        Ok(fs) => {
            println!(
                "fs: sosofs montado (generación {}, {} bloques)",
                fs.generation(),
                fs.block_count()
            );
            FS.call_once(|| Mutex::new(fs));
        }
        Err(e) => println!("fs: sin sosofs en el disco ({e:?})"),
    }
}
