//! Aceptación reservada de R04: `resolve_url` resuelve `.` y `..` (RFC 3986 §5.2.4).

use soso_web_core::html::resolve_url;

const BASE: &str = "https://ejemplo.org/a/b/pagina.html";

#[test]
fn resuelve_subidas() {
    assert_eq!(resolve_url(BASE, "../c/d.html"), "https://ejemplo.org/a/c/d.html");
    assert_eq!(resolve_url(BASE, "../../e.html"), "https://ejemplo.org/e.html");
}

#[test]
fn resuelve_puntos_simples() {
    assert_eq!(resolve_url(BASE, "./d.html"), "https://ejemplo.org/a/b/d.html");
    assert_eq!(resolve_url(BASE, "d.html"), "https://ejemplo.org/a/b/d.html");
}

#[test]
fn las_subidas_de_mas_se_descartan() {
    // RFC 3986 §5.2.4: los `..` que sobran no sacan la ruta de la raíz.
    assert_eq!(resolve_url(BASE, "../../../../x.html"), "https://ejemplo.org/x.html");
}

#[test]
fn lo_que_ya_funcionaba_sigue_igual() {
    assert_eq!(resolve_url(BASE, "https://otro/z"), "https://otro/z");
    assert_eq!(resolve_url(BASE, "/raiz.html"), "https://ejemplo.org/raiz.html");
    assert_eq!(resolve_url(BASE, "#ancla"), BASE);
    assert_eq!(resolve_url(BASE, ""), BASE);
}

#[test]
fn no_deja_segmentos_de_punto_en_la_salida() {
    for href in ["../c/d.html", "./d.html", "../../e.html"] {
        let url = resolve_url(BASE, href);
        assert!(!url.contains("/./"), "queda un '.' en {url}");
        assert!(!url.contains("/../"), "queda un '..' en {url}");
    }
}
