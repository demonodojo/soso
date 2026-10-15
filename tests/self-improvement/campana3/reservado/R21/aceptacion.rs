//! Aceptación reservada de R21.

use soso_http::resolve_redirect;

#[test]
fn las_absolutas_se_respetan() {
    let b = "https://huggingface.co/a/b";
    assert_eq!(resolve_redirect(b, "https://cdn.ejemplo.org/x?y=1"), "https://cdn.ejemplo.org/x?y=1");
    assert_eq!(resolve_redirect(b, "http://cdn.ejemplo.org/x"), "http://cdn.ejemplo.org/x");
}

#[test]
fn con_barra_inicial_usa_el_host_de_la_base() {
    assert_eq!(resolve_redirect("https://h.org/a/b", "/x/y"), "https://h.org/x/y");
    assert_eq!(resolve_redirect("https://h.org:8443/a/b?q=1", "/x"), "https://h.org:8443/x");
    assert_eq!(resolve_redirect("https://h.org?q=1/2", "/x"), "https://h.org/x");
    assert_eq!(resolve_redirect("https://h.org", "/x"), "https://h.org/x");
}

#[test]
fn referencia_de_red_toma_el_esquema_https() {
    assert_eq!(resolve_redirect("https://h.org/a/b", "//cdn.ejemplo.org/x.bin"), "https://cdn.ejemplo.org/x.bin");
    assert_eq!(resolve_redirect("https://h.org", "//otro.org"), "https://otro.org");
}

#[test]
fn las_relativas_cuelgan_del_directorio_de_la_base() {
    assert_eq!(resolve_redirect("https://h.org/a/b", "c"), "https://h.org/a/c");
    assert_eq!(resolve_redirect("https://h.org/a/b", "c/d.bin"), "https://h.org/a/c/d.bin");
    assert_eq!(resolve_redirect("https://h.org/a/b/", "c"), "https://h.org/a/b/c");
    assert_eq!(resolve_redirect("https://h.org/f", "g"), "https://h.org/g");
    assert_eq!(resolve_redirect("https://h.org", "g"), "https://h.org/g");
    // La query y el fragmento de la base no cuentan para localizar el directorio.
    assert_eq!(resolve_redirect("https://h.org/a/b?next=/z/w", "c"), "https://h.org/a/c");
    assert_eq!(resolve_redirect("https://h.org/a/b#/z/w", "c"), "https://h.org/a/c");
    assert_eq!(resolve_redirect("https://h.org/a?x=/y", "c"), "https://h.org/c");
}

#[test]
fn no_se_normalizan_los_puntos() {
    assert_eq!(resolve_redirect("https://h.org/a/b", "../c"), "https://h.org/a/../c");
    assert_eq!(resolve_redirect("https://h.org/a/b", "./c"), "https://h.org/a/./c");
}
