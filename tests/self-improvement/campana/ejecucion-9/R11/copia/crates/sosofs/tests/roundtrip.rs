//! Tests de host: construir imagen desde un directorio real, montarla y
//! verificar lecturas e integridad ante corrupción arbitraria.

#![cfg(feature = "std")]

use block_dev::{BLOCK_SIZE, BlockDevice, MemBlockDevice};
use sosofs::builder::build_image;
use sosofs::layout::ROOT_INODE;
use sosofs::{FsError, Sosofs};
use std::path::PathBuf;

/// Crea un árbol de prueba en un directorio temporal del target.
fn fixture(nombre: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(nombre);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("etc")).unwrap();
    std::fs::create_dir_all(dir.join("dir1/dir2")).unwrap();
    std::fs::write(dir.join("etc/motd"), "bienvenido a soso\n").unwrap();
    std::fs::write(dir.join("hola.txt"), "hola mundo").unwrap();
    std::fs::write(dir.join("vacio"), "").unwrap();
    // Grande: cruza varios extents (>128 KiB) con contenido no trivial.
    let grande: Vec<u8> = (0..300_000u32).map(|i| (i * 31 % 251) as u8).collect();
    std::fs::write(dir.join("dir1/grande.bin"), &grande).unwrap();
    std::fs::write(dir.join("dir1/dir2/anidado"), "profundidad").unwrap();
    // Muchos ficheros: fuerza varios niveles de árbol.
    for i in 0..200 {
        std::fs::write(dir.join(format!("dir1/f{i:03}")), format!("contenido {i}")).unwrap();
    }
    dir
}

fn build(nombre: &str) -> MemBlockDevice {
    let src = fixture(nombre);
    let mut dev = MemBlockDevice::new(4096); // 16 MiB
    build_image(&src, &mut dev).unwrap();
    dev
}

#[test]
fn lee_ficheros_y_directorios() {
    let mut fs = Sosofs::mount(build("basica")).unwrap();
    assert_eq!(fs.generation(), 1);

    let motd = fs.resolve("/etc/motd").unwrap();
    assert_eq!(fs.read_file(motd).unwrap(), b"bienvenido a soso\n");

    let vacio = fs.resolve("/vacio").unwrap();
    assert_eq!(fs.read_file(vacio).unwrap(), b"");

    let anidado = fs.resolve("/dir1/dir2/anidado").unwrap();
    assert_eq!(fs.read_file(anidado).unwrap(), b"profundidad");

    let raiz = fs.read_dir(ROOT_INODE).unwrap();
    let nombres: Vec<&str> = raiz.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(nombres, ["dir1", "etc", "hola.txt", "vacio"]);

    let dir1 = fs.resolve("/dir1").unwrap();
    assert_eq!(fs.read_dir(dir1).unwrap().len(), 202); // dir2 + grande.bin + 200

    // Contenido del fichero multi-extent, byte a byte.
    let esperado: Vec<u8> = (0..300_000u32).map(|i| (i * 31 % 251) as u8).collect();
    let grande = fs.resolve("/dir1/grande.bin").unwrap();
    assert_eq!(fs.read_file(grande).unwrap(), esperado);

    for i in [0, 42, 199] {
        let f = fs.resolve(&format!("/dir1/f{i:03}")).unwrap();
        assert_eq!(fs.read_file(f).unwrap(), format!("contenido {i}").as_bytes());
    }

    assert_eq!(fs.resolve("/no/existe"), Err(FsError::NotFound));
    assert_eq!(fs.resolve("/etc/motd/x"), Err(FsError::NotADir));
}

/// Lee todo el filesystem y devuelve un resumen sensible al contenido
/// (nombres y datos), o el primer error.
fn leer_todo(fs: &mut Sosofs<MemBlockDevice>, dir: u64) -> Result<u64, FsError> {
    let mut resumen: u64 = 0;
    for (nombre, ino) in fs.read_dir(dir)? {
        resumen = resumen
            .rotate_left(7)
            .wrapping_add(sosofs::crc32c(nombre.as_bytes()) as u64);
        let st = fs.stat_inode(ino)?;
        if st.file_type == sosofs::layout::FT_DIR {
            resumen = resumen.rotate_left(7).wrapping_add(leer_todo(fs, ino)?);
        } else {
            let data = fs.read_file(ino)?;
            resumen = resumen.rotate_left(7).wrapping_add(sosofs::crc32c(&data) as u64);
        }
    }
    Ok(resumen)
}

/// La propiedad central de sosofs: voltear UN byte en CUALQUIER bloque de
/// la imagen jamás produce datos incorrectos en silencio — o la lectura
/// completa sigue siendo idéntica (bloque sin usar / bitmap / copia del
/// superbloque) o falla con error.
#[test]
fn corrupcion_nunca_pasa_desapercibida() {
    let dev = build("corrupta");
    let mut fs = Sosofs::mount(dev.clone()).unwrap();
    let referencia = leer_todo(&mut fs, ROOT_INODE).unwrap();

    let bloques = dev.block_count();
    for bloque in 0..bloques {
        let mut roto = dev.clone();
        let off = bloque as usize * BLOCK_SIZE + 1234;
        roto.data_mut()[off] ^= 0xff;

        match Sosofs::mount(roto) {
            Err(_) => {} // ambos superbloques inválidos: detectado
            Ok(mut fs) => match leer_todo(&mut fs, ROOT_INODE) {
                Err(_) => {} // detectado
                Ok(total) => assert_eq!(
                    total, referencia,
                    "bloque {bloque}: lectura distinta sin error"
                ),
            },
        }
    }
}

/// Con el superbloque A roto se monta por el B.
#[test]
fn superbloque_b_rescata() {
    let mut dev = build("sb");
    dev.data_mut()[100] ^= 0xff; // dentro del slot A
    let mut fs = Sosofs::mount(dev).unwrap();
    let motd = fs.resolve("/etc/motd").unwrap();
    assert_eq!(fs.read_file(motd).unwrap(), b"bienvenido a soso\n");
}
