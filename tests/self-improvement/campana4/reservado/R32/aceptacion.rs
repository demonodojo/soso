//! Aceptación reservada de R32.

#![cfg(feature = "std")]

use block_dev::MemBlockDevice;
use sosofs::builder::build_image;
use sosofs::layout::ROOT_INODE;
use sosofs::{FsError, Sosofs};
use std::path::PathBuf;

fn montar(nombre: &str) -> Sosofs<MemBlockDevice> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(nombre);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a/b/c")).unwrap();
    std::fs::create_dir_all(dir.join("etc")).unwrap();
    std::fs::write(dir.join("a/b/c/hoja.txt"), "hoja").unwrap();
    std::fs::write(dir.join("a/x.txt"), "x").unwrap();
    std::fs::write(dir.join("etc/motd"), "hola").unwrap();
    let mut dev = MemBlockDevice::new(4096);
    build_image(&dir, &mut dev).unwrap();
    Sosofs::mount(dev).unwrap()
}

#[test]
fn las_rutas_sin_puntos_siguen_igual() {
    let mut fs = montar("r32-igual");
    let a = fs.resolve("/a").unwrap();
    assert_eq!(fs.resolve("a/").unwrap(), a);
    assert_eq!(fs.resolve("/./a/./b/").unwrap(), fs.resolve("/a/b").unwrap());
    assert_eq!(fs.resolve("/").unwrap(), ROOT_INODE);
    assert_eq!(fs.resolve("").unwrap(), ROOT_INODE);
    assert!(matches!(fs.resolve("/a/zzz"), Err(FsError::NotFound)));
}

#[test]
fn punto_punto_sube_al_padre() {
    let mut fs = montar("r32-sube");
    let a = fs.resolve("/a").unwrap();
    let b = fs.resolve("/a/b").unwrap();
    assert_eq!(fs.resolve("/a/b/..").unwrap(), a);
    assert_eq!(fs.resolve("/a/b/c/../..").unwrap(), a);
    assert_eq!(fs.resolve("/a/b/c/../../..").unwrap(), ROOT_INODE);
    assert_eq!(fs.resolve("/a/b/c/..").unwrap(), b);
    assert_eq!(fs.resolve("/a/b/../x.txt").unwrap(), fs.resolve("/a/x.txt").unwrap());
    assert_eq!(fs.resolve("/etc/../a/b/c/hoja.txt").unwrap(), fs.resolve("/a/b/c/hoja.txt").unwrap());
    assert_eq!(fs.resolve("/a/b/./../b/./c").unwrap(), fs.resolve("/a/b/c").unwrap());
}

#[test]
fn la_raiz_es_su_propio_padre() {
    let mut fs = montar("r32-raiz");
    assert_eq!(fs.resolve("/..").unwrap(), ROOT_INODE);
    assert_eq!(fs.resolve("/../..").unwrap(), ROOT_INODE);
    assert_eq!(fs.resolve("/../../etc").unwrap(), fs.resolve("/etc").unwrap());
    assert_eq!(fs.resolve("/a/../../..").unwrap(), ROOT_INODE);
    assert_eq!(fs.resolve("/a/../../a").unwrap(), fs.resolve("/a").unwrap());
}

#[test]
fn los_errores_se_conservan() {
    let mut fs = montar("r32-errores");
    // `..` sólo se aplica a directorios.
    assert!(matches!(fs.resolve("/a/x.txt/.."), Err(FsError::NotADir)));
    assert!(matches!(fs.resolve("/a/x.txt/../b"), Err(FsError::NotADir)));
    // Un componente inexistente falla antes de poder subir con `..`.
    assert!(matches!(fs.resolve("/nada/../a"), Err(FsError::NotFound)));
    // Tras subir, un nombre que no existe en el padre sigue siendo NotFound.
    assert!(matches!(fs.resolve("/a/b/../zzz"), Err(FsError::NotFound)));
}
