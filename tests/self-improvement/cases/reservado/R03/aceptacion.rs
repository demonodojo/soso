//! Aceptación reservada de R03: `header_value` no distingue mayúsculas.

use soso_http::header_value;

fn cabeceras() -> Vec<(String, String)> {
    vec![
        ("content-length".to_string(), "12".to_string()),
        ("Content-Type".to_string(), "text/plain".to_string()),
        ("X-Soso".to_string(), "1".to_string()),
    ]
}

#[test]
fn encuentra_con_cualquier_combinacion_de_mayusculas() {
    let h = cabeceras();
    assert_eq!(header_value(&h, "Content-Length"), Some("12"));
    assert_eq!(header_value(&h, "CONTENT-LENGTH"), Some("12"));
    assert_eq!(header_value(&h, "content-length"), Some("12"));
    assert_eq!(header_value(&h, "content-type"), Some("text/plain"));
    assert_eq!(header_value(&h, "x-soso"), Some("1"));
}

#[test]
fn sigue_devolviendo_none_si_no_esta() {
    let h = cabeceras();
    assert_eq!(header_value(&h, "authorization"), None);
    assert_eq!(header_value(&h, ""), None);
}

#[test]
fn devuelve_la_primera_aparicion() {
    let h = vec![
        ("etag".to_string(), "uno".to_string()),
        ("ETag".to_string(), "dos".to_string()),
    ];
    assert_eq!(header_value(&h, "ETAG"), Some("uno"));
}
