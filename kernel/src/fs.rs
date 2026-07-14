//! Montaje de sosofs (disco 0) y sosomfs (disco 1).

use crate::println;
use block_dev::{Block, BlockDevice, BlockError, BLOCK_SIZE};
use sosofs::{CachedBlockDevice, Sosofs};
use sosomfs::Sosomfs;
use spin::{Mutex, Once};

pub struct VirtioDev0;
pub struct VirtioDev1;

const SECTORS_PER_BLOCK: u64 = (BLOCK_SIZE / 512) as u64;

impl BlockDevice for VirtioDev0 {
    fn block_count(&self) -> u64 {
        crate::drivers::virtio_blk::capacity_sectors().unwrap_or(0) / SECTORS_PER_BLOCK
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        let blk = crate::drivers::virtio_blk::BLK0.get().ok_or(BlockError::Io)?;
        blk.lock()
            .read_blocks((block * SECTORS_PER_BLOCK) as usize, buf)
            .map_err(|_| BlockError::Io)
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        let blk = crate::drivers::virtio_blk::BLK0.get().ok_or(BlockError::Io)?;
        blk.lock()
            .write_blocks((block * SECTORS_PER_BLOCK) as usize, buf)
            .map_err(|_| BlockError::Io)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        let blk = crate::drivers::virtio_blk::BLK0.get().ok_or(BlockError::Io)?;
        blk.lock().flush().map_err(|_| BlockError::Io)
    }
}

impl BlockDevice for VirtioDev1 {
    fn block_count(&self) -> u64 {
        crate::drivers::virtio_blk::capacity_sectors1().unwrap_or(0) / SECTORS_PER_BLOCK
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        let blk = crate::drivers::virtio_blk::BLK1.get().ok_or(BlockError::Io)?;
        blk.lock()
            .read_blocks((block * SECTORS_PER_BLOCK) as usize, buf)
            .map_err(|_| BlockError::Io)
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        let blk = crate::drivers::virtio_blk::BLK1.get().ok_or(BlockError::Io)?;
        blk.lock()
            .write_blocks((block * SECTORS_PER_BLOCK) as usize, buf)
            .map_err(|_| BlockError::Io)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        let blk = crate::drivers::virtio_blk::BLK1.get().ok_or(BlockError::Io)?;
        blk.lock().flush().map_err(|_| BlockError::Io)
    }
}

pub type Fs = Sosofs<CachedBlockDevice<VirtioDev0>>;
pub type ModelsFs = Sosomfs<sosomfs::SingleDev<VirtioDev1>>;

pub static FS: Once<Mutex<Fs>> = Once::new();
pub static MODELS: Once<Mutex<ModelsFs>> = Once::new();

pub fn init() {
    let cached = CachedBlockDevice::with_capacity(VirtioDev0, 512);
    match Sosofs::mount(cached) {
        Ok(fs) => {
            println!(
                "fs: sosofs montado (generación {}, {} bloques, caché 512)",
                fs.generation(),
                fs.block_count()
            );
            FS.call_once(|| Mutex::new(fs));
        }
        Err(e) => println!("fs: sin sosofs en disco 0 ({e:?})"),
    }

    if crate::drivers::virtio_blk::BLK1.get().is_some() {
        match Sosomfs::mount_with_cache(sosomfs::SingleDev::new(VirtioDev1), 2048) {
            Ok(mfs) => {
                println!(
                    "fs: sosomfs montado (generación {}, {} bloques, caché 2048)",
                    mfs.generation(),
                    mfs.total_blocks()
                );
                MODELS.call_once(|| Mutex::new(mfs));
            }
            Err(e) => println!("fs: sin sosomfs en disco 1 ({e:?})"),
        }
    }
}

/// Carga una página de 4 KiB (delega en el VFS unificado).
pub fn load_file_page(inode: u64, file_off: usize, page: &mut [u8; 4096]) -> Result<(), ()> {
    crate::vfs::read_file_range(inode, file_off, 4096, page).map_err(|_| ())
}
