//! Aceptación reservada de R14.

use soso_update_core::{semver_cmp, FileEntry, Manifest};
use soso_update_core::semver::SemVer;

fn manifiesto() -> Manifest {
    let f = |p: &str, off: u64| FileEntry { path: p.into(), offset: off, size: 4, hash_hex: "0".repeat(64) };
    Manifest {
        version: SemVer { major: 0, minor: 1, patch: 0 },
        version_raw: "0.1.0".into(),
        build: String::new(),
        fecha: String::new(),
        kernel_hash: "0".repeat(64),
        kernel_size: 1,
        pack_hash: "0".repeat(64),
        pack_size: 8,
        files: vec![f("etc/hostname", 0), f("bin/sosh", 4)],
    }
}

#[test]
fn las_consultas_con_prefijo_se_resuelven() {
    let m = manifiesto();
    for q in ["etc/hostname", "./etc/hostname", "/etc/hostname", ".//etc/hostname", "/./etc/hostname", "././etc/hostname", "//etc/hostname"] {
        assert_eq!(m.find_file(q).map(|f| f.offset), Some(0), "{q}");
    }
    assert_eq!(m.find_file("./bin/sosh").map(|f| f.offset), Some(4));
}

#[test]
fn no_normaliza_nada_mas() {
    let m = manifiesto();
    for q in ["etc/../etc/hostname", "etc//hostname", "etc/hostname/", "ETC/hostname", "./etc/otro", "", "/", "./"] {
        assert!(m.find_file(q).is_none(), "{q:?}");
    }
    let _ = semver_cmp;
}
