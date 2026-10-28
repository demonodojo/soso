use soso_pkg_core::{
    comandos::reconstruir, decidir, indice::Indice, manifest::Manifest, urls::artefacto_remoto,
    AccionInstalacion,
};
use soso_update_core::{hex_sha256, parse_semver};

fn build_demo() -> (Manifest, Vec<u8>) {
    let body = b"#!/demo\n";
    let hash = hex_sha256(body);
    let man_txt = format!(
        "SOSOPKG v1\nnombre=demo\nversion=1.0.0\nf {hash} {} bin/demo\n",
        body.len()
    );
    let man = Manifest::parse(&man_txt).unwrap();
    (man, body.to_vec())
}

#[test]
fn manifiesto_y_pack() {
    let (man, pack) = build_demo();
    man.verify_pack(&pack, 1).unwrap();
}

#[test]
fn indice_y_urls() {
    let (man, pack) = build_demo();
    let txt = man.format();
    let idx_txt = format!(
        "SOSOPKG indice v1\np demo 1.0.0 {} {} {} {}\n",
        hex_sha256(txt.as_bytes()),
        txt.len(),
        hex_sha256(&pack),
        pack.len()
    );
    let idx = Indice::parse(&idx_txt).unwrap();
    assert_eq!(idx.entradas.len(), 1);
    let url = artefacto_remoto("https://ejemplo/pkgs", "demo", "1.0.0", "paquete.txt");
    assert!(url.ends_with("demo/1.0.0/paquete.txt"));
}

#[test]
fn decidir_versiones() {
    let v1 = parse_semver("1.0.0").unwrap();
    let v2 = parse_semver("2.0.0").unwrap();
    assert_eq!(decidir(&v2, Some(&v1), false), AccionInstalacion::Actualizar);
    assert_eq!(decidir(&v1, Some(&v2), false), AccionInstalacion::RequiereForzar);
    assert_eq!(decidir(&v1, None, false), AccionInstalacion::Instalar);
    assert_eq!(decidir(&v1, Some(&v1), false), AccionInstalacion::YaInstalado);
}

#[test]
fn indice_comandos_sin_conflicto() {
    let (man, _) = build_demo();
    let idx = reconstruir(&[("demo".into(), man)], &["ls"]).unwrap();
    assert!(idx.resolver("demo").is_some());
    assert!(idx.resolver("ls").is_none());
}

#[test]
fn indice_comandos_conflicto_bin() {
    let (man, _) = build_demo();
    let r = reconstruir(&[("demo".into(), man)], &["demo"]);
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("demo"));
}
