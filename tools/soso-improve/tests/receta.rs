//! T75: la receta Rust y `apply-patches.sh` dejan el mismo árbol.
//!
//! Las dos copias salen de la revisión fijada por T38, en temporales. El
//! vendor del usuario no se resetea ni se escribe.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use soso_improve::sistema::Host;
use soso_improve_core::receta::{self, Resultado};

const REV: &str = "32d94cc9be3f6e6c3fa1deaea9e0ab93c4980dba";

struct Temporal(PathBuf);

impl Temporal {
    fn nuevo(nombre: &str) -> Self {
        let ruta = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/self-improvement/tasks/T75")
            .join(format!(
                "{nombre}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir_all(&ruta).unwrap();
        Temporal(ruta)
    }
}

impl Drop for Temporal {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn vendor() -> PathBuf {
    if let Ok(v) = std::env::var("SOSO_RUST_VENDOR") {
        return PathBuf::from(v);
    }
    let home = std::env::var("HOME").expect("HOME");
    PathBuf::from(home).join(".cache/soso-rust-vendor")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} en {}: {e}", dir.display()));
    assert!(
        out.status.success(),
        "git {args:?} en {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn exportar(origen: &Path, dest: &Path) {
    std::fs::create_dir_all(dest).unwrap();
    let mut git_p = Command::new("git")
        .args(["-C", origen.to_str().unwrap(), "archive", "HEAD", "library/std"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let tar = Command::new("tar")
        .args(["-C", dest.to_str().unwrap(), "-xf", "-"])
        .stdin(git_p.stdout.take().unwrap())
        .status()
        .unwrap();
    let git_st = git_p.wait().unwrap();
    assert!(git_st.success() && tar.success(), "no pude exportar library/std");
}

fn plantar(raiz: &Path) {
    for rel in [
        "library/std/src/os/soso/resto.rs",
        "library/std/src/sys/pal/soso/resto.rs",
    ] {
        let p = raiz.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"resto que el espejo tiene que retirar\n").unwrap();
    }
    let ajeno = raiz.join("library/std/src/sys/args/no-tocar.rs");
    std::fs::write(&ajeno, b"modulo de upstream que no se borra\n").unwrap();
}

fn aplicar_script(raiz: &Path) {
    let script = repo().join("config/rust-soso/apply-patches.sh");
    let out = Command::new("bash")
        .arg(&script)
        .env("SOSO_RUST_VENDOR", raiz)
        .output()
        .unwrap_or_else(|e| panic!("bash {}: {e}", script.display()));
    assert!(
        out.status.success(),
        "apply-patches salió {:?}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn aplicar_receta(raiz: &Path) -> Vec<soso_improve_core::receta::Informe> {
    let r = repo();
    let pasos = receta::pasos_libstd(
        r.join("config/rust-soso").to_str().unwrap(),
        r.join("crates/soso-rt").to_str().unwrap(),
    );
    receta::aplicar(&mut Host, raiz.to_str().unwrap(), &pasos)
}

fn huellas(raiz: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let std = raiz.join("library/std");
    for ent in walkdir_simple(&std) {
        let rel = ent.strip_prefix(raiz).unwrap().to_string_lossy().replace('\\', "/");
        let datos = std::fs::read(&ent).unwrap();
        out.insert(rel, soso_improve_core::sha256_hex(&datos));
    }
    out
}

fn walkdir_simple(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pila = vec![dir.to_path_buf()];
    while let Some(d) = pila.pop() {
        for ent in std::fs::read_dir(&d).unwrap() {
            let ent = ent.unwrap().path();
            if ent.is_dir() {
                pila.push(ent);
            } else if ent.is_file() {
                out.push(ent);
            }
        }
    }
    out
}

fn assert_mismos(a: &Path, b: &Path) {
    let ha = huellas(a);
    let hb = huellas(b);
    let mut difs = Vec::new();
    for (ruta, h) in &ha {
        match hb.get(ruta) {
            None => difs.push(format!("solo en el script: {ruta}")),
            Some(o) if o != h => difs.push(format!("hash distinto: {ruta}")),
            _ => {}
        }
    }
    for ruta in hb.keys() {
        if !ha.contains_key(ruta) {
            difs.push(format!("solo en la receta: {ruta}"));
        }
    }
    assert!(difs.is_empty(), "script y receta divergen:\n{}", difs.join("\n"));
}

/// Estado que dejaba la receta anterior: copia sin borrar, `dep-of-std`,
/// bloque corto de env y el `pub mod soso` colgado del cfg de hermit.
fn estado_viejo(raiz: &Path) {
    let r = repo();
    let tree = r.join("config/rust-soso/tree");
    for rel in ["library/std/src/os/soso", "library/std/src/sys/pal/soso"] {
        copiar_sin_borrar(&tree.join(rel), &raiz.join(rel));
    }
    // Contenido obsoleto dentro de un directorio que ya existe.
    std::fs::write(
        raiz.join("library/std/src/os/soso/ffi.rs"),
        b"// contenido viejo que no vale por existir el directorio\n",
    )
    .unwrap();
    // Resto de la PAL antigua (T69): el espejo tiene que retirarlo.
    std::fs::write(
        raiz.join("library/std/src/sys/pal/soso/dl.rs"),
        b"// generacion antigua: nadie lo declara\n",
    )
    .unwrap();

    sustituir_fichero(
        &raiz.join("library/std/src/os/mod.rs"),
        "#[cfg(target_os = \"hermit\")]\npub mod hermit;",
        "#[cfg(target_os = \"hermit\")]\n#[cfg(target_os = \"soso\")]\npub mod soso;\npub mod hermit;",
    );
    sustituir_fichero(
        &raiz.join("library/std/src/os/mod.rs"),
        "    target_os = \"hermit\",\n",
        "    target_os = \"hermit\",\n    target_os = \"soso\",\n",
    );
    let env = raiz.join("library/std/src/sys/env_consts.rs");
    let mut texto = std::fs::read_to_string(&env).unwrap();
    let ancla = "#[cfg(target_os = \"hermit\")]\n";
    let corto = "\
#[cfg(target_os = \"soso\")]
pub mod os {
    pub const FAMILY: &str = \"unix\";
    pub const OS: &str = \"soso\";
}

";
    let i = texto.find(ancla).expect("ancla hermit en env_consts");
    texto.insert_str(i, corto);
    std::fs::write(&env, texto).unwrap();

    let cargo = raiz.join("library/std/Cargo.toml");
    let mut c = std::fs::read_to_string(&cargo).unwrap();
    let rt = r.join("crates/soso-rt").display().to_string();
    c.push_str(&format!(
        "\n[target.'cfg(target_os = \"soso\")'.dependencies]\nsoso-rt = {{ path = \"{rt}\", features = [\"dep-of-std\"] }}\n"
    ));
    std::fs::write(&cargo, c).unwrap();
}

fn copiar_sin_borrar(desde: &Path, hasta: &Path) {
    std::fs::create_dir_all(hasta).unwrap();
    for ent in std::fs::read_dir(desde).unwrap() {
        let ent = ent.unwrap();
        let dest = hasta.join(ent.file_name());
        if ent.path().is_dir() {
            copiar_sin_borrar(&ent.path(), &dest);
        } else {
            std::fs::copy(ent.path(), dest).unwrap();
        }
    }
}

fn sustituir_fichero(ruta: &Path, busca: &str, reemplazo: &str) {
    let texto = std::fs::read_to_string(ruta).unwrap();
    let n = texto.find(busca).unwrap_or_else(|| panic!("no está {busca:?} en {}", ruta.display()));
    let mut out = String::new();
    out.push_str(&texto[..n]);
    out.push_str(reemplazo);
    out.push_str(&texto[n + busca.len()..]);
    std::fs::write(ruta, out).unwrap();
}

#[test]
fn la_receta_iguala_al_script_sobre_la_revision_fijada() {
    let v = vendor();
    assert!(v.join("library/std").is_dir(), "falta el vendor en {}", v.display());
    let head = git(&v, &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), REV, "el lock fija {REV}");

    let tmp = Temporal::nuevo("equiv");
    let script = tmp.0.join("script");
    let rust = tmp.0.join("receta");
    exportar(&v, &script);
    exportar(&v, &rust);
    plantar(&script);
    plantar(&rust);

    aplicar_script(&script);
    let inf = aplicar_receta(&rust);
    let mal: Vec<_> = inf.iter().filter(|i| !i.ok()).collect();
    assert!(mal.is_empty(), "la receta falló: {mal:?}");
    assert_mismos(&script, &rust);
    assert!(!script.join("library/std/src/sys/pal/soso/dl.rs").exists());
    assert!(!rust.join("library/std/src/sys/pal/soso/dl.rs").exists());

    let antes = huellas(&rust);
    let segunda = aplicar_receta(&rust);
    for i in &segunda {
        assert_eq!(
            i.resultado,
            Resultado::YaEstaba,
            "segunda pasada tocó «{}»: {:?}",
            i.nombre,
            i.resultado
        );
    }
    assert_eq!(huellas(&rust), antes);

    let ajeno = std::fs::read_to_string(rust.join("library/std/src/sys/args/no-tocar.rs")).unwrap();
    assert_eq!(ajeno, "modulo de upstream que no se borra\n");
    assert!(!rust.join("library/std/src/os/soso/resto.rs").exists());
    assert!(!rust.join("library/std/src/sys/pal/soso/resto.rs").exists());
    assert!(!script.join("library/std/src/sys/pal/soso/resto.rs").exists());

    let r = repo();
    let pasos = receta::pasos_libstd(
        r.join("config/rust-soso").to_str().unwrap(),
        r.join("crates/soso-rt").to_str().unwrap(),
    );
    let informe = tmp.0.join("huella.txt");
    std::fs::write(
        &informe,
        format!("{}\n{}", receta::huella(&pasos), receta::inventario(&pasos)),
    )
    .unwrap();
    // El Drop borra el temporal, así que la huella que consume T39 se copia fuera.
    let estable = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/self-improvement/tasks/T75/huella.txt");
    std::fs::create_dir_all(estable.parent().unwrap()).unwrap();
    std::fs::copy(&informe, &estable).unwrap();
}

#[test]
fn converge_desde_el_parche_viejo() {
    let v = vendor();
    let head = git(&v, &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), REV);

    let tmp = Temporal::nuevo("viejo");
    let script = tmp.0.join("script");
    let rust = tmp.0.join("receta");
    exportar(&v, &script);
    exportar(&v, &rust);
    plantar(&script);
    plantar(&rust);
    estado_viejo(&rust);

    // Antes del arreglo, el directorio ya existe y el contenido está viejo:
    // comprobar no puede decir que el espejo ya estaba.
    let r = repo();
    let pasos = receta::pasos_libstd(
        r.join("config/rust-soso").to_str().unwrap(),
        r.join("crates/soso-rt").to_str().unwrap(),
    );
    let espejo = pasos.iter().find(|p| p.nombre == "PAL: os/soso").unwrap();
    let visto = receta::comprobar(&Host, rust.to_str().unwrap(), std::slice::from_ref(espejo));
    assert_eq!(visto[0].resultado, Resultado::Falta, "{:?}", visto[0].resultado);

    aplicar_script(&script);
    let inf = aplicar_receta(&rust);
    let mal: Vec<_> = inf.iter().filter(|i| !i.ok()).collect();
    assert!(mal.is_empty(), "no convergió: {mal:?}");
    assert_mismos(&script, &rust);
    assert!(!rust.join("library/std/src/sys/pal/soso/dl.rs").exists());
    let ffi = std::fs::read_to_string(rust.join("library/std/src/os/soso/ffi.rs")).unwrap();
    assert!(!ffi.contains("contenido viejo"));
}
