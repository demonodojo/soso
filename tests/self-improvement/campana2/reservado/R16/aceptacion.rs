//! Aceptación reservada de R16.

use soso_web_core::decode::decode_entities;

#[test]
fn las_invalidas_dan_el_caracter_de_reemplazo() {
    assert_eq!(decode_entities("a&#0;b"), "a\u{fffd}b");
    assert_eq!(decode_entities("&#xD800;"), "\u{fffd}");
    assert_eq!(decode_entities("&#xDFFF;x"), "\u{fffd}x");
    assert_eq!(decode_entities("&#1114112;"), "\u{fffd}");
    assert_eq!(decode_entities("&#x110000;"), "\u{fffd}");
}

#[test]
fn las_validas_siguen_igual() {
    assert_eq!(decode_entities("&#65;&#x42;&#X43;"), "ABC");
    assert_eq!(decode_entities("&#128512;"), "\u{1F600}");
    assert_eq!(decode_entities("&#x10FFFF;"), "\u{10ffff}");
}

#[test]
fn las_mal_formadas_se_dejan_tal_cual() {
    assert_eq!(decode_entities("&#xZZ;"), "&#xZZ;");
    assert_eq!(decode_entities("&#;"), "&#;");
    assert_eq!(decode_entities("&#99999999999;"), "&#99999999999;");
    assert_eq!(decode_entities("&amp;"), "&");
}
