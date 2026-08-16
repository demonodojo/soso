//! Montaje de sosofs (virtio-blk 0 o NVMe 0) y sosomfs (NVMe 1 / NVMe 0 / virtio-blk 1).

use crate::drivers::blkstat;
use crate::println;
use block_dev::{Block, BlockDevice, BlockError, BLOCK_SIZE};
use sosofs::{CachedBlockDevice, Sosofs};
use sosomfs::Sosomfs;
use spin::{Mutex, Once};

#[cfg(feature = "drv-virtio-blk")]
pub struct VirtioDev0;
#[cfg(feature = "drv-virtio-blk")]
pub struct VirtioDev1;

const SECTORS_PER_BLOCK: u64 = (BLOCK_SIZE / 512) as u64;

#[cfg(feature = "drv-virtio-blk")]
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

    fn max_blocks_per_request(&self) -> usize {
        sosomfs::MAX_REQ_BLOCKS
    }

    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let blk = crate::drivers::virtio_blk::BLK0.get().ok_or(BlockError::Io)?;
        blk.lock()
            .read_blocks((start * SECTORS_PER_BLOCK) as usize, buf)
            .map_err(|_| BlockError::Io)
    }
}

#[cfg(feature = "drv-virtio-blk")]
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

    fn max_blocks_per_request(&self) -> usize {
        sosomfs::MAX_REQ_BLOCKS
    }

    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let blk = crate::drivers::virtio_blk::BLK1.get().ok_or(BlockError::Io)?;
        blk.lock()
            .read_blocks((start * SECTORS_PER_BLOCK) as usize, buf)
            .map_err(|_| BlockError::Io)
    }
}

fn cache_blocks_modelos() -> usize {
    let libres = crate::mm::FRAME_ALLOC
        .get()
        .map(|a| a.lock().free_frames())
        .unwrap_or(0);
    (libres / 32).clamp(64, 2048)
}

fn grow_models_if_needed(mfs: &mut ModelsFs, part_blocks: u64) {
    if part_blocks <= mfs.total_blocks() {
        return;
    }
    let mut sb = *mfs.superblock();
    sosomfs::import::grow_superblock(&mut sb, part_blocks);
    if sosomfs::import::commit_grow(mfs.cache.volume_mut().inner_mut(), &sb).is_err() {
        println!("fs: sosomfs grow falló");
        return;
    }
    if mfs.reload_from_disk().is_err() {
        println!("fs: sosomfs reload tras grow falló");
        return;
    }
    println!(
        "fs: sosomfs grow → {} bloques (partición {})",
        mfs.total_blocks(),
        part_blocks
    );
}

/// Backend del rootfs: virtio-blk 0, NVMe 0, o partición GPT live.
pub enum RootDev {
    #[cfg(feature = "drv-virtio-blk")]
    Virtio(VirtioDev0),
    #[cfg(feature = "drv-nvme")]
    Nvme,
    #[cfg(feature = "drv-live-disk")]
    Live(crate::drivers::live_disk::LiveRootDev),
}

impl BlockDevice for RootDev {
    fn block_count(&self) -> u64 {
        match self {
            #[cfg(feature = "drv-virtio-blk")]
            RootDev::Virtio(v) => v.block_count(),
            #[cfg(feature = "drv-nvme")]
            RootDev::Nvme => crate::drivers::nvme::block_count_4k_slot(0).unwrap_or(0),
            #[cfg(feature = "drv-live-disk")]
            RootDev::Live(l) => l.block_count(),
        }
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        let t = blkstat::Peticion::empieza();
        let r = match self {
            #[cfg(feature = "drv-virtio-blk")]
            RootDev::Virtio(v) => v.read_block(block, buf),
            #[cfg(feature = "drv-nvme")]
            RootDev::Nvme => crate::drivers::nvme::read_block4k_slot(0, block, buf)
                .map_err(|_| BlockError::Io),
            #[cfg(feature = "drv-live-disk")]
            RootDev::Live(l) => l.read_block(block, buf),
        };
        t.termina(1);
        r
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        blkstat::escritura(1);
        match self {
            #[cfg(feature = "drv-virtio-blk")]
            RootDev::Virtio(v) => v.write_block(block, buf),
            #[cfg(feature = "drv-nvme")]
            RootDev::Nvme => crate::drivers::nvme::write_block4k_slot(0, block, buf)
                .map_err(|_| BlockError::Io),
            #[cfg(feature = "drv-live-disk")]
            RootDev::Live(l) => l.write_block(block, buf),
        }
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        match self {
            #[cfg(feature = "drv-virtio-blk")]
            RootDev::Virtio(v) => v.flush(),
            #[cfg(feature = "drv-nvme")]
            RootDev::Nvme => Ok(()),
            #[cfg(feature = "drv-live-disk")]
            RootDev::Live(l) => l.flush(),
        }
    }

    fn max_blocks_per_request(&self) -> usize {
        match self {
            #[cfg(feature = "drv-virtio-blk")]
            RootDev::Virtio(v) => v.max_blocks_per_request(),
            #[cfg(feature = "drv-nvme")]
            RootDev::Nvme => crate::drivers::nvme::max_blocks4k_slot(0)
                .min(sosomfs::MAX_REQ_BLOCKS),
            #[cfg(feature = "drv-live-disk")]
            RootDev::Live(l) => l.max_blocks_per_request(),
        }
    }

    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let bloques = (buf.len() / block_dev::BLOCK_SIZE) as u64;
        let t = blkstat::Peticion::empieza();
        let r = match self {
            #[cfg(feature = "drv-virtio-blk")]
            RootDev::Virtio(v) => v.read_blocks(start, buf),
            #[cfg(feature = "drv-nvme")]
            RootDev::Nvme => crate::drivers::nvme::read_blocks4k_slot(0, start, buf)
                .map_err(|_| BlockError::Io),
            #[cfg(feature = "drv-live-disk")]
            RootDev::Live(l) => l.read_blocks(start, buf),
        };
        t.termina(bloques);
        r
    }
}

/// Backend del disco de modelos: NVMe (slot 1 preferido) o virtio-blk 1.
pub enum ModelsDev {
    #[cfg(feature = "drv-nvme")]
    Nvme(usize),
    #[cfg(feature = "drv-virtio-blk")]
    Virtio(VirtioDev1),
    #[cfg(feature = "drv-live-disk")]
    Live(crate::drivers::live_disk::LiveModelsDev),
}

impl BlockDevice for ModelsDev {
    fn block_count(&self) -> u64 {
        match self {
            #[cfg(feature = "drv-nvme")]
            ModelsDev::Nvme(slot) => {
                crate::drivers::nvme::block_count_4k_slot(*slot).unwrap_or(0)
            }
            #[cfg(feature = "drv-virtio-blk")]
            ModelsDev::Virtio(v) => v.block_count(),
            #[cfg(feature = "drv-live-disk")]
            ModelsDev::Live(l) => l.block_count(),
        }
    }

    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
        let t = blkstat::Peticion::empieza();
        let r = match self {
            #[cfg(feature = "drv-nvme")]
            ModelsDev::Nvme(slot) => crate::drivers::nvme::read_block4k_slot(*slot, block, buf)
                .map_err(|_| BlockError::Io),
            #[cfg(feature = "drv-virtio-blk")]
            ModelsDev::Virtio(v) => v.read_block(block, buf),
            #[cfg(feature = "drv-live-disk")]
            ModelsDev::Live(l) => l.read_block(block, buf),
        };
        t.termina(1);
        r
    }

    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
        blkstat::escritura(1);
        match self {
            #[cfg(feature = "drv-nvme")]
            ModelsDev::Nvme(slot) => crate::drivers::nvme::write_block4k_slot(*slot, block, buf)
                .map_err(|_| BlockError::Io),
            #[cfg(feature = "drv-virtio-blk")]
            ModelsDev::Virtio(v) => v.write_block(block, buf),
            #[cfg(feature = "drv-live-disk")]
            ModelsDev::Live(l) => l.write_block(block, buf),
        }
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        match self {
            #[cfg(feature = "drv-nvme")]
            ModelsDev::Nvme(_) => Ok(()),
            #[cfg(feature = "drv-virtio-blk")]
            ModelsDev::Virtio(v) => v.flush(),
            #[cfg(feature = "drv-live-disk")]
            ModelsDev::Live(l) => l.flush(),
        }
    }

    fn max_blocks_per_request(&self) -> usize {
        match self {
            #[cfg(feature = "drv-nvme")]
            ModelsDev::Nvme(slot) => crate::drivers::nvme::max_blocks4k_slot(*slot)
                .min(sosomfs::MAX_REQ_BLOCKS),
            #[cfg(feature = "drv-virtio-blk")]
            ModelsDev::Virtio(v) => v.max_blocks_per_request(),
            #[cfg(feature = "drv-live-disk")]
            ModelsDev::Live(l) => l.max_blocks_per_request(),
        }
    }

    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let bloques = (buf.len() / block_dev::BLOCK_SIZE) as u64;
        let t = blkstat::Peticion::empieza();
        let r = match self {
            #[cfg(feature = "drv-nvme")]
            ModelsDev::Nvme(slot) => crate::drivers::nvme::read_blocks4k_slot(*slot, start, buf)
                .map_err(|_| BlockError::Io),
            #[cfg(feature = "drv-virtio-blk")]
            ModelsDev::Virtio(v) => v.read_blocks(start, buf),
            #[cfg(feature = "drv-live-disk")]
            ModelsDev::Live(l) => l.read_blocks(start, buf),
        };
        t.termina(bloques);
        r
    }
}

pub type Fs = Sosofs<CachedBlockDevice<RootDev>>;
pub type ModelsFs = Sosomfs<sosomfs::SingleDev<ModelsDev>>;

pub static FS: Once<Mutex<Fs>> = Once::new();
pub static MODELS: Once<Mutex<ModelsFs>> = Once::new();

#[cfg(feature = "drv-live-disk")]
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
        let part_blocks = models.block_count();
        let cache_blocks = cache_blocks_modelos();
        match Sosomfs::mount_with_cache(
            sosomfs::SingleDev::new(ModelsDev::Live(models)),
            cache_blocks,
        ) {
            Ok(mut mfs) => {
                grow_models_if_needed(&mut mfs, part_blocks);
                println!(
                    "fs: sosomfs live (generación {}, {} bloques, caché {})",
                    mfs.generation(),
                    mfs.total_blocks(),
                    cache_blocks
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

fn pick_root_backend() -> Option<RootDev> {
    #[cfg(feature = "drv-virtio-blk")]
    if crate::drivers::virtio_blk::BLK0.get().is_some() {
        return Some(RootDev::Virtio(VirtioDev0));
    }
    #[cfg(feature = "drv-nvme")]
    if crate::drivers::nvme::present_slot(0) {
        println!("fs: sosofs en NVMe");
        return Some(RootDev::Nvme);
    }
    None
}

fn pick_models_backend(root_on_virtio: bool) -> Option<ModelsDev> {
    #[cfg(feature = "drv-nvme")]
    if crate::drivers::nvme::present_slot(1) {
        println!("fs: sosomfs en NVMe (ctrl 1)");
        return Some(ModelsDev::Nvme(1));
    }
    #[cfg(all(feature = "drv-nvme", feature = "drv-virtio-blk"))]
    if crate::drivers::nvme::present_slot(0) && root_on_virtio {
        println!("fs: sosomfs en NVMe");
        return Some(ModelsDev::Nvme(0));
    }
    #[cfg(feature = "drv-virtio-blk")]
    if crate::drivers::virtio_blk::BLK1.get().is_some() {
        println!("fs: sosomfs en virtio-blk 1");
        return Some(ModelsDev::Virtio(VirtioDev1));
    }
    None
}

pub fn init() {
    #[cfg(feature = "drv-live-disk")]
    if crate::drivers::live_disk::active() {
        mount_live();
        return;
    }

    let Some(root_backend) = pick_root_backend() else {
        println!("fs: sin backend de bloque para sosofs");
        return;
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

    #[cfg(feature = "drv-virtio-blk")]
    let root_on_virtio = crate::drivers::virtio_blk::BLK0.get().is_some();
    #[cfg(not(feature = "drv-virtio-blk"))]
    let root_on_virtio = false;

    if let Some(dev) = pick_models_backend(root_on_virtio) {
        let part_blocks = dev.block_count();
        let cache_blocks = cache_blocks_modelos();
        match Sosomfs::mount_with_cache(sosomfs::SingleDev::new(dev), cache_blocks) {
            Ok(mut mfs) => {
                grow_models_if_needed(&mut mfs, part_blocks);
                println!(
                    "fs: sosomfs montado (generación {}, {} bloques, caché {})",
                    mfs.generation(),
                    mfs.total_blocks(),
                    cache_blocks
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

#[allow(dead_code)]
pub fn load_file_page(inode: u64, file_off: usize, page: &mut [u8; 4096]) -> Result<(), ()> {
    crate::vfs::read_file_range(inode, file_off, 4096, page).map_err(|_| ())
}

pub fn load_file_range(inode: u64, file_off: usize, out: &mut [u8]) -> Result<(), ()> {
    crate::vfs::read_file_range_direct(inode, file_off, out).map_err(|_| ())
}

pub fn load_file_range_racimo(inode: u64, file_off: usize, out: &mut [u8]) -> Result<(), ()> {
    crate::vfs::read_file_range_racimo(inode, file_off, out).map_err(|_| ())
}
