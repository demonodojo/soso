#![cfg(feature = "std")]

use block_dev::{MemBlockDevice, BLOCK_SIZE};
use sosofs::builder::build_image;
use sosofs::layout::ROOT_INODE;
use sosofs::Sosofs;
use std::path::PathBuf;

fn fs_vacio(nombre: &str) -> Sosofs<MemBlockDevice> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(nombre);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut dev = MemBlockDevice::new(4096);
    build_image(&dir, &mut dev).unwrap();
    Sosofs::mount(dev).unwrap()
}

#[test]
fn rename_y_truncate() {
    let mut fs = fs_vacio("rename-trunc");
    let ino = fs.create_file(ROOT_INODE, "a", b"hola mundo", 100).unwrap();
    fs.rename(ROOT_INODE, "a", ROOT_INODE, "b", 101).unwrap();
    assert!(fs.lookup(ROOT_INODE, "a").is_err());
    let ino2 = fs.lookup(ROOT_INODE, "b").unwrap();
    assert_eq!(ino, ino2);
    assert_eq!(fs.read_file(ino2).unwrap(), b"hola mundo");
    fs.truncate_file(ino2, 4, 102).unwrap();
    assert_eq!(fs.read_file(ino2).unwrap(), b"hola");
}
