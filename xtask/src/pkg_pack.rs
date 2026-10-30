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

pub fn run(args: &[String]) {
    let Some(pkg) = args.first() else {
        eprintln!("uso: cargo xtask pkg-pack <soso-web|opencode-sal> [--out DIR]");
        exit(2);
    };
    if pkg == OPENCODE_SAL_NOMBRE {
        run_opencode_sal(args);
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
    materializar_opencode_sal(root, &cat, true)
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
    fs::create_dir_all(catalog_dir).map_err(|e| e.to_string())?;
    fs::write(catalog_dir.join("paquete.txt"), &man_text).map_err(|e| e.to_string())?;
    fs::write(catalog_dir.join("paquete.pack"), &pack).map_err(|e| e.to_string())?;
    let prueba = root.join("rootfs/share/paquetes-prueba/opencode-sal");
    fs::create_dir_all(&prueba).map_err(|e| e.to_string())?;
    fs::write(prueba.join("paquete.txt"), &man_text).map_err(|e| e.to_string())?;
    fs::write(prueba.join("paquete.pack"), &pack).map_err(|e| e.to_string())?;
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
