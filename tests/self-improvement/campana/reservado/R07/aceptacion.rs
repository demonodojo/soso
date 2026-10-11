//! Aceptación reservada de R07: `Guid::parse` con llaves.

use gptdisk::Guid;

const CANON: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";

#[test]
fn con_llaves_es_igual_que_sin_ellas() {
    let a = Guid::parse(CANON).expect("canónica");
    assert_eq!(Guid::parse(&format!("{{{CANON}}}")), Some(a));
    assert_eq!(Guid::parse(&format!("{{{}}}", CANON.to_lowercase())), Some(a));
}

#[test]
fn llaves_desparejadas_o_basura_se_rechazan() {
    assert_eq!(Guid::parse(&format!("{{{CANON}")), None);
    assert_eq!(Guid::parse(&format!("{CANON}}}")), None);
    assert_eq!(Guid::parse(&format!("{{{{{CANON}}}}}")), None);
    assert_eq!(Guid::parse(&format!("{{{}}}", &CANON[1..])), None);
    assert_eq!(Guid::parse(""), None);
    assert_eq!(Guid::parse("{}"), None);
}

#[test]
fn la_forma_sin_llaves_sigue_igual_y_vuelve_por_display() {
    let g = Guid::parse(CANON).unwrap();
    assert_eq!(g.to_string(), CANON);
    assert_eq!(Guid::parse(&g.to_string()), Some(g));
}
