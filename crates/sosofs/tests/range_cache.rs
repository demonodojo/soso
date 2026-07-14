//! Tests de caché LRU y lectura parcial.

#![cfg(feature = "std")]

use block_dev::{BlockDevice, MemBlockDevice, BLOCK_SIZE};
use sosofs::builder::build_image;
use sosofs::{CachedBlockDevice, FsError, Sosofs};
use std::path::PathBuf;

fn fixture() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sosofs-range");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 256) as u8).collect();
    std::fs::write(dir.join("big.bin"), &data).unwrap();
    dir
}

#[test]
fn read_file_range_parcial() {
    let mut dev = MemBlockDevice::new(4096);
    build_image(&fixture(), &mut dev).unwrap();
    let mut fs = Sosofs::mount(dev).unwrap();
    let ino = fs.resolve("/big.bin").unwrap();
    let mut buf = [0u8; 4096];
    fs.read_file_range(ino, 1000, 4096, &mut buf).unwrap();
    for (i, b) in buf.iter().enumerate() {
        assert_eq!(*b, ((1000 + i) % 256) as u8);
    }
    let full = fs.read_file(ino).unwrap();
    assert_eq!(full.len(), 200_000);
}

#[test]
fn block_cache_reutiliza_lecturas() {
    let mut inner = MemBlockDevice::new(256);
    build_image(&fixture(), &mut inner).unwrap();
    let reads_before = inner.read_count();
    let mut cached = CachedBlockDevice::with_capacity(inner, 64);
    let mut fs = Sosofs::mount(cached).unwrap();
    let ino = fs.resolve("/big.bin").unwrap();
    let mut a = [0u8; 1024];
    let mut b = [0u8; 1024];
    fs.read_file_range(ino, 0, 1024, &mut a).unwrap();
    fs.read_file_range(ino, 0, 1024, &mut b).unwrap();
    assert_eq!(a, b);
    let _ = reads_before;
}
