//! Grow del volumen sosofs cuando la partición (o imagen sparse) es mayor.

#![cfg(feature = "std")]

use block_dev::{BLOCK_SIZE, BlockDevice, MemBlockDevice};
use sosofs::builder::build_image;
use sosofs::layout::ROOT_INODE;
use sosofs::{FsError, Sosofs};
use std::path::PathBuf;

fn fixture() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sosofs-grow-fixture");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("etc")).unwrap();
    std::fs::write(dir.join("etc/motd"), "grow test\n").unwrap();
    dir
}

#[test]
fn grow_ampliar_y_escribir() {
    let src = fixture();
    let pack_blocks = 2048u64;
    let large = pack_blocks * 4;
    let mut dev = MemBlockDevice::new(pack_blocks);
    build_image(&src, &mut dev).unwrap();

    let mut fs = Sosofs::mount(dev).unwrap();
    let before = fs.block_count();
    assert!(before <= pack_blocks);

    let mut dev = fs.into_device();
    dev.grow_to_blocks(large);
    let mut fs = Sosofs::mount(dev).unwrap();

    fs.grow_to(large).unwrap();
    assert_eq!(fs.block_count(), large);
    assert!(fs.free_blocks() > large - before);

    let ino = fs
        .create_file(ROOT_INODE, "nuevo.dat", b"post-grow", 1)
        .unwrap();
    assert_eq!(fs.read_file(ino).unwrap(), b"post-grow");

    let dev = fs.into_device();
    let mut fs2 = Sosofs::mount(dev).unwrap();
    assert_eq!(fs2.block_count(), large);
    let ino2 = fs2.resolve("/nuevo.dat").unwrap();
    assert_eq!(fs2.read_file(ino2).unwrap(), b"post-grow");
}

#[test]
fn grow_rechaza_mas_grande_que_dispositivo() {
    let src = fixture();
    let mut dev = MemBlockDevice::new(1024);
    build_image(&src, &mut dev).unwrap();
    let mut fs = Sosofs::mount(dev).unwrap();
    assert!(matches!(fs.grow_to(2048), Err(FsError::NoSpace)));
}
