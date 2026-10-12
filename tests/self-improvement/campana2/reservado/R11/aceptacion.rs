//! Aceptación reservada de R11.

use soso_update_core::semver::{cmp, parse, SemVer};
use std::cmp::Ordering;

fn v(a: u32, b: u32, c: u32) -> SemVer {
    SemVer { major: a, minor: b, patch: c }
}

#[test]
fn el_prefijo_v_se_acepta() {
    assert_eq!(parse("v1.2.3"), Some(v(1, 2, 3)));
    assert_eq!(parse("V0.10.0"), Some(v(0, 10, 0)));
    assert_eq!(parse("v1.2.3-rc1"), Some(v(1, 2, 3)));
    assert_eq!(parse("v1.2.3+build5"), Some(v(1, 2, 3)));
    assert_eq!(cmp(&parse("v0.2.0").unwrap(), &parse("0.10.0").unwrap()), Ordering::Less);
}

#[test]
fn los_componentes_de_mas_se_rechazan() {
    assert_eq!(parse("1.2.3.4"), None);
    assert_eq!(parse("v1.2.3.0"), None);
    assert_eq!(parse("1.2.3.4-rc1"), None);
}

#[test]
fn lo_que_ya_era_invalido_lo_sigue_siendo() {
    for malo in ["1.2", "1..3", "a.b.c", "", "v", "vv1.2.3", "1.2.x"] {
        assert_eq!(parse(malo), None, "{malo:?}");
    }
    assert_eq!(parse("1.2.3"), Some(v(1, 2, 3)));
    assert_eq!(parse(" 1.2.3 "), Some(v(1, 2, 3)));
}
