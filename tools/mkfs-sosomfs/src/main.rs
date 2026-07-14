//! mkfs-sosomfs: empaqueta un directorio sosomodel en una imagen sosomfs.

use block_dev::FileBlockDevice;
use sosomfs::build_from_dir;
use sosomfs::parse_size;
use std::env;
use std::path::PathBuf;

fn main() {
    let mut args = env::args().skip(1);
    let model_dir = PathBuf::from(args.next().unwrap_or_else(|| {
        eprintln!("uso: mkfs-sosomfs <dir_sosomodel> <imagen> [--size 8G]");
        std::process::exit(2);
    }));
    let image = PathBuf::from(args.next().expect("falta ruta de imagen"));
    let mut size_blocks = 8u64 * 1024 * 1024 * 1024 / 4096; // 8 GiB
    let mut rest: Vec<String> = args.collect();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--size" && i + 1 < rest.len() {
            size_blocks = parse_size(&rest[i + 1]).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(2);
            });
            i += 2;
        } else {
            i += 1;
        }
    }

    let mut dev = FileBlockDevice::create(&image, size_blocks).expect("crear imagen");
    let report = build_from_dir(&mut dev, &model_dir, 1).expect("construir sosomfs");
    println!(
        "mkfs-sosomfs: {} bloques, {} modelos, {} shards → {}",
        report.total_blocks,
        report.models,
        report.shards,
        image.display()
    );
}
