//! Aceptación reservada de R13.

use soso_resize_core::PartGeom;

#[test]
fn un_tramo_invertido_tiene_cero_sectores() {
    let g = PartGeom::from_lba(100, 50);
    assert_eq!(g.sectors, 0);
    assert_eq!((g.first, g.last), (100, 50));
    assert_eq!(PartGeom::from_lba(10, 10).sectors, 1);
    assert_eq!(PartGeom::from_lba(10, 19).sectors, 10);
    assert_eq!(PartGeom::from_lba(0, 0).sectors, 1);
}

#[test]
fn contains_incluye_ambos_extremos() {
    let g = PartGeom::from_lba(10, 20);
    assert!(g.contains(10) && g.contains(20) && g.contains(15));
    assert!(!g.contains(9) && !g.contains(21));
    assert!(!PartGeom::from_lba(20, 10).contains(15), "una geometría vacía no contiene nada");
}

#[test]
fn overlaps_con_rangos_cerrados() {
    let a = PartGeom::from_lba(10, 20);
    assert!(a.overlaps(&PartGeom::from_lba(20, 30)), "comparten el sector 20");
    assert!(a.overlaps(&PartGeom::from_lba(0, 10)));
    assert!(a.overlaps(&PartGeom::from_lba(12, 14)));
    assert!(!a.overlaps(&PartGeom::from_lba(21, 30)));
    assert!(!a.overlaps(&PartGeom::from_lba(0, 9)));
    assert!(!a.overlaps(&PartGeom::from_lba(30, 5)) && !PartGeom::from_lba(30, 5).overlaps(&a));
}

#[test]
fn overlaps_es_simetrico() {
    let casos = [(PartGeom::from_lba(0, 5), PartGeom::from_lba(5, 9)), (PartGeom::from_lba(0, 5), PartGeom::from_lba(6, 9)), (PartGeom::from_lba(3, 4), PartGeom::from_lba(0, 100))];
    for (a, b) in casos {
        assert_eq!(a.overlaps(&b), b.overlaps(&a));
    }
}
