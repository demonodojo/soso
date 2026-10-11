//! Aceptación reservada de R08: entidades tipográficas.

use soso_web_core::decode::decode_entities;

#[test]
fn las_tipograficas_se_decodifican() {
    assert_eq!(decode_entities("a&hellip;b"), "a\u{2026}b");
    assert_eq!(decode_entities("&mdash;&ndash;"), "\u{2014}\u{2013}");
    assert_eq!(decode_entities("&copy; 2026 &reg; &euro;5"), "\u{a9} 2026 \u{ae} \u{20ac}5");
}

#[test]
fn lo_demas_no_cambia() {
    assert_eq!(decode_entities("&foo; &amp; &lt;"), "&foo; & <");
    assert_eq!(decode_entities("&amp;hellip;"), "&hellip;");
    assert_eq!(decode_entities("&#65;&#x42;"), "AB");
}
