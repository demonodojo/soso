//! Aceptación reservada de R05: `Span::split` trocea un tramo sobredimensionado.

use soso_update_core::manifest::FileEntry;
use soso_update_core::plan::{plan_spans, Span};

fn entrada(offset: u64, size: u64) -> FileEntry {
    FileEntry {
        path: format!("f{offset}"),
        offset,
        size,
        hash_hex: "0".repeat(64),
    }
}

fn comprobar_cobertura(original: &Span, trozos: &[Span], span_max: u64) {
    assert!(!trozos.is_empty(), "un tramo siempre da al menos un trozo");
    assert_eq!(trozos[0].start, original.start, "el primer trozo empieza donde el tramo");
    assert_eq!(
        trozos[trozos.len() - 1].end,
        original.end,
        "el último trozo termina donde el tramo"
    );
    for par in trozos.windows(2) {
        assert_eq!(par[0].end, par[1].start, "los trozos deben ser contiguos y sin solape");
    }
    for t in trozos {
        assert!(t.len() <= span_max, "un trozo de {} pasa de {span_max}", t.len());
        assert!(!t.is_empty(), "no se emiten trozos vacíos");
    }
}

#[test]
fn trocea_un_fichero_mas_grande_que_el_maximo() {
    let span_max = 8 * 1024 * 1024;
    let spans = plan_spans(&[entrada(0, 20 * 1024 * 1024)], 256 * 1024, span_max);
    assert_eq!(spans.len(), 1);
    assert!(spans[0].oversized(span_max));

    let trozos = spans[0].split(span_max);
    assert_eq!(trozos.len(), 3, "20 MiB en trozos de 8 MiB son tres");
    comprobar_cobertura(&spans[0], &trozos, span_max);
    for t in &trozos {
        assert_eq!(t.files, spans[0].files, "cada trozo sigue cubriendo el mismo fichero");
    }
}

#[test]
fn un_tramo_que_cabe_se_devuelve_entero() {
    let span_max = 8 * 1024 * 1024;
    let spans = plan_spans(&[entrada(100, 1000)], 256 * 1024, span_max);
    let trozos = spans[0].split(span_max);
    assert_eq!(trozos.len(), 1);
    assert_eq!(trozos[0].start, spans[0].start);
    assert_eq!(trozos[0].end, spans[0].end);
    assert_eq!(trozos[0].files, spans[0].files);
}

#[test]
fn el_corte_exacto_no_deja_un_trozo_vacio() {
    let span_max = 1000;
    let spans = plan_spans(&[entrada(0, 2000)], 256 * 1024, span_max);
    let trozos = spans[0].split(span_max);
    assert_eq!(trozos.len(), 2);
    comprobar_cobertura(&spans[0], &trozos, span_max);
}
