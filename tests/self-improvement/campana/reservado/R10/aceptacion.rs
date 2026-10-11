//! Aceptación reservada de R10: `Span::contains` y `Span::overlaps`.

use soso_update_core::plan::Span;

fn t(start: u64, end: u64) -> Span {
    Span { start, end, files: vec![] }
}

#[test]
fn contains_es_semiabierto() {
    let s = t(10, 20);
    assert!(s.contains(10) && s.contains(19));
    assert!(!s.contains(9) && !s.contains(20) && !s.contains(u64::MAX));
    assert!(!t(5, 5).contains(5), "un tramo vacío no contiene nada");
}

#[test]
fn overlaps_cuenta_bytes_compartidos() {
    assert!(t(0, 10).overlaps(&t(9, 20)));
    assert!(t(0, 10).overlaps(&t(2, 3)), "uno dentro de otro");
    assert!(t(0, 10).overlaps(&t(0, 10)));
}

#[test]
fn tocarse_no_es_solaparse_y_lo_vacio_no_solapa() {
    assert!(!t(0, 10).overlaps(&t(10, 20)));
    assert!(!t(10, 20).overlaps(&t(0, 10)));
    assert!(!t(0, 10).overlaps(&t(30, 40)));
    assert!(!t(5, 5).overlaps(&t(0, 10)) && !t(0, 10).overlaps(&t(5, 5)));
    assert!(!t(5, 5).overlaps(&t(5, 5)));
}

#[test]
fn overlaps_es_simetrico() {
    let casos = [(t(0, 10), t(5, 15)), (t(0, 10), t(10, 12)), (t(3, 4), t(0, 100)), (t(7, 7), t(0, 9))];
    for (a, b) in casos {
        assert_eq!(a.overlaps(&b), b.overlaps(&a));
    }
}
