//! `cargo xtask release [--publish]` — empaqueta y opcionalmente publica en GitHub Releases.

use std::process::{Command, exit};

use soso_update_core::hash::hex_sha256;
use soso_update_core::manifest::Manifest;
use soso_update_core::pack::pack_rootfs;
use soso_update_core::semver::parse as parse_semver;

use crate::drivers::{self, DriverProfile};
use crate::version;

const GITHUB_BASE: &str = "https://github.com/demonodojo/soso/releases/latest/download";

pub fn run(args: &[String]) {
    let publish = args.iter().any(|a| a == "--publish");
    let root = crate::project_root();
    let ver = version::read_version(&root);
    if parse_semver(&ver).is_none() {
        eprintln!("release: VERSION inválida: {ver}");
        exit(1);
    }

    let profile = live_profile();
    crate::build_user();
    let _ = crate::build_image_with_profile(&profile, true);
    crate::mkfs_rootfs_with_profile(true, &profile, crate::RootfsImgMode::PackOnly);

    let out_dir = root.join(format!("target/release-soso/v{ver}"));
    std::fs::create_dir_all(&out_dir).expect("release dir");

    let kernel_src = root.join("target/kernel/x86_64-soso/debug/kernel");
    let kernel_dst = out_dir.join("kernel-x86_64");
    std::fs::copy(&kernel_src, &kernel_dst).expect("copiar kernel");
    strip_kernel(&kernel_dst);

    let (pack_blob, files) = pack_rootfs(&root.join("rootfs")).expect("empaquetar rootfs");
    let pack_path = out_dir.join("rootfs.pack");
    std::fs::write(&pack_path, &pack_blob).expect("rootfs.pack");

    let kernel_bytes = std::fs::read(&kernel_dst).expect("leer kernel");
    let kernel_hash = hex_sha256(&kernel_bytes);
    let pack_hash = hex_sha256(&pack_blob);

    let manifest = Manifest {
        version: parse_semver(&ver).unwrap(),
        version_raw: ver.clone(),
        build: version::git_build(&root),
        fecha: version::build_date(),
        kernel_hash: kernel_hash.clone(),
        kernel_size: kernel_bytes.len() as u64,
        pack_hash: pack_hash.clone(),
        pack_size: pack_blob.len() as u64,
        files,
    };
    let manifest_text = manifest.format();
    let manifest_path = out_dir.join("manifest.txt");
    std::fs::write(&manifest_path, &manifest_text).expect("manifest.txt");

    let notes = out_dir.join("NOTES.md");
    std::fs::write(
        &notes,
        format!(
            "# soso {ver}\n\n\
             Descarga desde soso instalado:\n\n\
             ```\n\
             soso-update comprobar\n\
             soso-update aplicar\n\
             ```\n\n\
             URL base: {GITHUB_BASE}\n"
        ),
    )
    .expect("NOTES.md");

    println!("release: artefactos en {}", out_dir.display());
    println!("  kernel-x86_64 ({} B)", kernel_bytes.len());
    println!("  rootfs.pack ({} B)", pack_blob.len());
    println!("  manifest.txt ({} ficheros)", manifest.files.len());

    if !publish {
        println!("release: usa --publish para subir con gh release create");
        return;
    }

    let tag = format!("v{ver}");
    let st = Command::new("gh")
        .args(["auth", "status"])
        .status()
        .unwrap_or_else(|e| {
            eprintln!("release: gh no disponible: {e}");
            exit(1);
        });
    if !st.success() {
        eprintln!("release: gh auth status falló — ejecuta `gh auth login`");
        exit(1);
    }

    let status = Command::new("gh")
        .args([
            "release",
            "create",
            &tag,
            "--title",
            &format!("soso {ver}"),
            "--notes-file",
            notes.to_str().unwrap(),
        ])
        .arg(&manifest_path)
        .arg(&pack_path)
        .arg(&kernel_dst)
        .status()
        .expect("gh release create");
    if !status.success() {
        eprintln!("release: gh release create falló");
        exit(status.code().unwrap_or(1));
    }
    println!("release: publicado {tag}");
}

/// Quita la información de depuración del kernel que se publica.
///
/// El perfil de compilación es `debug`, así que dos tercios largos del ELF son
/// secciones `.debug_*` que nadie lee en tiempo de ejecución (el kernel no
/// resuelve símbolos para sus panics). Descargarlas en cada actualización sería
/// pagar ~25 MB por nada; el binario con símbolos sigue en `target/` para gdb.
pub(crate) fn strip_kernel(path: &std::path::Path) {
    let antes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    for tool in ["strip", "llvm-strip"] {
        let ok = Command::new(tool)
            .arg("-s")
            .arg(path)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            let despues = std::fs::metadata(path).map(|m| m.len()).unwrap_or(antes);
            println!(
                "release: kernel sin símbolos de depuración ({antes} B -> {despues} B, {tool})"
            );
            return;
        }
    }
    eprintln!("release: aviso — no encontré strip/llvm-strip, publico el kernel entero ({antes} B)");
}

fn live_profile() -> DriverProfile {
    if std::env::var("SOSO_DRIVERS")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        drivers::profile_from_env_or_args()
    } else {
        drivers::preset_live_usb()
    }
}
