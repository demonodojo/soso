//! Aceptación reservada de R45.

use soso_http::{header_value, resolve_redirect};

const BASE: &str = "https://h.org:8443/a/b/c?z=1#f";

#[test]
fn absolutas_sin_cambios() {
    assert_eq!(resolve_redirect(BASE, "https://x.org/p?q=1"), "https://x.org/p?q=1");
    assert_eq!(resolve_redirect(BASE, "http://x.org/p"), "http://x.org/p");
    assert_eq!(resolve_redirect(BASE, "  https://x.org/p \r\n"), "https://x.org/p");
}

#[test]
fn sin_esquema() {
    assert_eq!(resolve_redirect(BASE, "//cdn.h.org/f.bin"), "https://cdn.h.org/f.bin");
}

#[test]
fn ruta_absoluta() {
    assert_eq!(resolve_redirect(BASE, "/x/y"), "https://h.org:8443/x/y");
    assert_eq!(resolve_redirect("https://h.org?x=1", "/x"), "https://h.org/x");
    assert_eq!(resolve_redirect("https://h.org#f", "/x?y=2"), "https://h.org/x?y=2");
    assert_eq!(resolve_redirect(BASE, "/x/./y?q=a/../b"), "https://h.org:8443/x/y?q=a/../b");
    assert_eq!(resolve_redirect(BASE, "/x/../../y#a/./b"), "https://h.org:8443/y#a/./b");
}

#[test]
fn solo_query() {
    assert_eq!(resolve_redirect(BASE, "?p=2"), "https://h.org:8443/a/b/c?p=2");
    assert_eq!(resolve_redirect("https://h.org?x=1", "?p=2"), "https://h.org/?p=2");
    assert_eq!(resolve_redirect("https://h.org/a#f", "?p=2#g"), "https://h.org/a?p=2#g");
}

#[test]
fn ruta_relativa() {
    assert_eq!(resolve_redirect(BASE, "next/page"), "https://h.org:8443/a/b/next/page");
    assert_eq!(resolve_redirect(BASE, "./x?q=1"), "https://h.org:8443/a/b/x?q=1");
    assert_eq!(resolve_redirect("https://h.org", "x/y"), "https://h.org/x/y");
    assert_eq!(resolve_redirect("https://h.org?k=a/b", "x"), "https://h.org/x");
    assert_eq!(resolve_redirect("https://h.org/a/b/", "x"), "https://h.org/a/b/x");
    assert_eq!(resolve_redirect("https://h.org/a/b?k=1/2", "x"), "https://h.org/a/x");
}

#[test]
fn segmentos_punto() {
    assert_eq!(resolve_redirect(BASE, "../d"), "https://h.org:8443/a/d");
    assert_eq!(resolve_redirect(BASE, "../../../d"), "https://h.org:8443/d");
    assert_eq!(resolve_redirect(BASE, "../../../../../d"), "https://h.org:8443/d");
    assert_eq!(resolve_redirect(BASE, "."), "https://h.org:8443/a/b/");
    assert_eq!(resolve_redirect(BASE, ".."), "https://h.org:8443/a/");
    assert_eq!(resolve_redirect(BASE, "../.."), "https://h.org:8443/");
    assert_eq!(resolve_redirect(BASE, "d/e/../f/."), "https://h.org:8443/a/b/d/f/");
    assert_eq!(resolve_redirect(BASE, "../d?q=../x#../y"), "https://h.org:8443/a/d?q=../x#../y");
}

#[test]
fn header_value_sin_distinguir_mayusculas() {
    let h = vec![
        ("content-type".to_string(), "text/html".to_string()),
        ("X-Dup".to_string(), "uno".to_string()),
        ("x-dup".to_string(), "dos".to_string()),
        ("Location".to_string(), "/z".to_string()),
    ];
    assert_eq!(header_value(&h, "Content-Type"), Some("text/html"));
    assert_eq!(header_value(&h, "content-type"), Some("text/html"));
    assert_eq!(header_value(&h, "LOCATION"), Some("/z"));
    assert_eq!(header_value(&h, "x-dup"), Some("uno"));
    assert_eq!(header_value(&h, "X-DUP"), Some("uno"));
    assert_eq!(header_value(&h, "etag"), None);
}
