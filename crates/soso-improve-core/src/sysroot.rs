//! Manifiesto del sysroot de `library/std` cruzada (T39 paso 3).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Entrada `(nombre.rlib, sha256 hex)`.
pub type EntradaSysroot = (String, String);

/// Lista ordenada por nombre los `.rlib` de un directorio de sysroot.
#[cfg(feature = "std")]
pub fn catalogar_rlibs(dir: &std::path::Path) -> Result<Vec<EntradaSysroot>, String> {
    if !dir.is_dir() {
        return Err(format!("no es un directorio: {}", dir.display()));
    }
    let mut out = Vec::new();
    for ent in std::fs::read_dir(dir).map_err(|e| format!("read_dir: {e}"))? {
        let ent = ent.map_err(|e| format!("read_dir ent: {e}"))?;
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rlib") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("nombre no utf8: {}", path.display()))?
            .to_string();
        let bytes = std::fs::read(&path).map_err(|e| format!("leer {}: {e}", path.display()))?;
        out.push((name, crate::sha256_hex(&bytes)));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Busca `…/rustlib/<target>/lib` bajo el árbol `build-soso` del vendor.
#[cfg(feature = "std")]
pub fn buscar_lib_sysroot(vendor: &std::path::Path, target: &str) -> Option<std::path::PathBuf> {
    let base = vendor.join("build-soso");
    if !base.is_dir() {
        return None;
    }
    let needle = format!("rustlib/{target}/lib");
    let mut candidatos = Vec::new();
    let Ok(hosts) = std::fs::read_dir(&base) else {
        return None;
    };
    for host in hosts.flatten() {
        let libroot = host.path().join("stage2/lib");
        let candidate = libroot.join(&needle);
        if candidate.is_dir() {
            candidatos.push(candidate);
        }
    }
    candidatos
        .into_iter()
        .max_by_key(|p| contar_rlibs(p).unwrap_or(0))
}

#[cfg(feature = "std")]
fn contar_rlibs(dir: &std::path::Path) -> Result<usize, ()> {
    let mut n = 0usize;
    for ent in std::fs::read_dir(dir).map_err(|_| ())? {
        let ent = ent.map_err(|_| ())?;
        if ent.path().extension().and_then(|e| e.to_str()) == Some("rlib") {
            n += 1;
        }
    }
    Ok(n)
}

/// Texto estable del manifiesto (una línea por rlib: `nombre\\tsha256`).
pub fn render_manifiesto(target: &str, entries: &[EntradaSysroot]) -> String {
    let mut s = String::new();
    s.push_str("target=");
    s.push_str(target);
    s.push('\n');
    for (name, hash) in entries {
        s.push_str(name);
        s.push('\t');
        s.push_str(hash);
        s.push('\n');
    }
    s
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn manifiesto_ordena_y_hashea() {
        let td = std::env::temp_dir().join(format!("soso-sysroot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&td);
        fs::create_dir_all(&td).unwrap();
        fs::write(td.join("libstd-abc.rlib"), b"abc").unwrap();
        fs::write(td.join("liballoc-xyz.rlib"), b"xyz").unwrap();
        fs::write(td.join("not.rmeta"), b"x").unwrap();
        let cat = catalogar_rlibs(&td).unwrap();
        assert_eq!(cat.len(), 2);
        assert_eq!(cat[0].0, "liballoc-xyz.rlib");
        let text = render_manifiesto("x86_64-unknown-soso", &cat);
        assert!(text.contains("target=x86_64-unknown-soso"));
        assert!(text.contains("libstd-abc.rlib"));
        let _ = fs::remove_dir_all(&td);
    }
}
