//! Aceptación reservada de R28.

use soso_hw::fbrot::Rot;

fn p(s: &str) -> Option<Rot> {
    Rot::parse(s, 800, 1280)
}

#[test]
fn formas_de_hoy_siguen_igual() {
    assert_eq!(p("0"), Some(Rot::R0));
    assert_eq!(p("normal"), Some(Rot::R0));
    assert_eq!(p("90"), Some(Rot::R90));
    assert_eq!(p(" 180 "), Some(Rot::R180));
    assert_eq!(p("270"), Some(Rot::R270));
    assert_eq!(p("auto"), Some(Rot::R270));
    assert_eq!(Rot::parse("auto", 1920, 1080), Some(Rot::R0));
    assert_eq!(p("45"), None);
}

#[test]
fn palabras_sin_distinguir_mayusculas() {
    assert_eq!(p("AUTO"), Some(Rot::R270));
    assert_eq!(Rot::parse("Auto", 1920, 1080), Some(Rot::R0));
    assert_eq!(p("Normal"), Some(Rot::R0));
    assert_eq!(p("NORMAL"), Some(Rot::R0));
}

#[test]
fn sufijos() {
    assert_eq!(p("90°"), Some(Rot::R90));
    assert_eq!(p("270deg"), Some(Rot::R270));
    assert_eq!(p("270 deg"), Some(Rot::R270));
    assert_eq!(p("180DEG"), Some(Rot::R180));
    assert_eq!(p(" 90 ° "), Some(Rot::R90));
    assert_eq!(p("0°"), Some(Rot::R0));
}

#[test]
fn signo_y_modulo() {
    assert_eq!(p("+90"), Some(Rot::R90));
    assert_eq!(p("-90"), Some(Rot::R270));
    assert_eq!(p("-180"), Some(Rot::R180));
    assert_eq!(p("-270"), Some(Rot::R90));
    assert_eq!(p("-0"), Some(Rot::R0));
    assert_eq!(p("-360"), Some(Rot::R0));
    assert_eq!(p("360"), Some(Rot::R0));
    assert_eq!(p("450"), Some(Rot::R90));
    assert_eq!(p("540deg"), Some(Rot::R180));
    assert_eq!(p("-450°"), Some(Rot::R270));
    assert_eq!(p("3600"), Some(Rot::R0));
    assert_eq!(p("2147483647"), None); // cabe en i32 pero no es múltiplo de 90
    assert_eq!(p("-2147483648"), None);
    assert_eq!(p("2147483640"), None);
    assert_eq!(p("2147483610"), Some(Rot::R90)); // 2147483610 mod 360 = 90
}

#[test]
fn invalidos() {
    for s in [
        "", " ", "°", "deg", "deg90", "45", "91", "-45", "90.0", "9 0", "abc", "--90", "+-90",
        "90°°", "90 degrees", "99999999999", "-99999999999", "autoo", "ñ", "9°deg", "°90",
        "90é", "éé", "°°°",
    ] {
        assert_eq!(p(s), None, "{s:?}");
    }
}

#[test]
fn multibyte_no_entra_en_panico() {
    for s in ["日本語", "é", "90日", "日90", "😀😀😀", "ab°", "°deg", "dég"] {
        let _ = p(s);
    }
}
