//! Empaquetado host de aplicaciones (`cargo xtask pkg-pack`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

use soso_abi::ABI_VERSION;
use soso_pkg_core::comandos::reconstruir;
use soso_pkg_core::empaquetar;

// Versiones **mínimas** de cada paquete. La que se materializa la decide
// `version_destino`: si el contenido cambió y la última ya está publicada, el
// build crea la siguiente patch sola. Subir una constante a mano sirve para un
// minor o un major.
pub const SOSO_WEB_VERSION: &str = "1.0.2";
const SOSO_WEB_NOMBRE: &str = "soso-web";
pub const OPENCODE_SAL_VERSION: &str = "1.0.0";
const OPENCODE_SAL_NOMBRE: &str = "opencode-sal";
pub const SOSO_ED_VERSION: &str = "0.1.1";
/// Cliente de actualizaciones OTA, instalable/actualizable sin cambiar kernel ni rootfs.
pub const SOSO_UPDATE_PACKAGE_VERSION: &str = "0.1.2";
pub const SOSO_PKG_VERSION: &str = "0.4.2";
pub const SOSO_WM_VERSION: &str = "0.1.3";
/// Editor vi escrito en Sal (repositorio hermano `../salvi`).
pub const SALVI_VERSION: &str = "0.1.0";
const SALVI_NOMBRE: &str = "salvi";

pub fn run(args: &[String]) {
    let Some(pkg) = args.first() else {
        eprintln!(
            "uso: cargo xtask pkg-pack <soso-web|opencode-sal|soso-ed|soso-pkg|soso-update|soso-wm|salvi> [--out DIR]"
        );
        exit(2);
    };
    if pkg == OPENCODE_SAL_NOMBRE {
        run_opencode_sal(args);
        return;
    }
    if pkg == "sal" {
        eprintln!(
            "xtask pkg-pack: Sal se publica desde su propio repositorio: \
             scripts/paquete-soso.sh en ../sal (usa cargo xtask sal-paquete)"
        );
        exit(2);
    }
    if pkg == SALVI_NOMBRE {
        let root = crate::project_root();
        let out = args
            .iter()
            .skip(1)
            .find_map(|a| a.strip_prefix("--out=").map(PathBuf::from));
        let hecho = construir_salvi(&root).and_then(|data| match out {
            Some(out) => {
                let (man_text, pack, _) = empaquetar_salvi(SALVI_VERSION, &data)?;
                escribir_paquete(&out, &man_text, &pack).map(|()| out)
            }
            None => materializar_en_catalogo(&root, SALVI_NOMBRE, SALVI_VERSION, |v| {
                empaquetar_salvi(v, &data)
            })
            .map(|(v, ..)| root.join("rootfs/share/paquetes").join(SALVI_NOMBRE).join(v)),
        });
        match hecho {
            Ok(out) => {
                if let Err(e) = generar_indice(&root) {
                    eprintln!("xtask pkg-pack: no pude actualizar indice.txt: {e}");
                    exit(1);
                }
                println!("xtask pkg-pack: {}", out.display());
            }
            Err(e) => {
                eprintln!("xtask pkg-pack: {e}");
                exit(1);
            }
        }
        return;
    }
    if pkg == "soso-ed" || pkg == "soso-pkg" || pkg == "soso-update" || pkg == "soso-wm" {
        let root = crate::project_root();
        let version = match pkg.as_str() {
            "soso-ed" => SOSO_ED_VERSION,
            "soso-pkg" => SOSO_PKG_VERSION,
            "soso-update" => SOSO_UPDATE_PACKAGE_VERSION,
            _ => SOSO_WM_VERSION,
        };
        let out = args
            .iter()
            .skip(1)
            .find_map(|a| a.strip_prefix("--out=").map(PathBuf::from));
        let hecho = match out {
            Some(out) => materializar_bin_sistema(&root, pkg, version, &out).map(|()| out),
            None => materializar_bin_sistema_catalogo(&root, pkg, version),
        };
        match hecho {
            Ok(out) => {
                if let Err(e) = generar_indice(&root) {
                    eprintln!("xtask pkg-pack: no pude actualizar indice.txt: {e}");
                    exit(1);
                }
                println!("xtask pkg-pack: {}", out.display());
            }
            Err(e) => {
                eprintln!("xtask pkg-pack: {e}");
                exit(1);
            }
        }
        return;
    }
    if pkg != SOSO_WEB_NOMBRE {
        eprintln!("xtask pkg-pack: paquete desconocido {pkg:?}");
        exit(2);
    }
    let root = crate::project_root();
    let out = args
        .iter()
        .skip(1)
        .find_map(|a| a.strip_prefix("--out=").map(PathBuf::from))
        .or_else(|| {
            let mut i = args.iter().skip(1);
            if i.next()? == "--out" {
                i.next().map(PathBuf::from)
            } else {
                None
            }
        });
    let hecho = match out {
        Some(out) => materializar_soso_web(&root, &out, false).map(|()| out),
        None => recoger_soso_web(&root).and_then(|files| {
            materializar_en_catalogo(&root, SOSO_WEB_NOMBRE, SOSO_WEB_VERSION, |v| {
                empaquetar::empaquetar(SOSO_WEB_NOMBRE, v, Some(ABI_VERSION), files.clone())
                    .map_err(|e| format!("{e}"))
            })
            .map(|(v, ..)| root.join("rootfs/share/paquetes").join(SOSO_WEB_NOMBRE).join(v))
        }),
    };
    match hecho {
        Ok(out) => println!("xtask pkg-pack: {}", out.display()),
        Err(e) => {
            eprintln!("xtask pkg-pack: {e}");
            exit(1);
        }
    }
}

pub fn materializar_paquetes_sistema(root: &Path) -> Result<(), String> {
    for (nombre, version) in [
        ("soso-ed", SOSO_ED_VERSION),
        ("soso-pkg", SOSO_PKG_VERSION),
        ("soso-update", SOSO_UPDATE_PACKAGE_VERSION),
        ("soso-wm", SOSO_WM_VERSION),
    ] {
        materializar_bin_sistema_catalogo(root, nombre, version)?;
    }
    generar_indice(root)
}

/// Paquete del sistema en el catálogo, con la versión que le toque.
fn materializar_bin_sistema_catalogo(root: &Path, nombre: &str, base: &str) -> Result<PathBuf, String> {
    let data = leer_bin_sistema(root, nombre)?;
    let (version, _, _, _) = materializar_en_catalogo(root, nombre, base, |v| {
        empaquetar_bin_sistema(nombre, v, &data)
    })?;
    Ok(root.join("rootfs/share/paquetes").join(nombre).join(version))
}

// ---- versión de cada paquete ------------------------------------------------
//
// Una versión **publicada** no cambia nunca: los clientes la identifican por su
// número, y el build la reescribía en silencio cada vez que cambiaba el binario
// (la std de Rust, soso-abi…). Así se colaron soso-pkg 0.4.2, soso-update 0.1.2
// y soso-web 1.0.2 con otro contenido tras publicarse en v0.3.36.

/// Etiqueta de la última release (`v*`) alcanzable desde HEAD.
fn ultima_release(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["describe", "--tags", "--abbrev=0", "--match", "v*"])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let tag = String::from_utf8(out.stdout).ok()?;
    let tag = tag.trim();
    (!tag.is_empty()).then(|| tag.to_string())
}

fn git_fichero(root: &Path, rev: &str, ruta: &str) -> Option<Vec<u8>> {
    let out = std::process::Command::new("git")
        .args(["show", &format!("{rev}:{ruta}")])
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

/// Revisión donde está publicada `nombre`/`version`: la última release o, si
/// no estaba ahí, HEAD (lo que ya está en `main` lo sirve el índice).
fn publicada_en(root: &Path, revs: &[String], nombre: &str, version: &str) -> Option<String> {
    let ruta = format!("rootfs/share/paquetes/{nombre}/{version}/paquete.txt");
    revs.iter()
        .find(|rev| git_fichero(root, rev, &ruta).is_some())
        .cloned()
}

fn semver(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.split('.').map(|p| p.parse::<u64>().ok());
    let r = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(r)
}

/// Deja cada versión publicada de `nombre` tal como se publicó: si el árbol la
/// tiene distinta (un build anterior la pisó), la devuelve desde git.
fn restaurar_publicadas(root: &Path, revs: &[String], nombre: &str) -> Result<(), String> {
    let dir = root.join("rootfs/share/paquetes").join(nombre);
    let Ok(entradas) = fs::read_dir(&dir) else {
        return Ok(());
    };
    for e in entradas.flatten() {
        let Some(version) = e.file_name().to_str().map(String::from) else {
            continue;
        };
        if semver(&version).is_none() {
            continue;
        }
        let Some(rev) = publicada_en(root, revs, nombre, &version) else {
            continue;
        };
        for f in ["paquete.txt", "paquete.pack"] {
            let ruta = format!("rootfs/share/paquetes/{nombre}/{version}/{f}");
            let Some(publicado) = git_fichero(root, &rev, &ruta) else {
                continue;
            };
            let local = root.join(&ruta);
            if fs::read(&local).ok().as_deref() != Some(&publicado[..]) {
                eprintln!("xtask: {nombre} {version} ya está publicada ({rev}): se restaura {f}");
                fs::write(&local, &publicado).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// Versión en la que debe quedar `nombre` con el contenido que produce
/// `hacer(version)` (manifiesto en texto), y si hay que escribirla.
///
/// - Ninguna versión, o la mínima `base` por encima de todas: `base`.
/// - La última no está publicada: se regenera en su sitio.
/// - La última está publicada con el mismo contenido: no se toca.
/// - La última está publicada con otro contenido: la siguiente patch.
fn version_destino(
    root: &Path,
    revs: &[String],
    nombre: &str,
    base: &str,
    hacer: &dyn Fn(&str) -> Result<String, String>,
) -> Result<(String, bool), String> {
    let dir = root.join("rootfs/share/paquetes").join(nombre);
    let ultima = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("paquete.txt").is_file())
        .filter_map(|e| e.file_name().to_str().and_then(|v| semver(v).map(|t| (t, v.to_string()))))
        .max();
    let Some((tu, u)) = ultima else {
        return Ok((base.to_string(), true));
    };
    let tb = semver(base).ok_or_else(|| format!("versión mínima inválida {base:?}"))?;
    if tb > tu {
        return Ok((base.to_string(), true));
    }
    if publicada_en(root, revs, nombre, &u).is_none() {
        return Ok((u, true));
    }
    let publicado = fs::read(dir.join(&u).join("paquete.txt")).map_err(|e| e.to_string())?;
    if hacer(&u)?.as_bytes() == &publicado[..] {
        return Ok((u, false));
    }
    let nueva = format!("{}.{}.{}", tu.0, tu.1, tu.2 + 1);
    eprintln!("xtask: {nombre} cambió y {u} ya está publicada: pasa a {nueva}");
    Ok((nueva, true))
}

/// Materializa `nombre` en `rootfs/share/paquetes/<nombre>/<versión>` con la
/// versión de [`version_destino`]. Devuelve la versión, el manifiesto en texto,
/// el pack y el manifiesto.
fn materializar_en_catalogo(
    root: &Path,
    nombre: &str,
    base: &str,
    hacer: impl Fn(&str) -> Result<(String, Vec<u8>, soso_pkg_core::Manifest), String>,
) -> Result<(String, String, Vec<u8>, soso_pkg_core::Manifest), String> {
    let revs: Vec<String> = ultima_release(root).into_iter().chain(["HEAD".to_string()]).collect();
    restaurar_publicadas(root, &revs, nombre)?;
    let (version, escribir) =
        version_destino(root, &revs, nombre, base, &|v| hacer(v).map(|(t, _, _)| t))?;
    let (man_text, pack, man) = hacer(&version)?;
    if escribir {
        let out = root.join("rootfs/share/paquetes").join(nombre).join(&version);
        escribir_paquete(&out, &man_text, &pack)?;
    }
    Ok((version, man_text, pack, man))
}

fn escribir_paquete(out: &Path, man_text: &str, pack: &[u8]) -> Result<(), String> {
    if out.exists() {
        fs::remove_dir_all(out).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    fs::write(out.join("paquete.txt"), man_text).map_err(|e| e.to_string())?;
    fs::write(out.join("paquete.pack"), pack).map_err(|e| e.to_string())
}

/// Paquete de aplicación (sin `destino=sistema`): queda en `/var/paquetes/salvi`
/// y sosh lo encuentra por `/var/paquetes/comandos`. No se instala en la imagen.
fn construir_salvi(root: &Path) -> Result<Vec<u8>, String> {
    let repo = root.join("../salvi");
    let script = repo.join("build.sh");
    if !script.is_file() {
        return Err(format!("no hay {script:?}; clona salvi junto a soso"));
    }
    let status = std::process::Command::new("sh")
        .arg(&script)
        .arg("soso")
        .current_dir(&repo)
        .env("SOSO", root)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("build.sh de salvi falló (¿cargo xtask sal-build --cross-only?)".into());
    }
    let elf = repo.join("salvi-soso");
    fs::read(&elf).map_err(|_| format!("no hay {elf:?}"))
}

fn empaquetar_salvi(
    version: &str,
    data: &[u8],
) -> Result<(String, Vec<u8>, soso_pkg_core::Manifest), String> {
    empaquetar::empaquetar(
        SALVI_NOMBRE,
        version,
        Some(ABI_VERSION),
        vec![("bin/salvi".into(), data.to_vec())],
    )
    .map_err(|e| format!("{e}"))
}

fn leer_bin_sistema(root: &Path, binario: &str) -> Result<Vec<u8>, String> {
    let elf = if binario == "soso-wm" {
        root.join("rootfs/bin/soso-wm")
    } else {
        root.join("target/user/x86_64-soso-user/release")
            .join(binario)
    };
    fs::read(&elf).map_err(|_| format!("no hay {elf:?}; ejecuta build_user"))
}

fn empaquetar_bin_sistema(
    nombre: &str,
    version: &str,
    data: &[u8],
) -> Result<(String, Vec<u8>, soso_pkg_core::Manifest), String> {
    let (_, pack, mut man) = empaquetar::empaquetar(
        nombre,
        version,
        Some(ABI_VERSION),
        vec![(format!("bin/{nombre}"), data.to_vec())],
    )
    .map_err(|e| format!("{e}"))?;
    man.destino_sistema = true;
    Ok((man.format(), pack, man))
}

/// Con `--out` explícito: la versión mínima, sin mirar el catálogo.
fn materializar_bin_sistema(root: &Path, nombre: &str, version: &str, out: &Path) -> Result<(), String> {
    let data = leer_bin_sistema(root, nombre)?;
    let (man_text, pack, _) = empaquetar_bin_sistema(nombre, version, &data)?;
    escribir_paquete(out, &man_text, &pack)
}

/// Índice HTTPS estático para servir este árbol con raw.githubusercontent.com.
pub fn generar_indice(root: &Path) -> Result<(), String> {
    generar_indice_en(&root.join("rootfs/share/paquetes"), |n| {
        n == "soso-ed"
            || n == "soso-pkg"
            || n == "soso-update"
            || n == "soso-wm"
            || n == SALVI_NOMBRE
    })
}

/// `indice.txt` de un árbol `<nombre>/<versión>/paquete.{txt,pack}` con los
/// paquetes que acepte `publicado`. Lo usa también el repositorio de Sal.
pub fn generar_indice_en(base: &Path, publicado: impl Fn(&str) -> bool) -> Result<(), String> {
    let mut filas = Vec::new();
    for pkg in fs::read_dir(base).map_err(|e| e.to_string())? {
        let pkg = pkg.map_err(|e| e.to_string())?.path();
        if !pkg.is_dir() {
            continue;
        }
        let publicado = pkg.file_name().and_then(|n| n.to_str()).is_some_and(&publicado);
        if !publicado {
            continue;
        }
        for ver in fs::read_dir(&pkg).map_err(|e| e.to_string())? {
            let dir = ver.map_err(|e| e.to_string())?.path();
            if !dir.is_dir() {
                continue;
            }
            let txt = match fs::read(dir.join("paquete.txt")) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let pack = match fs::read(dir.join("paquete.pack")) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let text = core::str::from_utf8(&txt).map_err(|e| e.to_string())?;
            let man = soso_pkg_core::Manifest::parse(text).map_err(|e| format!("{e}"))?;
            let version = man.version_raw;
            filas.push(format!(
                "p {} {} {} {} {} {}\n",
                man.nombre,
                version,
                soso_update_core::hex_sha256(&txt),
                txt.len(),
                soso_update_core::hex_sha256(&pack),
                pack.len(),
            ));
        }
    }
    filas.sort();
    let mut indice = String::from("SOSOPKG indice v1\n");
    for fila in filas {
        indice.push_str(&fila);
    }
    fs::write(base.join("indice.txt"), indice).map_err(|e| e.to_string())
}

/// Catálogo en `share/` + copia instalada bajo `rootfs/var/paquetes/` (imagen live).
pub fn materializar_soso_web_en_rootfs(root: &Path) -> Result<(), String> {
    let files = recoger_soso_web(root)?;
    let (_, man_text, pack, man) =
        materializar_en_catalogo(root, SOSO_WEB_NOMBRE, SOSO_WEB_VERSION, |v| {
            empaquetar::empaquetar(SOSO_WEB_NOMBRE, v, Some(ABI_VERSION), files.clone())
                .map_err(|e| format!("{e}"))
        })?;
    escribir_instalado(root, &man_text, &pack, &man)
}

fn materializar_soso_web(
    root: &Path,
    catalog_dir: &Path,
    instalar_en_rootfs: bool,
) -> Result<(), String> {
    let files = recoger_soso_web(root)?;
    let (man_text, pack, man) =
        empaquetar::empaquetar(SOSO_WEB_NOMBRE, SOSO_WEB_VERSION, Some(ABI_VERSION), files)
            .map_err(|e| format!("{e}"))?;
    fs::create_dir_all(catalog_dir).map_err(|e| e.to_string())?;
    fs::write(catalog_dir.join("paquete.txt"), &man_text).map_err(|e| e.to_string())?;
    fs::write(catalog_dir.join("paquete.pack"), &pack).map_err(|e| e.to_string())?;
    if instalar_en_rootfs {
        escribir_instalado(root, &man_text, &pack, &man)?;
    }
    Ok(())
}

fn recoger_soso_web(root: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let elf = root.join("target/user/x86_64-soso-user/release/soso-web");
    let font = root.join("rootfs/lib/fonts/DejaVuSans.ttf");
    let demo = root.join("rootfs/etc/web-prueba.html");
    let bin = fs::read(&elf).map_err(|_| format!("no hay {elf:?}; ejecuta build_user"))?;
    let font_bytes = fs::read(&font).map_err(|e| e.to_string())?;
    let demo_bytes = fs::read(&demo).map_err(|e| e.to_string())?;
    Ok(vec![
        ("bin/soso-web".into(), bin),
        ("share/fonts/DejaVuSans.ttf".into(), font_bytes),
        ("share/ejemplo/web-prueba.html".into(), demo_bytes),
    ])
}

fn escribir_instalado(
    root: &Path,
    man_text: &str,
    pack: &[u8],
    man: &soso_pkg_core::Manifest,
) -> Result<(), String> {
    let base = root.join("rootfs/var/paquetes").join(SOSO_WEB_NOMBRE);
    if base.exists() {
        fs::remove_dir_all(&base).ok();
    }
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    fs::write(base.join("paquete.txt"), man_text.as_bytes()).map_err(|e| e.to_string())?;
    for f in &man.files {
        let start = f.offset as usize;
        let end = start + f.size as usize;
        let dest = base.join(&f.path);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        fs::write(&dest, &pack[start..end]).map_err(|e| e.to_string())?;
    }
    actualizar_comandos_rootfs(root, man)
}

fn actualizar_comandos_rootfs(
    root: &Path,
    man_soso_web: &soso_pkg_core::Manifest,
) -> Result<(), String> {
    let bin_dir = root.join("rootfs/bin");
    let mut bin_sistema: Vec<String> = fs::read_dir(&bin_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    bin_sistema.sort();
    let refs: Vec<&str> = bin_sistema.iter().map(|s| s.as_str()).collect();
    let pkgs = vec![(SOSO_WEB_NOMBRE.into(), man_soso_web.clone())];
    let idx = reconstruir(&pkgs, &refs)?;
    let comandos = root.join("rootfs/var/paquetes/comandos");
    if let Some(p) = comandos.parent() {
        fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    fs::write(&comandos, idx.format().as_bytes()).map_err(|e| e.to_string())
}

fn run_opencode_sal(args: &[String]) {
    let root = crate::project_root();
    let out = args
        .iter()
        .skip(1)
        .find_map(|a| a.strip_prefix("--out=").map(PathBuf::from))
        .or_else(|| {
            let mut i = args.iter().skip(1);
            if i.next()? == "--out" {
                i.next().map(PathBuf::from)
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            root.join("rootfs/share/paquetes/opencode-sal")
                .join(OPENCODE_SAL_VERSION)
        });
    match materializar_opencode_sal(&root, &out, false) {
        Ok(()) => println!("xtask pkg-pack: {}", out.display()),
        Err(e) => {
            eprintln!("xtask pkg-pack: {e}");
            exit(1);
        }
    }
}

pub fn materializar_opencode_sal_en_rootfs(root: &Path) -> Result<(), String> {
    let cat = root
        .join("rootfs/share/paquetes/opencode-sal")
        .join(OPENCODE_SAL_VERSION);
    // No instalar en rootfs/var: la fase QEMU «opencode-sal» debe ejercitar
    // `soso-pkg instalar` desde paquetes-prueba sin choque en /bin/opencode.
    materializar_opencode_sal(root, &cat, false)
}

fn materializar_opencode_sal(
    root: &Path,
    catalog_dir: &Path,
    instalar_en_rootfs: bool,
) -> Result<(), String> {
    let repo = root.join("../opencode-sal");
    let oc_path = repo.join("out/bin/opencode");
    let imp_path = repo.join("out/bin/soso-improve-sal");
    if !oc_path.is_file() {
        let script = repo.join("build.sh");
        if script.is_file() {
            let status = std::process::Command::new("bash")
                .arg(&script)
                .current_dir(&repo)
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("build.sh de opencode-sal falló".into());
            }
        }
    }
    let oc = fs::read(&oc_path).map_err(|_| {
        format!(
            "no hay {oc_path:?}; ejecuta opencode-sal/build.sh o cargo xtask pkg-pack opencode-sal"
        )
    })?;
    let imp = fs::read(&imp_path).map_err(|e| e.to_string())?;
    let ver = format!("opencode-sal {OPENCODE_SAL_VERSION}\n");
    let files = vec![
        ("bin/opencode".into(), oc),
        ("bin/soso-improve-sal".into(), imp),
        ("share/opencode/VERSION".into(), ver.into_bytes()),
    ];
    let (man_text, pack, man) = empaquetar::empaquetar(
        OPENCODE_SAL_NOMBRE,
        OPENCODE_SAL_VERSION,
        Some(ABI_VERSION),
        files,
    )
    .map_err(|e| format!("{e}"))?;
    if catalog_dir.exists() {
        fs::remove_dir_all(catalog_dir).ok();
    }
    fs::create_dir_all(catalog_dir).map_err(|e| e.to_string())?;
    fs::write(catalog_dir.join("paquete.txt"), &man_text).map_err(|e| e.to_string())?;
    fs::write(catalog_dir.join("paquete.pack"), &pack).map_err(|e| e.to_string())?;
    for f in &man.files {
        let start = f.offset as usize;
        let end = start + f.size as usize;
        let dest = catalog_dir.join(&f.path);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        fs::write(&dest, &pack[start..end]).map_err(|e| e.to_string())?;
    }
    let prueba = root.join("rootfs/share/paquetes-prueba/opencode-sal");
    if prueba.exists() {
        fs::remove_dir_all(&prueba).ok();
    }
    fs::create_dir_all(&prueba).map_err(|e| e.to_string())?;
    fs::write(prueba.join("paquete.txt"), &man_text).map_err(|e| e.to_string())?;
    // En QEMU, `soso-pkg` ensambla desde ficheros sueltos (como v1/v2 del demo).
    for f in &man.files {
        let start = f.offset as usize;
        let end = start + f.size as usize;
        let dest = prueba.join(&f.path);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        fs::write(&dest, &pack[start..end]).map_err(|e| e.to_string())?;
    }
    if instalar_en_rootfs {
        escribir_instalado_opencode(root, &man_text, &pack, &man)?;
    }
    Ok(())
}

fn escribir_instalado_opencode(
    root: &Path,
    man_text: &str,
    pack: &[u8],
    man: &soso_pkg_core::Manifest,
) -> Result<(), String> {
    let base = root.join("rootfs/var/paquetes").join(OPENCODE_SAL_NOMBRE);
    if base.exists() {
        fs::remove_dir_all(&base).ok();
    }
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    fs::write(base.join("paquete.txt"), man_text.as_bytes()).map_err(|e| e.to_string())?;
    for f in &man.files {
        let start = f.offset as usize;
        let end = start + f.size as usize;
        let dest = base.join(&f.path);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        fs::write(&dest, &pack[start..end]).map_err(|e| e.to_string())?;
    }
    actualizar_comandos_opencode(root, man)
}

fn actualizar_comandos_opencode(
    root: &Path,
    man_opencode: &soso_pkg_core::Manifest,
) -> Result<(), String> {
    let bin_dir = root.join("rootfs/bin");
    let mut bin_sistema: Vec<String> = fs::read_dir(&bin_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    bin_sistema.sort();
    let refs: Vec<&str> = bin_sistema.iter().map(|s| s.as_str()).collect();
    let pkgs = vec![(OPENCODE_SAL_NOMBRE.into(), man_opencode.clone())];
    let idx = reconstruir(&pkgs, &refs)?;
    let comandos = root.join("rootfs/var/paquetes/comandos");
    if let Some(p) = comandos.parent() {
        fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    fs::write(&comandos, idx.format().as_bytes()).map_err(|e| e.to_string())
}
