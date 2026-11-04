//! `uso::bloques_en_uso`: copiar sólo esa cabeza de un volumen basta para
//! montarlo y leerlo entero, aunque lo de detrás sea basura.

#![cfg(feature = "std")]

use block_dev::{BLOCK_SIZE, Block, BlockDevice, MemBlockDevice};
use sosofs::builder::build_image;
use sosofs::layout::ROOT_INODE;
use sosofs::uso::bloques_en_uso;
use sosofs::Sosofs;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sosofs-uso-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("etc")).unwrap();
    std::fs::write(dir.join("etc/motd"), "uso\n").unwrap();
    dir
}

fn en_uso(dev: &mut MemBlockDevice) -> u64 {
    bloques_en_uso(|b, buf: &mut Block| dev.read_block(b, buf).is_ok()).unwrap()
}

/// Copia los primeros `n` bloques de `dev` sobre un disco del mismo tamaño
/// lleno de basura.
fn clonar_cabeza(dev: &MemBlockDevice, n: u64) -> MemBlockDevice {
    let mut otro = MemBlockDevice::new(dev.block_count());
    otro.data_mut().fill(0xA5);
    let bytes = n as usize * BLOCK_SIZE;
    otro.data_mut()[..bytes].copy_from_slice(&dev.data()[..bytes]);
    otro
}

#[test]
fn recien_construido_coincide_con_el_builder() {
    let src = fixture("recien_construido");
    let mut dev = MemBlockDevice::new(1024);
    let stats = build_image(&src, &mut dev).unwrap();
    assert_eq!(en_uso(&mut dev), stats.blocks_used);
}

#[test]
fn tras_crecer_y_escribir_la_cabeza_basta() {
    let src = fixture("crecer_y_escribir");
    let mut dev = MemBlockDevice::new(512);
    build_image(&src, &mut dev).unwrap();
    dev.grow_to_blocks(8192);
    let mut fs = Sosofs::mount(dev).unwrap();
    fs.grow_to(8192).unwrap();
    let datos = vec![0x42u8; 300 * 1024];
    fs.create_file(ROOT_INODE, "grande.dat", &datos, 1).unwrap();
    fs.create_file(ROOT_INODE, "chico.txt", b"hola", 2).unwrap();
    let mut dev = fs.into_device();

    let n = en_uso(&mut dev);
    assert!(n < 8192 / 2, "el volumen casi vacío no debería ocupar {n} bloques");
    let mut fs = Sosofs::mount(clonar_cabeza(&dev, n)).unwrap();
    assert_eq!(fs.block_count(), 8192);
    let ino = fs.resolve("/grande.dat").unwrap();
    assert_eq!(fs.read_file(ino).unwrap(), datos);
    let ino = fs.resolve("/chico.txt").unwrap();
    assert_eq!(fs.read_file(ino).unwrap(), b"hola");
    let ino = fs.resolve("/etc/motd").unwrap();
    assert_eq!(fs.read_file(ino).unwrap(), b"uso\n");
    // Y el clon sigue aceptando escrituras.
    fs.create_file(ROOT_INODE, "despues.txt", b"ok", 3).unwrap();
}

#[test]
fn sin_superbloque_no_hay_respuesta() {
    let mut dev = MemBlockDevice::new(64);
    assert_eq!(
        bloques_en_uso(|b, buf: &mut Block| dev.read_block(b, buf).is_ok()),
        None
    );
}

#[test]
fn una_lectura_fallida_no_da_respuesta() {
    let src = fixture("lectura_fallida");
    let mut dev = MemBlockDevice::new(1024);
    build_image(&src, &mut dev).unwrap();
    assert_eq!(
        bloques_en_uso(|b, buf: &mut Block| b < 2 && dev.read_block(b, buf).is_ok()),
        None
    );
}
