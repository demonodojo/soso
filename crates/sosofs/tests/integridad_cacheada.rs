//! Un bloque corrupto tiene que seguir siéndolo en la segunda lectura.
//!
//! [N-012](../../../docs/self-improvement/native/N-012.md) dejó de recalcular
//! el CRC de cada nodo en **cada** lectura y lo comprueba sólo cuando el
//! bloque llega del dispositivo. Eso abre un agujero evidente si se hace mal:
//! el bloque malo se quedaría en la caché y la lectura siguiente sería un
//! acierto, que se daría por bueno sin mirar. **Un error que se detecta una
//! vez y después se calla es peor que no detectarlo.**
//!
//! Este test es esa comprobación, y no es teórica: sin la llamada a
//! `invalidate_block` en `read_node`, la segunda lectura devuelve `Ok`.

#![cfg(feature = "std")]

use block_dev::{BLOCK_SIZE, Block, BlockDevice, MemBlockDevice};
use sosofs::builder::build_image;
use sosofs::layout::{ROOT_INODE, Superblock};
use sosofs::{CachedBlockDevice, FsError, Sosofs};
use std::path::PathBuf;

fn fixture() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sosofs-integridad-cacheada");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("etc")).unwrap();
    std::fs::write(dir.join("etc/motd"), "integridad\n").unwrap();
    dir
}

/// El bloque de la raíz del árbol, leído del superbloque como hace `mount`.
fn tree_root(dev: &mut MemBlockDevice) -> u64 {
    let mut mejor: Option<Superblock> = None;
    for slot in 0..2 {
        let mut buf: Box<Block> = Box::new([0; BLOCK_SIZE]);
        if dev.read_block(slot, &mut buf).is_err() {
            continue;
        }
        if let Some(sb) = Superblock::parse(&buf)
            && mejor.is_none_or(|b| sb.generation.get() > b.generation.get())
        {
            mejor = Some(sb);
        }
    }
    mejor.expect("superbloque").tree_root.get()
}

#[test]
fn un_nodo_corrupto_falla_las_dos_veces() {
    let src = fixture();
    let mut dev = MemBlockDevice::new(2048);
    build_image(&src, &mut dev).unwrap();

    // Se estropea la raíz del árbol **en el dispositivo**, antes de montar,
    // para que la caché nazca vacía y la primera lectura vaya al disco.
    let raiz = tree_root(&mut dev);
    let mut buf: Box<Block> = Box::new([0; BLOCK_SIZE]);
    dev.read_block(raiz, &mut buf).unwrap();
    // Un byte del payload, lejos del CRC que ocupa los primeros cuatro.
    buf[100] ^= 0xff;
    dev.write_block(raiz, &buf).unwrap();

    let mut fs = Sosofs::mount(CachedBlockDevice::with_capacity(dev, 512)).unwrap();

    let primera = fs.stat_inode(ROOT_INODE);
    assert!(
        matches!(primera, Err(FsError::BadChecksum { .. })),
        "la primera lectura tenía que detectar el CRC malo, dio {primera:?}"
    );

    // **La que importa.** Si el bloque malo se hubiera quedado cacheado, esto
    // sería un acierto y pasaría sin comprobar nada.
    let segunda = fs.stat_inode(ROOT_INODE);
    assert!(
        matches!(segunda, Err(FsError::BadChecksum { .. })),
        "la segunda lectura se lo tragó: el bloque corrupto se quedó en la caché ({segunda:?})"
    );
}
