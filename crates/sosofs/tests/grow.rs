//! Grow del volumen sosofs cuando la partición (o imagen sparse) es mayor.

#![cfg(feature = "std")]

use block_dev::{BLOCK_SIZE, BlockDevice, MemBlockDevice};
use sosofs::builder::build_image;
use sosofs::layout::ROOT_INODE;
use sosofs::{FsError, Sosofs};
use std::path::PathBuf;

/// Un directorio por test: los tests corren en paralelo y compartir el
/// fixture (remove_dir_all + create_dir_all) fallaba de forma intermitente.
fn fixture(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sosofs-grow-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("etc")).unwrap();
    std::fs::write(dir.join("etc/motd"), "grow test\n").unwrap();
    dir
}

#[test]
fn grow_ampliar_y_escribir() {
    let src = fixture("grow_ampliar_y_escribir");
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
    let src = fixture("grow_rechaza_mas_grande_que_dispositivo");
    let mut dev = MemBlockDevice::new(1024);
    build_image(&src, &mut dev).unwrap();
    let mut fs = Sosofs::mount(dev).unwrap();
    assert!(matches!(fs.grow_to(2048), Err(FsError::NoSpace)));
}

/// Crecer lo bastante como para que el bitmap pase de 2 a 3 bloques (cada uno cubre
/// 32768). El commit liberaba `bitmap_blocks` bloques del bitmap *viejo* usando la
/// cuenta *nueva*, así que soltaba el bloque de detrás —que es de otro— y las
/// escrituras siguientes lo pisaban: en una instalación con el disco mayor que el
/// live, `ls /var` daba error de E/S tras el primer arranque.
#[test]
fn grow_con_mas_bloques_de_bitmap_no_pisa_lo_que_hay_detras() {
    let src = fixture("grow_bitmap_crece");
    for i in 0..40 {
        let datos: Vec<u8> = (0..5000u32).map(|n| (n as u8).wrapping_add(i as u8)).collect();
        std::fs::write(src.join(format!("etc/f{i:02}.bin")), datos).unwrap();
    }
    let pack_blocks = 40_000u64;
    let large = 70_000u64;
    let mut dev = MemBlockDevice::new(pack_blocks);
    build_image(&src, &mut dev).unwrap();
    let mut fs = Sosofs::mount(dev).unwrap();
    // Volumen lleno: así los bloques de detrás del bitmap están en uso, como en un
    // rootfs de verdad después de unas cuantas generaciones.
    let mut llenos = Vec::new();
    for i in 0..400 {
        let nombre = format!("relleno{i}.bin");
        let datos = vec![(i % 251) as u8; 600_000];
        match fs.create_file(ROOT_INODE, &nombre, &datos, 1) {
            Ok(_) => llenos.push((nombre, datos[0])),
            Err(_) => break,
        }
    }
    assert!(llenos.len() > 10, "no se pudo llenar el volumen");
    // Remontar (el asignador vuelve a empezar por delante) y escribir poco: las
    // migajas libres quedan detrás del bitmap y el nuevo bitmap ocupa el sitio del viejo.
    let mut fs = Sosofs::mount(fs.into_device()).unwrap();
    for i in 0..6 {
        let _ = fs.create_file(ROOT_INODE, &format!("migaja{i}.bin"), &vec![7u8; 3000 + i * 900], 1);
    }
    let old_bitmap = fs.bitmap_blocks();
    // Lo que hay justo detrás del bitmap viejo y no es suyo: tras crecer sigue en uso.
    let detras: Vec<u64> = (old_bitmap as u64..3)
        .map(|k| fs.bitmap_start() + k)
        .filter(|b| fs.bloque_en_uso(*b))
        .collect();
    // (si no hay nada detrás, el test sigue valiendo como prueba de integridad)
    let mut dev = fs.into_device();
    dev.grow_to_blocks(large);
    let mut fs = Sosofs::mount(dev).unwrap();
    fs.grow_to(large).unwrap();
    assert!(fs.bitmap_blocks() > old_bitmap, "el caso necesita más bloques de bitmap");
    for b in &detras {
        assert!(fs.bloque_en_uso(*b), "el bloque {b}, que no era del bitmap, quedó libre tras crecer");
    }

    // Escribir después de crecer: si se soltó algo ajeno, esto lo pisa.
    for i in 0..30 {
        fs.create_file(ROOT_INODE, &format!("nuevo{i}.dat"), &vec![0xA5u8; 20_000], 1)
            .unwrap();
    }
    let dev = fs.into_device();
    let mut fs2 = Sosofs::mount(dev).unwrap();
    for i in 0..40 {
        let esperado: Vec<u8> = (0..5000u32).map(|n| (n as u8).wrapping_add(i as u8)).collect();
        let ino = fs2.resolve(&format!("/etc/f{i:02}.bin")).unwrap();
        assert_eq!(fs2.read_file(ino).unwrap(), esperado, "f{i:02}.bin corrupto");
    }
    let ino = fs2.resolve("/etc/motd").unwrap();
    assert_eq!(fs2.read_file(ino).unwrap(), b"grow test\n");
    for (nombre, byte) in &llenos {
        let ino = fs2.resolve(&format!("/{nombre}")).unwrap();
        let datos = fs2.read_file(ino).unwrap();
        assert!(datos.iter().all(|b| b == byte), "{nombre} corrupto");
    }
}
