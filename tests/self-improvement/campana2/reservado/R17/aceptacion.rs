//! Aceptación reservada de R17.

use soso_web_core::html::resolve_url;

#[test]
fn protocolo_relativo_toma_el_esquema_de_la_base() {
    assert_eq!(resolve_url("https://ejemplo.org/a/b.html", "//cdn.org/x.css"), "https://cdn.org/x.css");
    assert_eq!(resolve_url("http://ejemplo.org/a/b.html", "//cdn.org/x.css"), "http://cdn.org/x.css");
}

#[test]
fn raiz_relativa_funciona_con_http() {
    assert_eq!(resolve_url("http://h/a/b", "/x"), "http://h/x");
    assert_eq!(resolve_url("https://h/a/b", "/x"), "https://h/x");
    assert_eq!(resolve_url("http://h:8080/a", "/x?y=1"), "http://h:8080/x?y=1");
}

#[test]
fn lo_demas_no_cambia() {
    assert_eq!(resolve_url("https://h/a/b", "https://otro/x"), "https://otro/x");
    assert_eq!(resolve_url("https://h/a/b", "http://otro/x"), "http://otro/x");
    assert_eq!(resolve_url("https://h/a/b.html", ""), "https://h/a/b.html");
    assert_eq!(resolve_url("https://h/a/b.html", "c.html"), "https://h/a/c.html");
}
