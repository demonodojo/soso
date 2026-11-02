//! `comprobar` y `aplicar` deciden igual frente a la release remota.
//!
//! El caso que lo motivó (ROG, 2026-10-11): placa con 0.3.10 de un build de
//! desarrollo, remota 0.3.10 publicada antes. `comprobar` decía «ya estás en
//! la última versión» y `aplicar` empezaba a bajar los binarios viejos.

use soso_update_core::oferta::{decidir, Lado, Oferta};
use soso_update_core::semver::parse;

fn lado<'a>(v: &str, build: &'a str, fecha: &'a str) -> Lado<'a> {
    Lado {
        version: parse(v).unwrap(),
        build,
        fecha,
    }
}

#[test]
fn misma_version_con_build_de_desarrollo_no_se_aplica_sola() {
    let remota = lado("0.3.10", "13dd2e766", "2026-10-11");
    let local = lado("0.3.10", "3ef326300-dirty", "2026-10-11");
    let o = decidir(&remota, &local, true);
    assert_eq!(o, Oferta::BuildDistinto);
    assert!(!o.se_aplica_sola());
}

#[test]
fn misma_version_publicada_antes_que_lo_instalado_no_se_aplica_sola() {
    let remota = lado("0.3.10", "13dd2e766", "2026-10-10");
    let local = lado("0.3.10", "3ef326300", "2026-10-11");
    assert_eq!(decidir(&remota, &local, true), Oferta::BuildDistinto);
}

#[test]
fn version_menor_no_se_aplica_sola_aunque_haya_trabajo() {
    let remota = lado("0.3.9", "aaaa", "2026-10-12");
    let local = lado("0.3.10", "bbbb", "2026-10-11");
    let o = decidir(&remota, &local, true);
    assert_eq!(o, Oferta::Antigua);
    assert!(!o.se_aplica_sola());
}

#[test]
fn version_mayor_se_aplica() {
    let remota = lado("0.3.11", "aaaa", "2026-10-01");
    let local = lado("0.3.10", "bbbb", "2026-10-11");
    let o = decidir(&remota, &local, true);
    assert_eq!(o, Oferta::Nueva);
    assert!(o.se_aplica_sola());
}

/// Kernel reflasheado in situ sobre el rootfs de esa misma release.
#[test]
fn mismo_build_con_artefactos_distintos_se_repara() {
    let remota = lado("0.3.10", "13dd2e766", "2026-10-10");
    let local = lado("0.3.10", "13dd2e766", "2026-10-10");
    let o = decidir(&remota, &local, true);
    assert_eq!(o, Oferta::Reparar);
    assert!(o.se_aplica_sola());
}

#[test]
fn republicada_un_dia_posterior_se_aplica() {
    let remota = lado("0.3.10", "13dd2e766", "2026-10-12");
    let local = lado("0.3.10", "0aa11bb22", "2026-10-10");
    let o = decidir(&remota, &local, true);
    assert_eq!(o, Oferta::Republicada);
    assert!(o.se_aplica_sola());
}

#[test]
fn sin_trabajo_esta_al_dia_aunque_cambie_el_build() {
    let remota = lado("0.3.10", "13dd2e766", "2026-10-12");
    let local = lado("0.3.10", "0aa11bb22", "2026-10-10");
    let o = decidir(&remota, &local, false);
    assert_eq!(o, Oferta::AlDia);
    assert!(!o.se_aplica_sola());
}

#[test]
fn sin_build_o_fecha_legible_no_se_presume_nada() {
    let remota = lado("0.3.10", "13dd2e766", "2026-10-12");
    assert_eq!(
        decidir(&remota, &lado("0.3.10", "", ""), true),
        Oferta::BuildDistinto
    );
    assert_eq!(
        decidir(&remota, &lado("0.3.10", "", "10/10/2026"), true),
        Oferta::BuildDistinto
    );
    assert_eq!(
        decidir(&lado("0.3.10", "", "2026-10-12"), &lado("0.3.10", "", "2026-10-10"), true),
        Oferta::Republicada
    );
}
