//! Ventana deslizante real del KV (G4): expulsar filas antiguas sin cambiar las que quedan.
use soso_llm_core::kv::LayerKv;

fn fila(t: usize, d: usize) -> Vec<f32> {
    (0..d).map(|i| (t * 10 + i) as f32).collect()
}

#[test]
fn expulsar_conserva_las_filas_recientes_y_cuenta_posiciones_absolutas() {
    let d = 4;
    let mut kv = LayerKv::new();
    for t in 0..10 {
        kv.append_f16(&fila(t, d), &fila(t, d));
    }
    kv.expulsar_delante(6, d);
    assert_eq!((kv.dropped, kv.tokens(d), kv.posiciones(d)), (6, 4, 10));
    // La primera fila guardada es la posición 6.
    let k = kv.k_f16_slice();
    assert_eq!(soso_llm_core::f16::f16_to_f32(k[0]), 60.0);
    // Recortar a la posición absoluta 8 deja las posiciones 6 y 7.
    kv.truncate_tokens(8, d);
    assert_eq!((kv.tokens(d), kv.posiciones(d)), (2, 8));
    // Y se puede seguir añadiendo: la posición absoluta continúa.
    kv.append_f16(&fila(8, d), &fila(8, d));
    assert_eq!(kv.posiciones(d), 9);
    kv.reset();
    assert_eq!((kv.dropped, kv.posiciones(d)), (0, 0));
}
