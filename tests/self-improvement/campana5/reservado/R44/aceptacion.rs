//! Aceptación reservada de R44.

use core::cmp::Ordering::{self, Equal, Greater, Less};
use soso_update_core::semver::{comparar, elegir_mas_nueva};

fn c(a: &str, b: &str) -> Option<Ordering> {
    comparar(a, b)
}

#[test]
fn nucleo_numerico() {
    assert_eq!(c("1.2.3", "1.2.3"), Some(Equal));
    assert_eq!(c("0.2.0", "0.10.0"), Some(Less));
    assert_eq!(c("2.0.0", "1.99.99"), Some(Greater));
    assert_eq!(c("1.2.10", "1.2.9"), Some(Greater));
    assert_eq!(c("  1.0.0 ", "1.0.0"), Some(Equal));
    assert_eq!(c("4294967295.0.0", "4294967294.9.9"), Some(Greater));
}

#[test]
fn formato_invalido() {
    for mal in [
        "", "1", "1.2", "1.2.3.4", "1.2.", ".1.2", "1..2", "a.b.c", "01.2.3", "1.02.3", "1.2.03",
        "-1.2.3", "+1.2.3", "1.2.3-", "1.2.3+", "1.2.3-a..b", "1.2.3-.a", "1.2.3-a.", "1.2.3-01",
        "1.2.3-a.01", "1.2.3-é", "1.2.3+b!", "1.2.3+a..b", "4294967296.0.0", "1.2.3 4", "1.2.3-a b",
        "v1.2.3", "1.2.3-a_b",
    ] {
        assert_eq!(c(mal, "1.0.0"), None, "{mal:?}");
        assert_eq!(c("1.0.0", mal), None, "{mal:?}");
    }
    // Válidas límite.
    for bien in ["0.0.0", "1.2.3-0", "1.2.3-0a", "1.2.3-a-b", "1.2.3+001", "1.2.3-a+b-c", "1.2.3--"] {
        assert!(c(bien, bien).is_some(), "{bien:?}");
    }
}

#[test]
fn prerelease_es_menor_que_la_release() {
    assert_eq!(c("1.0.0-rc.1", "1.0.0"), Some(Less));
    assert_eq!(c("1.0.0", "1.0.0-rc.1"), Some(Greater));
    assert_eq!(c("1.0.0-z", "0.9.9"), Some(Greater));
    assert_eq!(c("1.0.0-alpha", "1.0.1-alpha"), Some(Less));
}

#[test]
fn orden_de_identificadores_de_prerelease() {
    // Cadena canónica de la norma, estrictamente creciente.
    let orden = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
    ];
    for i in 0..orden.len() {
        for j in 0..orden.len() {
            assert_eq!(c(orden[i], orden[j]), Some(i.cmp(&j)), "{} vs {}", orden[i], orden[j]);
        }
    }
    assert_eq!(c("1.0.0-9", "1.0.0-10"), Some(Less));
    assert_eq!(
        c("1.0.0-99999999999999999999999999999", "1.0.0-100000000000000000000000000000"),
        Some(Less)
    );
    assert_eq!(c("1.0.0-1", "1.0.0-a"), Some(Less));
    assert_eq!(c("1.0.0-0a", "1.0.0-1"), Some(Greater)); // `0a` es alfanumérico
    assert_eq!(c("1.0.0-B", "1.0.0-a"), Some(Less)); // ASCII: mayúsculas antes
    assert_eq!(c("1.0.0-a-b", "1.0.0-a.b"), Some(Greater)); // un identificador `a-b` > `a`
    assert_eq!(c("1.0.0-a.b.c", "1.0.0-a.b"), Some(Greater));
}

#[test]
fn el_build_no_cuenta() {
    assert_eq!(c("1.0.0+a", "1.0.0+b"), Some(Equal));
    assert_eq!(c("1.0.0+9", "1.0.0"), Some(Equal));
    assert_eq!(c("1.0.0-rc.1+x.y", "1.0.0-rc.1+z"), Some(Equal));
    assert_eq!(c("1.0.0-rc.1+zzz", "1.0.0+aaa"), Some(Less));
    assert_eq!(c("1.0.1+0", "1.0.0+9"), Some(Greater));
}

#[test]
fn elegir_sin_prerelease() {
    let l = ["0.3.2", "0.3.10", "0.4.0-rc.1", "0.3.9", "basura", "0.4.0-rc.2"];
    assert_eq!(elegir_mas_nueva(&l, false), Some("0.3.10"));
    assert_eq!(elegir_mas_nueva(&l, true), Some("0.4.0-rc.2"));
    assert_eq!(elegir_mas_nueva(&["0.4.0-rc.1", "0.4.0"], true), Some("0.4.0"));
    assert_eq!(elegir_mas_nueva(&["0.4.0-rc.1", "0.4.0"], false), Some("0.4.0"));
}

#[test]
fn elegir_nada() {
    assert_eq!(elegir_mas_nueva(&[], true), None);
    assert_eq!(elegir_mas_nueva(&["x", "1.2", "01.0.0"], true), None);
    assert_eq!(elegir_mas_nueva(&["1.0.0-a", "2.0.0-b"], false), None);
}

#[test]
fn elegir_empates_y_forma_original() {
    // Mismo orden, otro build: gana la primera de la lista, tal cual está escrita.
    assert_eq!(elegir_mas_nueva(&["1.0.0+b", "1.0.0+a"], false), Some("1.0.0+b"));
    assert_eq!(elegir_mas_nueva(&["1.0.0+a", "1.0.0+b", "0.9.0"], false), Some("1.0.0+a"));
    assert_eq!(elegir_mas_nueva(&["0.9.0", "1.0.0+z", "1.0.0"], false), Some("1.0.0+z"));
    // Los espacios no se recortan en la devolución.
    assert_eq!(elegir_mas_nueva(&[" 1.0.0 ", "0.1.0"], false), Some(" 1.0.0 "));
    // Una inválida con aspecto de mayor no gana.
    assert_eq!(elegir_mas_nueva(&["9.9.9.9", "1.0.0"], true), Some("1.0.0"));
    assert_eq!(elegir_mas_nueva(&["1.0.0-rc.1+a", "1.0.0-rc.1+b"], true), Some("1.0.0-rc.1+a"));
}
