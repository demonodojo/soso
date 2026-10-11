//! Aceptación reservada de R06: `header_values` devuelve todas las apariciones.

use soso_http::header_values;

fn cabeceras() -> Vec<(String, String)> {
    [("set-cookie", "a=1"), ("content-type", "text/html"), ("Set-Cookie", "b=2"), ("SET-COOKIE", "c=3")]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn todas_en_orden_sin_distinguir_mayusculas() {
    assert_eq!(header_values(&cabeceras(), "Set-Cookie"), vec!["a=1", "b=2", "c=3"]);
}

#[test]
fn una_sola_y_ninguna() {
    assert_eq!(header_values(&cabeceras(), "content-type"), vec!["text/html"]);
    assert!(header_values(&cabeceras(), "x-no-esta").is_empty());
    assert!(header_values(&[], "x").is_empty());
}
