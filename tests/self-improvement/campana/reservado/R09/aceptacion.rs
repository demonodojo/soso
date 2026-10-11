//! Aceptación reservada de R09: `#` y `?` en `resolve_url`.

use soso_web_core::html::resolve_url;

const BASE: &str = "https://h/a/p.html?x=1#viejo";

#[test]
fn el_fragmento_se_sustituye_y_la_consulta_se_conserva() {
    assert_eq!(resolve_url(BASE, "#nuevo"), "https://h/a/p.html?x=1#nuevo");
    assert_eq!(resolve_url("https://h/a/p.html", "#z"), "https://h/a/p.html#z");
}

#[test]
fn la_consulta_sustituye_consulta_y_fragmento() {
    assert_eq!(resolve_url(BASE, "?y=2"), "https://h/a/p.html?y=2");
    assert_eq!(resolve_url("https://h/a/p.html", "?pagina=2"), "https://h/a/p.html?pagina=2");
}

#[test]
fn lo_demas_no_cambia() {
    assert_eq!(resolve_url(BASE, ""), BASE);
    assert_eq!(resolve_url(BASE, "https://otro/x"), "https://otro/x");
    assert_eq!(resolve_url(BASE, "/raiz"), "https://h/raiz");
}
