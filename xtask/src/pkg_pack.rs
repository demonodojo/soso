//! Empaquetado host de aplicaciones (`cargo xtask pkg-pack`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

use soso_abi::ABI_VERSION;
use soso_pkg_core::comandos::reconstruir;
use soso_pkg_core::empaquetar;

pub const SOSO_WEB_VERSION: &str = "1.0.0";
const SOSO_WEB_NOMBRE: &str = "soso-web";
pub const OPENCODE_SAL_VERSION: &str = "1.0.0";
const OPENCODE_SAL_NOMBRE: &str = "opencode-sal";
pub const SOSO_ED_VERSION: &str = "0.1.1";
pub const SOSO_PKG_VERSION: &str = "0.1.0";
pub const SOSO_WM_VERSION: &str = "0.1.0";
/// Debe coincidir con la versión del compilador en el checkout fijado por sal-source.lock.
pub const SAL_VERSION: &str = "0.5.0";

pub fn run(args: &[String]) {
    let Some(pkg) = args.first() else {
        eprintln!("uso: cargo xtask pkg-pack <soso-web|opencode-sal|soso-ed|soso-pkg|soso-wm|sal> [--out DIR]");
        exit(2);
    };
    if pkg == OPENCODE_SAL_NOMBRE {
        run_opencode_sal(args);
        return;
    }
    if pkg == "sal" {
        let root = crate::project_root();
        let out = args.iter().skip(1).find_map(|a| a.strip_prefix("--out=").map(PathBuf::from))
            .unwrap_or_else(|| root.join("rootfs/share/paquetes/sal").join(SAL_VERSION));
        match materializar_sal(&root, &out) {
            Ok(()) => {
                if let Err(e) = generar_indice(&root) {
                    eprintln!("xtask pkg-pack: no pude actualizar indice.txt: {e}");
                    exit(1);
                }
                println!("xtask pkg-pack: {}", out.display());
            }
            Err(e) => { eprintln!("xtask pkg-pack: {e}"); exit(1); }
        }
        return;
    }
    if pkg == "soso-ed" || pkg == "soso-pkg" || pkg == "soso-wm" {
        let root = crate::project_root();
        let version = match pkg.as_str() {
            "soso-ed" => SOSO_ED_VERSION,
            "soso-pkg" => SOSO_PKG_VERSION,
            _ => SOSO_WM_VERSION,
        };
        let out = args.iter().skip(1).find_map(|a| a.strip_prefix("--out=").map(PathBuf::from))
            .unwrap_or_else(|| root.join("rootfs/share/paquetes").join(pkg).join(version));
        match materializar_bin_sistema(&root, pkg, version, pkg, &out) {
            Ok(()) => {
                if let Err(e) = generar_indice(&root) {
                    eprintln!("xtask pkg-pack: no pude actualizar indice.txt: {e}");
                    exit(1);
                }
                println!("xtask pkg-pack: {}", out.display());
            }
            Err(e) => { eprintln!("xtask pkg-pack: {e}"); exit(1); }
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
        })
        .unwrap_or_else(|| {
            root.join("rootfs/share/paquetes/soso-web").join(SOSO_WEB_VERSION)
        });
    match materializar_soso_web(&root, &out, false) {
        Ok(()) => println!("xtask pkg-pack: {}", out.display()),
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
        ("soso-wm", SOSO_WM_VERSION),
    ] {
        let dir = root.join("rootfs/share/paquetes").join(nombre).join(version);
        materializar_bin_sistema(root, nombre, version, nombre, &dir)?;
    }
    // El compilador nativo solo existe tras `sal-build` sin --cross-only.
    // Las imágenes que no lo preparan conservan el paquete remoto existente.
    if root.join("rootfs/bin/sal").is_file() {
        materializar_sal(root, &root.join("rootfs/share/paquetes/sal").join(SAL_VERSION))?;
    }
    generar_indice(root)
}

fn materializar_sal(root: &Path, out: &Path) -> Result<(), String> {
    let elf = root.join("rootfs/bin/sal");
    let data = fs::read(&elf).map_err(|_| format!("no hay {elf:?}; ejecuta cargo xtask sal-build"))?;
    let (_, pack, mut man) = empaquetar::empaquetar(
        "sal",
        SAL_VERSION,
        Some(ABI_VERSION),
        vec![("bin/sal".into(), data)],
    ).map_err(|e| format!("{e}"))?;
    man.destino_sistema = true;
    let man_text = man.format();
    if out.exists() { fs::remove_dir_all(out).map_err(|e| e.to_string())?; }
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    fs::write(out.join("paquete.txt"), &man_text).map_err(|e| e.to_string())?;
    fs::write(out.join("paquete.pack"), &pack).map_err(|e| e.to_string())?;
    Ok(())
}

fn materializar_bin_sistema(
    root: &Path,
    nombre: &str,
    version: &str,
    binario: &str,
    out: &Path,
) -> Result<(), String> {
    let elf = if binario == "soso-wm" {
        root.join("rootfs/bin/soso-wm")
    } else {
        root.join("target/user/x86_64-soso-user/release").join(binario)
    };
    let data = fs::read(&elf).map_err(|_| format!("no hay {elf:?}; ejecuta build_user"))?;
    let (_, pack, mut man) = empaquetar::empaquetar(
        nombre,
        version,
        Some(ABI_VERSION),
        vec![(format!("bin/{binario}"), data.clone())],
    ).map_err(|e| format!("{e}"))?;
    man.destino_sistema = true;
    let man_text = man.format();
    if out.exists() { fs::remove_dir_all(out).map_err(|e| e.to_string())?; }
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    fs::write(out.join("paquete.txt"), &man_text).map_err(|e| e.to_string())?;
    fs::write(out.join("paquete.pack"), &pack).map_err(|e| e.to_string())?;
    Ok(())
}

/// Índice HTTPS estático para servir este árbol con raw.githubusercontent.com.
pub fn generar_indice(root: &Path) -> Result<(), String> {
    let base = root.join("rootfs/share/paquetes");
    let mut filas = Vec::new();
    for pkg in fs::read_dir(&base).map_err(|e| e.to_string())? {
        let pkg = pkg.map_err(|e| e.to_string())?.path();
        if !pkg.is_dir() { continue; }
        let publicado = pkg.file_name().and_then(|n| n.to_str())
            .is_some_and(|n| n == "soso-ed" || n == "soso-pkg" || n == "soso-wm" || n == "sal");
        if !publicado { continue; }
        for ver in fs::read_dir(&pkg).map_err(|e| e.to_string())? {
            let dir = ver.map_err(|e| e.to_string())?.path();
            if !dir.is_dir() { continue; }
            let txt = match fs::read(dir.join("paquete.txt")) { Ok(b) => b, Err(_) => continue };
            let pack = match fs::read(dir.join("paquete.pack")) { Ok(b) => b, Err(_) => continue };
            let text = core::str::from_utf8(&txt).map_err(|e| e.to_string())?;
            let man = soso_pkg_core::Manifest::parse(text).map_err(|e| format!("{e}"))?;
            let version = man.version_raw;
            filas.push(format!(
                "p {} {} {} {} {} {}\n",
                man.nombre,
                version,
                soso_update_core::hex_sha256(&txt), txt.len(),
                soso_update_core::hex_sha256(&pack), pack.len(),
            ));
        }
    }
    filas.sort();
    let mut indice = String::from("SOSOPKG indice v1\n");
    for fila in filas { indice.push_str(&fila); }
    fs::write(base.join("indice.txt"), indice).map_err(|e| e.to_string())
}

/// Catálogo en `share/` + copia instalada bajo `rootfs/var/paquetes/` (imagen live).
pub fn materializar_soso_web_en_rootfs(root: &Path) -> Result<(), String> {
    let cat = root
        .join("rootfs/share/paquetes/soso-web")
        .join(SOSO_WEB_VERSION);
    materializar_soso_web(root, &cat, true)
}

fn materializar_soso_web(root: &Path, catalog_dir: &Path, instalar_en_rootfs: bool) -> Result<(), String> {
    let files = recoger_soso_web(root)?;
    let (man_text, pack, man) = empaquetar::empaquetar(
        SOSO_WEB_NOMBRE,
        SOSO_WEB_VERSION,
        Some(ABI_VERSION),
        files,
    )
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

fn escribir_instalado(root: &Path, man_text: &str, pack: &[u8], man: &soso_pkg_core::Manifest) -> Result<(), String> {
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

fn actualizar_comandos_rootfs(root: &Path, man_soso_web: &soso_pkg_core::Manifest) -> Result<(), String> {
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
            root.join("rootfs/share/paquetes/opencode-sal").join(OPENCODE_SAL_VERSION)
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
