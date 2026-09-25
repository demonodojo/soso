//! Dónde se va el tiempo de un `stat` — paso 1 de
//! [N-012](../../../docs/self-improvement/native/N-012.md).
//!
//! En el guest, `stat` cuesta ~17 M de ciclos para una ruta de cinco
//! componentes y ~1,7 M para `/`. Un `getpid` cuesta **98**, así que el coste
//! no es entrar al kernel: está aquí dentro.
//!
//! Esto lo reproduce **en el host y sobre RAM** (`MemBlockDevice`), que quita
//! de la ecuación el disco y el driver. Si sigue siendo caro, es trabajo de
//! CPU y se puede señalar con el dedo; si aquí es barato, lo caro era el
//! dispositivo y había que mirar en otro sitio.
//!
//! No afirma nada sobre cuánto *debería* costar: imprime los números para que
//! la ficha decida con ellos, como hizo el paso 1 de N-001.

#![cfg(feature = "std")]

use block_dev::MemBlockDevice;
use sosofs::builder::build_image;
use sosofs::layout::ROOT_INODE;
use sosofs::{CachedBlockDevice, Sosofs};
use std::path::PathBuf;
use std::time::Instant;
use zerocopy::FromBytes;

const FICHEROS: usize = 64;
const VUELTAS: usize = 200;

/// **Un directorio por test.** Los tests corren en paralelo, y compartir el
/// fixture significa que uno hace `remove_dir_all` mientras el otro construye
/// la imagen desde él: falla una vez de cada tantas y parece cosa del cambio
/// que estés probando. `tests/grow.rs` ya lo tenía escrito en este repo y lo
/// repetí igual.
fn fixture(nombre: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sosofs-coste-{nombre}"));
    let _ = std::fs::remove_dir_all(&dir);
    // Una ruta de cinco componentes, como la que se midió en el guest.
    let hondo = dir.join("var/self-improvement/probe/vigilancia");
    std::fs::create_dir_all(&hondo).unwrap();
    for i in 0..FICHEROS {
        std::fs::write(hondo.join(format!("f{i:03}.txt")), "x\n").unwrap();
    }
    dir
}

#[test]
fn coste_de_stat_sobre_ram() {
    let src = fixture("stat-ram");
    let mut dev = MemBlockDevice::new(8192);
    build_image(&src, &mut dev).unwrap();
    // Con la misma caché que usa el kernel, para medir lo que él mide.
    let mut fs = Sosofs::mount(CachedBlockDevice::with_capacity(dev, 512)).unwrap();

    let hondo = "/var/self-improvement/probe/vigilancia/f000.txt";

    // Calienta: la primera pasada paga el primer acceso a cada bloque, y lo
    // que interesa es el coste **repetido**, que es el que paga un sondeo.
    for _ in 0..8 {
        let ino = fs.resolve(hondo).unwrap();
        let _ = fs.stat_inode(ino).unwrap();
    }

    let t = Instant::now();
    for _ in 0..VUELTAS {
        let _ = fs.stat_inode(ROOT_INODE).unwrap();
    }
    let raiz = t.elapsed() / VUELTAS as u32;

    let t = Instant::now();
    for _ in 0..VUELTAS {
        let ino = fs.resolve(hondo).unwrap();
        let _ = fs.stat_inode(ino).unwrap();
    }
    let profundo = t.elapsed() / VUELTAS as u32;

    // Y el CRC suelto, que es el sospechoso: `read_node` lo recalcula sobre
    // los 4092 bytes del bloque **en cada lectura**, también cuando el bloque
    // ya estaba en la caché.
    let bloque = [0x5au8; 4092];
    let t = Instant::now();
    let mut acc = 0u32;
    for _ in 0..VUELTAS {
        acc ^= sosofs::crc32c(&bloque);
    }
    let crc = t.elapsed() / VUELTAS as u32;

    println!("coste-stat: stat_inode(raíz)            {raiz:?}");
    println!("coste-stat: resolve+stat (5 componentes) {profundo:?}");
    println!("coste-stat: crc32c de 4092 bytes         {crc:?}  (acc {acc:08x})");
    let veces = profundo.as_nanos() as f64 / crc.as_nanos().max(1) as f64;
    println!("coste-stat: el stat profundo son ~{veces:.1} CRC de bloque");
}

/// ¿El coste de resolver un nombre crece con el **tamaño del directorio**?
///
/// La sospecha viene de que el guest cuesta ~3× lo que predice el host, y la
/// diferencia entre los dos es justo esa: en el guest los directorios de la
/// ruta tienen muchas entradas y en el fixture del host tenían una.
///
/// Importa porque `lookup` (`crates/sosofs/src/fs.rs`) vuelca **todos** los
/// dirents del directorio en un `Vec` y después busca el nombre linealmente —
/// aunque la clave del dirent ya lleva `name_hash(nombre)`, que permitiría ir
/// casi directo.
#[test]
fn coste_de_lookup_por_tamano_de_directorio() {
    for entradas in [8usize, 64, 256] {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("sosofs-lookup-{entradas}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("d")).unwrap();
        for i in 0..entradas {
            std::fs::write(dir.join("d").join(format!("f{i:04}.txt")), "x\n").unwrap();
        }
        let mut dev = MemBlockDevice::new(16384);
        build_image(&dir, &mut dev).unwrap();
        let mut fs = Sosofs::mount(CachedBlockDevice::with_capacity(dev, 512)).unwrap();

        // El **último** nombre, no el primero: buscar el primero saldría
        // barato con cualquier implementación y no distinguiría nada.
        let objetivo = format!("/d/f{:04}.txt", entradas - 1);
        for _ in 0..8 {
            let _ = fs.resolve(&objetivo).unwrap();
        }
        let t = Instant::now();
        for _ in 0..VUELTAS {
            let _ = fs.resolve(&objetivo).unwrap();
        }
        let c = t.elapsed() / VUELTAS as u32;
        println!("coste-lookup: directorio de {entradas:>4} entradas → resolve {c:?}");
    }
}

/// ¿El suelo de `stat` está en **cuántos** nodos se leen o en lo que cuesta
/// cada lectura?
///
/// `stat("/")` cuesta 1,72 M de ciclos en el guest y no se movió con ninguno
/// de los dos arreglos de [N-012](../../../docs/self-improvement/native/N-012.md).
/// Es una consulta puntual sin disco, así que el tiempo está en bajar por el
/// árbol — y bajar por el árbol son N lecturas de nodo.
///
/// Se cuenta N con el contador del propio sistema de ficheros en vez de
/// deducirlo, y se compara el coste por lectura contra lo que cuestan las dos
/// cosas que `read_node` hace siempre: **reservar** 4 KiB de montón y
/// **copiar** el bloque entero, aunque ya estuviera en la caché.
#[test]
fn coste_por_lectura_de_nodo() {
    let src = fixture("lectura-nodo");
    let mut dev = MemBlockDevice::new(8192);
    build_image(&src, &mut dev).unwrap();
    let mut fs = Sosofs::mount(CachedBlockDevice::with_capacity(dev, 512)).unwrap();

    for _ in 0..8 {
        let _ = fs.stat_inode(ROOT_INODE).unwrap();
    }
    let antes = fs.nodos_leidos();
    let t = Instant::now();
    for _ in 0..VUELTAS {
        let _ = fs.stat_inode(ROOT_INODE).unwrap();
    }
    let punto = t.elapsed() / VUELTAS as u32;
    let nodos = (fs.nodos_leidos() - antes) / VUELTAS as u64;

    // Lo que `read_node` hace en cada lectura, medido suelto.
    let t = Instant::now();
    let mut suma = 0u64;
    for _ in 0..VUELTAS {
        let b: Box<[u8; 4096]> = Box::new([0; 4096]);
        suma = suma.wrapping_add(b[0] as u64);
    }
    let reservar = t.elapsed() / VUELTAS as u32;

    let origen = [7u8; 4096];
    let mut destino = Box::new([0u8; 4096]);
    let t = Instant::now();
    for _ in 0..VUELTAS {
        destino.copy_from_slice(&origen);
        suma = suma.wrapping_add(destino[0] as u64);
    }
    let copiar = t.elapsed() / VUELTAS as u32;

    println!("coste-nodo: consulta puntual {punto:?} en {nodos} lecturas de nodo");
    if nodos > 0 {
        println!("coste-nodo: por lectura de nodo {:?}", punto / nodos as u32);
    }
    println!("coste-nodo: reservar 4 KiB {reservar:?} · copiar 4 KiB {copiar:?} (suma {suma})");

    // El otro sospechoso: dentro de cada nodo, la búsqueda es **lineal** sobre
    // las claves, y un nodo interno cabe 127. Por cada una se parsea la clave
    // y además la siguiente, para saber hasta dónde llega el hijo.
    let nodo = vec![0u8; 4096];
    let t = Instant::now();
    let mut n = 0u64;
    for _ in 0..VUELTAS {
        for i in 0..127usize {
            let off = 40 + i * 24;
            if let Ok((dk, _)) = sosofs::layout::DiskKey::read_from_prefix(&nodo[off..]) {
                n = n.wrapping_add(dk.key().inode);
            }
        }
    }
    let recorrer = t.elapsed() / VUELTAS as u32;
    println!("coste-nodo: recorrer 127 claves de un nodo interno {recorrer:?} (n {n})");
}

/// Si el coste por lectura de nodo **crece con el tamaño de la caché**, el
/// tiempo no está leyendo: está *buscando* en ella.
///
/// `CachedBlockDevice` guarda cada entrada como `{block, data[4096], age}` y
/// busca recorriéndolas en orden comparando `block`. Eso significa pasear por
/// 4 KiB de memoria por cada número de bloque que compara — con 512 entradas,
/// 2 MB para encontrar uno.
///
/// La predicción, escrita antes de mirar: el coste por lectura sube con la
/// capacidad aunque el trabajo útil sea idéntico. Si no sube, la hipótesis
/// está mal y hay que buscar en otro sitio.
#[test]
fn coste_por_lectura_segun_tamano_de_cache() {
    for capacidad in [16usize, 128, 512] {
        let src = fixture(&format!("cache-{capacidad}"));
        let mut dev = MemBlockDevice::new(8192);
        build_image(&src, &mut dev).unwrap();
        let mut fs = Sosofs::mount(CachedBlockDevice::with_capacity(dev, capacidad)).unwrap();

        // Se llena la caché con bloques **distintos**. En el primer intento
        // este bucle repetía `stat_inode(ROOT)`, que toca siempre los mismos
        // dos nodos: la caché se quedaba con dos entradas y las tres
        // capacidades medían exactamente lo mismo. Un control que no controla
        // nada da tres cifras iguales y parece que refuta la hipótesis.
        for i in 0..FICHEROS {
            let ruta = format!("/var/self-improvement/probe/vigilancia/f{i:03}.txt");
            let _ = fs.resolve(&ruta);
        }
        for _ in 0..8 {
            let _ = fs.stat_inode(ROOT_INODE).unwrap();
        }
        let antes = fs.nodos_leidos();
        let t = Instant::now();
        for _ in 0..VUELTAS {
            let _ = fs.stat_inode(ROOT_INODE).unwrap();
        }
        let punto = t.elapsed() / VUELTAS as u32;
        let nodos = (fs.nodos_leidos() - antes) / VUELTAS as u64;
        println!(
            "coste-cache: capacidad {capacidad:>3} → {punto:?} por consulta ({nodos} nodos)"
        );
    }
}
