//! Aceptación reservada de R20.

use soso_update_core::semver::{max, parse, SemVer};

fn v(a: u32, b: u32, c: u32) -> SemVer {
    SemVer { major: a, minor: b, patch: c }
}

#[test]
fn display_y_vuelta() {
    assert_eq!(v(0, 10, 3).to_string(), "0.10.3");
    assert_eq!(v(1, 0, 0).to_string(), "1.0.0");
    assert_eq!(format!("{}", v(12, 34, 56)), "12.34.56");
    for s in ["0.2.0", "10.20.30", "0.0.0"] {
        assert_eq!(parse(s).unwrap().to_string(), s);
    }
}

#[test]
fn max_elige_la_mayor() {
    let l = [v(0, 2, 0), v(0, 10, 0), v(0, 9, 9)];
    assert_eq!(max(&l), Some(&v(0, 10, 0)));
    assert_eq!(max(&[v(1, 0, 0)]), Some(&v(1, 0, 0)));
    assert_eq!(max(&[]), None);
}

#[test]
fn en_un_empate_gana_la_primera() {
    let l = [v(1, 0, 0), v(2, 0, 0), v(2, 0, 0)];
    let r = max(&l).unwrap();
    assert!(std::ptr::eq(r, &l[1]), "la primera de las máximas");
}
