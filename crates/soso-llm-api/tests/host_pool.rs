//! `HostPool`: cada fila se procesa exactamente una vez, también con muchas llamadas seguidas,
//! con pocas filas, tras un rato ocioso (hilos dormidos) y al cerrar.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use soso_llm_api::HostPool;
use soso_llm_core::parallel::RowParallel;

fn cubre(pool: &HostPool, filas: usize) {
    let contadores: Vec<AtomicUsize> = (0..filas).map(|_| AtomicUsize::new(0)).collect();
    pool.for_rows(filas, &|a, b| {
        for i in a..b {
            contadores[i].fetch_add(1, Ordering::Relaxed);
        }
    });
    for (i, c) in contadores.iter().enumerate() {
        assert_eq!(c.load(Ordering::Relaxed), 1, "fila {i} de {filas}");
    }
}

#[test]
fn cada_fila_se_procesa_una_vez() {
    for hilos in [1usize, 2, 4, 7] {
        let pool = HostPool::new(hilos);
        for filas in [0usize, 1, 3, 4, 5, 8, 100, 1021] {
            cubre(&pool, filas);
        }
    }
}

#[test]
fn muchas_llamadas_seguidas_y_tras_dormir() {
    let pool = HostPool::new(4);
    for ronda in 0..2000 {
        cubre(&pool, 1 + ronda % 37);
    }
    // Los hilos pasan a dormir en la condvar y tienen que despertar.
    std::thread::sleep(Duration::from_millis(200));
    for ronda in 0..50 {
        cubre(&pool, 16 + ronda);
    }
}

#[test]
fn el_resultado_no_depende_del_numero_de_hilos() {
    let datos: Vec<f32> = (0..10_000).map(|i| ((i * 37) % 101) as f32 * 0.01).collect();
    let calcula = |hilos: usize| {
        let pool = HostPool::new(hilos);
        let mut salida = vec![0.0f32; datos.len()];
        let ptr = salida.as_mut_ptr() as usize;
        pool.for_rows(datos.len(), &|a, b| {
            for i in a..b {
                unsafe { *(ptr as *mut f32).add(i) = datos[i] * 2.0 + 1.0 };
            }
        });
        salida.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
    };
    let uno = calcula(1);
    for hilos in [2usize, 3, 4, 8] {
        assert_eq!(calcula(hilos), uno, "{hilos} hilos");
    }
}
