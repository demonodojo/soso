//! Enlaza ensamblador pregenerado de `ring` cuando el target no tiene OS (soso-user).

use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if os != "none" || arch != "x86_64" {
        return;
    }

    let ring_pregen = find_ring_pregenerated();
    let Some(ring_pregen) = ring_pregen else {
        println!("cargo:warning=soso-http: no se encontró ring pregenerated; TLS puede fallar al enlazar");
        return;
    };

    let mut asm_files = Vec::new();
    for entry in fs::read_dir(&ring_pregen).unwrap().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with("-elf.S") {
            asm_files.push(entry.path());
        }
    }
    asm_files.sort();

    let mut cc_build = cc::Build::new();
    let ring_root = ring_pregen.parent().unwrap();
    let target = env::var("TARGET").unwrap();
    cc_build
        .files(&asm_files)
        .flag("-std=c11")
        .flag("-fno-stack-protector")
        .include(ring_root.join("include"))
        .include(&ring_pregen)
        .target(&target);
    cc_build.compile("soso_ring_asm");

    println!("cargo:rerun-if-changed={}", ring_pregen.display());
    println!("cargo:rerun-if-changed=Cargo.lock");
}

fn ring_version_from_lock() -> Option<String> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    let lock = manifest_dir.join("../../Cargo.lock");
    let text = fs::read_to_string(lock).ok()?;
    let mut in_ring = false;
    for line in text.lines() {
        if line.trim() == "name = \"ring\"" {
            in_ring = true;
            continue;
        }
        if in_ring {
            if let Some(ver) = line.strip_prefix("version = \"") {
                return Some(ver.trim_end_matches('"').to_string());
            }
            if line.starts_with("name = ") {
                break;
            }
        }
    }
    None
}

fn find_ring_pregenerated() -> Option<PathBuf> {
    let version = ring_version_from_lock().unwrap_or_else(|| "0.17.14".into());
    let dir_name = format!("ring-{version}");
    let cargo_home = env::var("CARGO_HOME")
        .or_else(|_| env::var("HOME").map(|h| format!("{h}/.cargo")))
        .ok()?;
    let registry = PathBuf::from(cargo_home).join("registry/src");
    if !registry.is_dir() {
        return None;
    }
    for index_dir in fs::read_dir(&registry).ok()?.flatten() {
        if !index_dir.file_type().ok()?.is_dir() {
            continue;
        }
        let candidate = index_dir.path().join(&dir_name).join("pregenerated");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    find_ring_fallback(&registry)
}

fn find_ring_fallback(registry: &Path) -> Option<PathBuf> {
    for index_dir in fs::read_dir(registry).ok()?.flatten() {
        if !index_dir.file_type().ok()?.is_dir() {
            continue;
        }
        for crate_dir in fs::read_dir(index_dir.path()).ok()?.flatten() {
            let name = crate_dir.file_name().to_string_lossy().into_owned();
            if name.starts_with("ring-0.17.") {
                let pregen = crate_dir.path().join("pregenerated");
                if pregen.is_dir() {
                    return Some(pregen);
                }
            }
        }
    }
    None
}
