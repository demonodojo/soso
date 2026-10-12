//! mkfs-sosomfs: empaqueta uno o más directorios sosomodel en una imagen sosomfs.

use block_dev::FileBlockDevice;
use sosomfs::build_from_dirs;
use sosomfs::parse_size;
use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let raw: Vec<String> = env::args().skip(1).collect();
    let mut model_dirs: Vec<PathBuf> = Vec::new();
    let mut image: Option<PathBuf> = None;
    let mut size_blocks = 8u64 * 1024 * 1024 * 1024 / 4096; // 8 GiB
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--size" => {
                i += 1;
                size_blocks = parse_size(raw.get(i).expect("falta valor de --size")).unwrap_or_else(
                    |e| {
                        eprintln!("{e}");
                        std::process::exit(2);
                    },
                );
            }
            "--extra-model" => {
                i += 1;
                model_dirs.push(PathBuf::from(
                    raw.get(i).expect("falta dir tras --extra-model"),
                ));
            }
            s if s.starts_with('-') => {
                eprintln!("mkfs-sosomfs: opción desconocida {s}");
                std::process::exit(2);
            }
            other => {
                positional.push(other.into());
            }
        }
        i += 1;
    }

    if !positional.is_empty() {
        if positional.len() == 1 {
            model_dirs.push(PathBuf::from(&positional[0]));
        } else {
            image = Some(PathBuf::from(positional.last().unwrap()));
            for p in &positional[..positional.len() - 1] {
                model_dirs.push(PathBuf::from(p));
            }
        }
    }

    if model_dirs.is_empty() || image.is_none() {
        eprintln!(
            "uso: mkfs-sosomfs <dir_sosomodel> [dir_extra...] <imagen> [--size 8G] [--extra-model dir]"
        );
        std::process::exit(2);
    }
    let image = image.unwrap();
    let refs: Vec<&Path> = model_dirs.iter().map(|p| p.as_path()).collect();

    let mut dev = FileBlockDevice::create(&image, size_blocks).expect("crear imagen");
    let report = build_from_dirs(&mut dev, &refs, 1).expect("construir sosomfs");
    println!(
        "mkfs-sosomfs: {} bloques, {} modelos, {} shards → {}",
        report.total_blocks,
        report.models,
        report.shards,
        image.display()
    );
}
