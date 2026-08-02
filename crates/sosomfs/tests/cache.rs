//! Caché de bloques: índice, reloj de evicción y lectura de rango.
//!
//! POR QUÉ EXISTE ESTE FICHERO. La primera versión de este índice se escribió
//! sin tests y se probó directamente en QEMU. Falló un arranque, se le echó la
//! culpa a la corrupción, y se revirtió — cuando el fallo era de otro sitio.
//! Un índice que se desincroniza de `entries` no devuelve un error: devuelve
//! **el bloque equivocado**, y eso se manifiesta a kilómetros del origen. Aquí
//! se comprueban los invariantes en cada paso, con cachés diminutas para que
//! los desalojos y las lápidas ocurran de verdad.

use block_dev::{MemBlockDevice, BlockDevice, BLOCK_SIZE};
use sosomfs::layout::{CACHE_NORMAL, CACHE_PIN, CACHE_STREAM};
use sosomfs::{BlockCache, SingleDev};

/// Disco donde el byte 0..8 de cada bloque es su propio LBA: cualquier bloque
/// servido desde la ranura equivocada se detecta al instante.
fn disco(bloques: u64) -> SingleDev<MemBlockDevice> {
    let mut dev = MemBlockDevice::new(bloques);
    for lba in 0..bloques {
        let mut b = [0u8; BLOCK_SIZE];
        b[..8].copy_from_slice(&lba.to_le_bytes());
        dev.write_block(lba, &b).unwrap();
    }
    SingleDev::new(dev)
}

fn lba_de(b: &[u8]) -> u64 {
    u64::from_le_bytes(b[..8].try_into().unwrap())
}

fn leer(c: &mut BlockCache<SingleDev<MemBlockDevice>>, lba: u64, pol: u8) -> u64 {
    let mut b = [0u8; BLOCK_SIZE];
    c.read_lba(lba, pol, &mut b).unwrap();
    lba_de(&b)
}

#[test]
fn devuelve_siempre_el_bloque_pedido_con_desalojos() {
    // Capacidad mínima (8) contra 200 bloques: se desaloja continuamente.
    let mut c = BlockCache::new(disco(200), 8);
    for vuelta in 0..5u64 {
        for lba in 0..200u64 {
            assert_eq!(leer(&mut c, lba, CACHE_NORMAL), lba, "vuelta {vuelta}, lba {lba}");
            c.comprobar_invariantes().unwrap();
        }
    }
}

#[test]
fn acceso_alterno_no_desincroniza_el_indice() {
    // Alternar entre dos zonas lejanas fuerza cadenas de sondeo largas y
    // lápidas entremezcladas.
    let mut c = BlockCache::new(disco(4096), 16);
    for i in 0..500u64 {
        let a = i % 40;
        let b = 4000 + (i % 40);
        assert_eq!(leer(&mut c, a, CACHE_NORMAL), a);
        assert_eq!(leer(&mut c, b, CACHE_STREAM), b);
    }
    c.comprobar_invariantes().unwrap();
}

#[test]
fn las_pin_no_se_desalojan_nunca() {
    let mut c = BlockCache::new(disco(500), 8);
    for lba in 0..4u64 {
        assert_eq!(leer(&mut c, lba, CACHE_PIN), lba);
    }
    // Machacar con tráfico normal: las cuatro PIN deben seguir ahí, y seguir
    // siendo correctas.
    for lba in 100..400u64 {
        assert_eq!(leer(&mut c, lba, CACHE_NORMAL), lba);
    }
    c.comprobar_invariantes().unwrap();
    let (aciertos_antes, _) = c.estadisticas();
    for lba in 0..4u64 {
        assert_eq!(leer(&mut c, lba, CACHE_PIN), lba);
    }
    let (aciertos, _) = c.estadisticas();
    assert_eq!(
        aciertos - aciertos_antes,
        4,
        "las PIN deberían seguir en caché tras 300 lecturas normales"
    );
}

#[test]
fn stream_no_desaloja_normales() {
    let mut c = BlockCache::new(disco(2000), 64);
    for lba in 0..8u64 {
        leer(&mut c, lba, CACHE_NORMAL);
    }
    for lba in 1000..1600u64 {
        leer(&mut c, lba, CACHE_STREAM);
    }
    c.comprobar_invariantes().unwrap();
    let (antes, _) = c.estadisticas();
    for lba in 0..8u64 {
        assert_eq!(leer(&mut c, lba, CACHE_NORMAL), lba);
    }
    let (ahora, _) = c.estadisticas();
    assert_eq!(
        ahora - antes,
        8,
        "el tráfico STREAM no debe desalojar entradas NORMAL"
    );
}

#[test]
fn read_range_agrupa_y_cachea() {
    let mut c = BlockCache::new(disco(1000), 256);
    let mut buf = vec![0u8; 32 * BLOCK_SIZE];

    let antes = c.volume_mut().inner().read_count();
    c.read_range(100, &mut buf, CACHE_STREAM).unwrap();
    let peticiones = c.volume_mut().inner().read_count() - antes;
    assert_eq!(peticiones, 1, "32 bloques deberían ser una sola petición");
    for i in 0..32u64 {
        assert_eq!(lba_de(&buf[i as usize * BLOCK_SIZE..]), 100 + i);
    }
    c.comprobar_invariantes().unwrap();

    // Segunda vez: todo servido de caché, cero disco.
    let antes = c.volume_mut().inner().read_count();
    let mut buf2 = vec![0u8; 32 * BLOCK_SIZE];
    c.read_range(100, &mut buf2, CACHE_STREAM).unwrap();
    assert_eq!(
        c.volume_mut().inner().read_count() - antes,
        0,
        "el rango ya cacheado no debe tocar el disco"
    );
    assert_eq!(buf, buf2);

    // Y los bloques quedaron accesibles uno a uno.
    for i in 0..32u64 {
        assert_eq!(leer(&mut c, 100 + i, CACHE_STREAM), 100 + i);
    }
    c.comprobar_invariantes().unwrap();
}

#[test]
fn read_range_parcialmente_cacheado_es_correcto() {
    let mut c = BlockCache::new(disco(1000), 256);
    // Meter en caché la mitad de en medio, y con datos ya tocados.
    for lba in 108..116u64 {
        leer(&mut c, lba, CACHE_STREAM);
    }
    let mut buf = vec![0u8; 32 * BLOCK_SIZE];
    c.read_range(100, &mut buf, CACHE_STREAM).unwrap();
    for i in 0..32u64 {
        assert_eq!(
            lba_de(&buf[i as usize * BLOCK_SIZE..]),
            100 + i,
            "bloque {i} del rango mixto"
        );
    }
    c.comprobar_invariantes().unwrap();
}

#[test]
fn read_range_bajo_presion_no_miente() {
    // Caché más pequeña que el rango: se desaloja mientras se puebla.
    let mut c = BlockCache::new(disco(1000), 8);
    for base in (0..320u64).step_by(32) {
        let mut buf = vec![0u8; 32 * BLOCK_SIZE];
        c.read_range(base, &mut buf, CACHE_STREAM).unwrap();
        for i in 0..32u64 {
            assert_eq!(lba_de(&buf[i as usize * BLOCK_SIZE..]), base + i);
        }
        c.comprobar_invariantes().unwrap();
    }
}

#[test]
fn el_indice_no_degenera_con_lba_consecutivos() {
    // Los LBA de un shard son consecutivos: si el hash no los dispersa, el
    // sondeo lineal se convierte en una lista y el índice no sirve de nada.
    // Con 512 entradas y capacidad 512 todo debe seguir encontrándose.
    let mut c = BlockCache::new(disco(4096), 512);
    for lba in 0..512u64 {
        leer(&mut c, lba, CACHE_NORMAL);
    }
    c.comprobar_invariantes().unwrap();
    let (antes, _) = c.estadisticas();
    for lba in 0..512u64 {
        assert_eq!(leer(&mut c, lba, CACHE_NORMAL), lba);
    }
    let (ahora, _) = c.estadisticas();
    assert_eq!(ahora - antes, 512, "los 512 bloques deberían seguir cacheados");
}

/// El índice tiene que **seguir funcionando** después de mucho desalojo.
///
/// Este test existe porque el resto no detectaba una `desindexar` rota: al
/// verificar `entries[v].lba == lba`, una ranura obsoleta da la respuesta
/// correcta y sólo satura la tabla. La corrección aguanta; lo que se pierde en
/// silencio es el índice, que degenera a «todo fallo» — exactamente el O(n) que
/// se venía a quitar, disfrazado de funcionamiento normal.
#[test]
fn el_indice_sobrevive_al_desalojo_continuo() {
    let mut c = BlockCache::new(disco(4096), 32);
    // Churn largo: muchas más lecturas distintas que capacidad.
    for lba in 0..2000u64 {
        leer(&mut c, lba, CACHE_NORMAL);
    }
    c.comprobar_invariantes().unwrap();
    // Las últimas leídas deben seguir residentes...
    let (antes, fallos_antes) = c.estadisticas();
    c.reiniciar_sondeos();
    for lba in 1990..2000u64 {
        assert_eq!(leer(&mut c, lba, CACHE_NORMAL), lba);
    }
    let (ahora, fallos) = c.estadisticas();
    assert_eq!(
        ahora - antes,
        10,
        "el índice ya no encuentra lo que sí está en caché ({} fallos nuevos)",
        fallos - fallos_antes
    );
    // ...y encontrarse en pocos sondeos. Esto es lo que de verdad se está
    // comprobando: una tabla saturada de ranuras obsoletas sigue acertando,
    // pero barriéndola entera. Con 64 ranuras, 10 búsquedas certeras deberían
    // costar del orden de 10-20 sondeos, no 640.
    let sondeos = c.sondeos();
    assert!(
        sondeos < 40,
        "10 aciertos han costado {sondeos} sondeos: el índice ha degenerado a \
         barrido lineal (¿bajas sin lápida, o lápidas que no se recompactan?)"
    );
}
