//! `append_file`: escritura en streaming sobre un fichero existente.
//!
//! POR QUÉ EXISTE. La primera versión insertaba el extent nuevo con clave =
//! tamaño actual del fichero, sin alinear a bloque. Las claves de extent son
//! offsets **alineados a bloque** —`create_file_inner` sólo emite múltiplos de
//! bloque y `read_file_range` calcula `ext_end = key.offset + bloques*4096`—,
//! así que el extent nuevo se solapaba con el anterior, la lectura contaba dos
//! veces y el kernel moría con «range end index 9 out of range for slice of
//! length 6». Como `sys_write` pasa por aquí, le pasaba a cualquier escritura
//! de tamaño no múltiplo de 4096: el caso normal, no el raro.

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
    let mut dev = MemBlockDevice::new(4096); // 16 MiB
    build_image(&dir, &mut dev).unwrap();
    Sosofs::mount(dev).unwrap()
}

/// Escribe `trozos` uno a uno y comprueba el contenido tras cada uno.
fn comprueba_append(nombre: &str, trozos: &[Vec<u8>]) {
    let mut fs = fs_vacio(nombre);
    let ino = fs.create_file(ROOT_INODE, "f", b"", 0).unwrap();
    let mut esperado: Vec<u8> = Vec::new();
    for (i, t) in trozos.iter().enumerate() {
        fs.append_file(ino, t, 0).unwrap();
        esperado.extend_from_slice(t);
        let leido = fs
            .read_file(ino)
            .unwrap_or_else(|e| panic!("lectura tras el trozo {i} ({} B): {e:?}", t.len()));
        assert_eq!(
            leido.len(),
            esperado.len(),
            "tamaño tras el trozo {i}: {} trozos de {:?} bytes",
            trozos.len(),
            trozos.iter().map(|t| t.len()).collect::<Vec<_>>()
        );
        assert_eq!(leido, esperado, "contenido tras el trozo {i}");
    }
}

fn patron(n: usize, semilla: u8) -> Vec<u8> {
    (0..n).map(|i| ((i as u32 * 31 + semilla as u32) % 251) as u8).collect()
}

#[test]
fn append_de_seis_bytes() {
    // El caso exacto que panicaba: fichero diminuto, tamaño no alineado.
    comprueba_append("ap_6", &[b"hola\n\n".to_vec()]);
}

#[test]
fn appends_pequenos_sucesivos() {
    // Cada uno deja el tamaño en una frontera distinta dentro del bloque.
    let trozos: Vec<Vec<u8>> = (0..40).map(|i| patron(7 + i % 13, i as u8)).collect();
    comprueba_append("ap_peq", &trozos);
}

#[test]
fn append_que_cruza_bloques_y_extents() {
    // Cruza el bloque de 4 KiB y el extent máximo de 128 KiB.
    let trozos = vec![
        patron(4000, 1),
        patron(200, 2),   // cruza los 4 KiB
        patron(130_000, 3), // cruza los 128 KiB de EXTENT_MAX_BLOCKS
        patron(5, 4),
    ];
    comprueba_append("ap_grande", &trozos);
}

#[test]
fn append_justo_en_frontera_de_bloque() {
    let trozos = vec![patron(BLOCK_SIZE, 1), patron(1, 2), patron(BLOCK_SIZE, 3)];
    comprueba_append("ap_frontera", &trozos);
}

#[test]
fn append_sobrevive_al_remontaje() {
    // Los extents tienen que quedar bien en disco, no sólo en la vista viva.
    let mut fs = fs_vacio("ap_remonta");
    let ino = fs.create_file(ROOT_INODE, "f", b"", 0).unwrap();
    let mut esperado = Vec::new();
    for i in 0..12u8 {
        let t = patron(3000 + i as usize * 111, i);
        fs.append_file(ino, &t, 0).unwrap();
        esperado.extend_from_slice(&t);
    }
    let dev = fs.into_device();
    let mut fs2 = Sosofs::mount(dev).unwrap();
    let ino2 = fs2.resolve("/f").unwrap();
    assert_eq!(fs2.read_file(ino2).unwrap(), esperado);
}
