//! Aceptación reservada de R01: el texto no ASCII sobrevive a `decode_entities`.
//!
//! No comprueba cómo está escrito el decodificador, solo lo que se observa:
//! las entidades se siguen resolviendo y el resto del texto llega intacto.

use soso_web_core::decode::decode_entities;
use soso_web_core::html::reflow_html;

#[test]
fn el_texto_no_ascii_llega_intacto() {
    assert_eq!(decode_entities("café ☕"), "café ☕");
    assert_eq!(decode_entities("señor &amp; señora"), "señor & señora");
    assert_eq!(decode_entities("Ñoño"), "Ñoño");
}

#[test]
fn las_entidades_se_siguen_resolviendo() {
    assert_eq!(decode_entities("a&amp;b"), "a&b");
    assert_eq!(decode_entities("&lt;p&gt;"), "<p>");
    assert_eq!(decode_entities("&quot;&apos;"), "\"'");
    assert_eq!(decode_entities("&#65;&#x41;"), "AA");
    assert_eq!(decode_entities("&#233; &#x2615;"), "é ☕");
    assert_eq!(decode_entities("&nbsp;"), "\u{a0}");
    // Lo que no es una entidad conocida se queda tal cual, sin comerse el '&'.
    assert_eq!(decode_entities("&nosoyentidad;"), "&nosoyentidad;");
    assert_eq!(decode_entities("sin punto y coma & final"), "sin punto y coma & final");
}

#[test]
fn el_reflow_conserva_el_texto_de_la_pagina() {
    let salida = reflow_html("<p>café ☕ para el señor Ñoño</p>", "https://ejemplo/", 60);
    assert!(
        salida.text.contains("café ☕ para el señor Ñoño"),
        "el reflow perdió el texto: {:?}",
        salida.text
    );
}
