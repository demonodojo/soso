//! Aceptación reservada de R46.

use soso_web_core::{percent_decode, percent_encode_component, PercentError};

#[test]
fn codifica_no_reservados_y_resto() {
    assert_eq!(percent_encode_component(""), "");
    assert_eq!(percent_encode_component("AZaz09-._~"), "AZaz09-._~");
    assert_eq!(percent_encode_component("a b"), "a%20b");
    assert_eq!(percent_encode_component("a/b?c=d&e"), "a%2Fb%3Fc%3Dd%26e");
    assert_eq!(percent_encode_component("%"), "%25");
    assert_eq!(percent_encode_component("+"), "%2B");
    assert_eq!(percent_encode_component("\n\0"), "%0A%00");
}

#[test]
fn codifica_utf8_en_mayusculas() {
    assert_eq!(percent_encode_component("é"), "%C3%A9");
    assert_eq!(percent_encode_component("ñandú €"), "%C3%B1and%C3%BA%20%E2%82%AC");
}

#[test]
fn decodifica_basico() {
    assert_eq!(percent_decode(""), Ok(String::new()));
    assert_eq!(percent_decode("abc"), Ok("abc".to_string()));
    assert_eq!(percent_decode("a%20b"), Ok("a b".to_string()));
    assert_eq!(percent_decode("%41%62"), Ok("Ab".to_string()));
    assert_eq!(percent_decode("%c3%a9"), Ok("é".to_string()));
    assert_eq!(percent_decode("%C3%A9"), Ok("é".to_string()));
    assert_eq!(percent_decode("%00"), Ok("\0".to_string()));
    assert_eq!(percent_decode("é%25"), Ok("é%".to_string()));
    assert_eq!(percent_decode("%2525"), Ok("%25".to_string()));
}

#[test]
fn el_mas_no_es_espacio() {
    assert_eq!(percent_decode("a+b"), Ok("a+b".to_string()));
    assert_eq!(percent_decode("%2b"), Ok("+".to_string()));
}

#[test]
fn escapes_mal_formados() {
    for s in ["%", "%4", "%zz", "100%", "%4g", "a%2", "%%41", "%é9"] {
        assert_eq!(percent_decode(s), Err(PercentError::BadEscape), "{s:?}");
    }
}

#[test]
fn utf8_invalido() {
    for s in ["%FF", "%C3", "a%E9b", "%C3%28"] {
        assert_eq!(percent_decode(s), Err(PercentError::InvalidUtf8), "{s:?}");
    }
}

#[test]
fn bad_escape_gana_a_invalid_utf8() {
    assert_eq!(percent_decode("%FF%"), Err(PercentError::BadEscape));
    assert_eq!(percent_decode("%FF%zz"), Err(PercentError::BadEscape));
    assert_eq!(percent_decode("%zz%FF"), Err(PercentError::BadEscape));
}

#[test]
fn ida_y_vuelta() {
    for s in ["", "hola mundo", "a/b?c=d&e=%20+", "ñandú € 日本語 🦀", "\u{0}\u{7f}~._-"] {
        let enc = percent_encode_component(s);
        assert_eq!(percent_decode(&enc), Ok(s.to_string()), "{s:?}");
    }
}
