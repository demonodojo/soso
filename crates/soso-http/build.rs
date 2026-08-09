//! Enlaza ensamblador pregenerado de `ring` cuando el target no tiene OS (soso-user).

use std::path::PathBuf;
use std::{env, fs};

fn main() {
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if os != "none" || arch != "x86_64" {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let ring_pregen = find_ring_pregenerated(&manifest_dir);
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
}

fn find_ring_pregenerated(_manifest_dir: &std::path::Path) -> Option<PathBuf> {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .ok()?;
    let src = PathBuf::from(home).join(".cargo/registry/src/index.crates.io-1949cf8c6b5b557f");
    if !src.is_dir() {
        return None;
    }
    for crate_dir in fs::read_dir(&src).ok()?.flatten() {
        let name = crate_dir.file_name().to_string_lossy().into_owned();
        if name.starts_with("ring-0.17.") {
            let pregen = crate_dir.path().join("pregenerated");
            if pregen.is_dir() {
                return Some(pregen);
            }
        }
    }
    None
}
