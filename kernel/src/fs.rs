//! Montaje de sosofs (virtio-blk 0 o NVMe 0) y sosomfs (NVMe 1 / NVMe 0 / virtio-blk 1).

use crate::drivers::nvme;
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

/// Backend del rootfs: virtio-blk 0, NVMe 0, o partición GPT live.
pub enum RootDev {
    Virtio(VirtioDev0),
    Nvme,
    Live(crate::drivers::live_disk::LiveRootDev),
}

impl BlockDevice for RootDev {
    fn block_count(&self) -> u64 {
        match self {
            RootDev::Virtio(v) => v.block_count(),
            RootDev::Nvme => nvme::block_count_4k_slot(0).unwrap_or(0),
            RootDev::Live(l) => l.block_count(),
        }
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        match self {
            RootDev::Virtio(v) => v.read_block(block, buf),
            RootDev::Nvme => nvme::read_block4k_slot(0, block, buf).map_err(|_| BlockError::Io),
            RootDev::Live(l) => l.read_block(block, buf),
        }
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        match self {
            RootDev::Virtio(v) => v.write_block(block, buf),
            RootDev::Nvme => nvme::write_block4k_slot(0, block, buf).map_err(|_| BlockError::Io),
            RootDev::Live(l) => l.write_block(block, buf),
        }
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        match self {
            RootDev::Virtio(v) => v.flush(),
            RootDev::Nvme => Ok(()),
            RootDev::Live(l) => l.flush(),
        }
    }
}

/// Backend del disco de modelos: NVMe (slot 1 preferido) o virtio-blk 1.
pub enum ModelsDev {
    Nvme(usize),
    Virtio(VirtioDev1),
    Live(crate::drivers::live_disk::LiveModelsDev),
}

impl BlockDevice for ModelsDev {
    fn block_count(&self) -> u64 {
        match self {
            ModelsDev::Nvme(slot) => nvme::block_count_4k_slot(*slot).unwrap_or(0),
            ModelsDev::Virtio(v) => v.block_count(),
            ModelsDev::Live(l) => l.block_count(),
        }
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        match self {
            ModelsDev::Nvme(slot) => {
                nvme::read_block4k_slot(*slot, block, buf).map_err(|_| BlockError::Io)
            }
            ModelsDev::Virtio(v) => v.read_block(block, buf),
            ModelsDev::Live(l) => l.read_block(block, buf),
        }
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        match self {
            ModelsDev::Nvme(slot) => {
                nvme::write_block4k_slot(*slot, block, buf).map_err(|_| BlockError::Io)
            }
            ModelsDev::Virtio(v) => v.write_block(block, buf),
            ModelsDev::Live(l) => l.write_block(block, buf),
        }
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        match self {
            ModelsDev::Nvme(_) => Ok(()),
            ModelsDev::Virtio(v) => v.flush(),
            ModelsDev::Live(l) => l.flush(),
        }
    }
}

pub type Fs = Sosofs<CachedBlockDevice<RootDev>>;
pub type ModelsFs = Sosomfs<sosomfs::SingleDev<ModelsDev>>;

pub static FS: Once<Mutex<Fs>> = Once::new();
pub static MODELS: Once<Mutex<ModelsFs>> = Once::new();

fn mount_live() {
    let Some(root) = crate::drivers::live_disk::root_dev() else {
        println!("fs: live sin partición root");
        return;
    };
    let cached = CachedBlockDevice::with_capacity(RootDev::Live(root), 512);
    match Sosofs::mount(cached) {
        Ok(fs) => {
            println!(
                "fs: sosofs live (generación {}, {} bloques)",
                fs.generation(),
                fs.block_count()
            );
            FS.call_once(|| Mutex::new(fs));
        }
        Err(e) => println!("fs: live sosofs falló ({e:?})"),
    }

    if let Some(models) = crate::drivers::live_disk::models_dev() {
        match Sosomfs::mount_with_cache(sosomfs::SingleDev::new(ModelsDev::Live(models)), 2048) {
            Ok(mfs) => {
                println!(
                    "fs: sosomfs live (generación {}, {} bloques)",
                    mfs.generation(),
                    mfs.total_blocks()
                );
                for m in &mfs.catalog.models {
                    println!("fs:   modelo {} ({} shards)", m.name, m.shards.len());
                }
                MODELS.call_once(|| Mutex::new(mfs));
            }
            Err(e) => println!("fs: live sosomfs falló ({e:?})"),
        }
    }
}

pub fn init() {
    if crate::drivers::live_disk::active() {
        mount_live();
        return;
    }

    let root_backend = if crate::drivers::virtio_blk::BLK0.get().is_some() {
        RootDev::Virtio(VirtioDev0)
    } else if nvme::present_slot(0) {
        println!("fs: sosofs en NVMe");
        RootDev::Nvme
    } else {
        RootDev::Virtio(VirtioDev0)
    };

    let cached = CachedBlockDevice::with_capacity(root_backend, 512);
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

    let root_on_virtio = crate::drivers::virtio_blk::BLK0.get().is_some();
    let models_backend = if nvme::present_slot(1) {
        println!("fs: sosomfs en NVMe (ctrl 1)");
        Some(ModelsDev::Nvme(1))
    } else if nvme::present_slot(0) && root_on_virtio {
        println!("fs: sosomfs en NVMe");
        Some(ModelsDev::Nvme(0))
    } else if crate::drivers::virtio_blk::BLK1.get().is_some() {
        println!("fs: sosomfs en virtio-blk 1");
        Some(ModelsDev::Virtio(VirtioDev1))
    } else {
        None
    };

    if let Some(dev) = models_backend {
        match Sosomfs::mount_with_cache(sosomfs::SingleDev::new(dev), 2048) {
            Ok(mfs) => {
                println!(
                    "fs: sosomfs montado (generación {}, {} bloques, caché 2048)",
                    mfs.generation(),
                    mfs.total_blocks()
                );
                for m in &mfs.catalog.models {
                    println!("fs:   modelo {} ({} shards)", m.name, m.shards.len());
                }
                MODELS.call_once(|| Mutex::new(mfs));
            }
            Err(e) => println!("fs: sin sosomfs ({e:?})"),
        }
    }
}

/// Carga una página de 4 KiB (delega en el VFS unificado).
#[allow(dead_code)]
pub fn load_file_page(inode: u64, file_off: usize, page: &mut [u8; 4096]) -> Result<(), ()> {
    crate::vfs::read_file_range(inode, file_off, 4096, page).map_err(|_| ())
}

/// Rellena `out` (múltiplo de bloque) desde `file_off` con lectura directa,
/// sin caché de bloques: el camino de los faults de 2 MiB del mmap.
pub fn load_file_range(inode: u64, file_off: usize, out: &mut [u8]) -> Result<(), ()> {
    crate::vfs::read_file_range_direct(inode, file_off, out).map_err(|_| ())
}
