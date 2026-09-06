//! `cargo xtask flash-usb-live <usb> [--yes] [--skip-models] [--only …]`
//!
//! Graba `soso-live.img` en un pendrive y estira p3 (sosomfs). p4 SOSOINSTALL
//! ya va en la imagen (FAT entre rootfs y modelos), para que Linux la monte.
//!
//! Con `--skip-models` o `--only` actualiza solo ESP y/o rootfs sin tocar p3.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

use crate::install_disk;
use crate::package_live;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlashPart {
    Kernel,
    Rootfs,
}

pub fn run(args: &[String]) {
    let parsed = parse_args(args);
    let Some(usb) = parsed.device else {
        usage();
    };

    install_disk::validate_device(&usb, parsed.yes, None);

    let disk_bytes = blockdev_bytes(&usb).unwrap_or_else(|| {
        eprintln!("flash-usb-live: no pude leer el tamaño de {}", usb.display());
        exit(1);
    });
    println!(
        "flash-usb-live: pendrive {} ({})",
        usb.display(),
        crate::live_models::format_bytes(disk_bytes)
    );

    if parsed.incremental {
        run_incremental(&usb, &parsed.parts);
    } else {
        run_full(&usb, disk_bytes);
    }
}

struct ParsedArgs {
    device: Option<PathBuf>,
    yes: bool,
    incremental: bool,
    parts: Vec<FlashPart>,
}

fn parse_args(args: &[String]) -> ParsedArgs {
    let mut device = None;
    let mut yes = false;
    let mut skip_models = false;
    let mut only: Option<Vec<FlashPart>> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--yes" | "-y" => yes = true,
            "--skip-models" => skip_models = true,
            "--only" => {
                i += 1;
                let spec = args.get(i).unwrap_or_else(|| {
                    eprintln!("flash-usb-live: --only requiere kernel,rootfs o ambos");
                    usage();
                });
                only = Some(parse_only_spec(spec));
            }
            s if s.starts_with("--only=") => {
                only = Some(parse_only_spec(s.trim_start_matches("--only=")));
            }
            s if s.starts_with('-') => {
                eprintln!("flash-usb-live: opción desconocida: {s}");
                usage();
            }
            s => {
                if device.is_some() {
                    eprintln!("flash-usb-live: demasiados argumentos");
                    usage();
                }
                device = Some(PathBuf::from(s));
            }
        }
        i += 1;
    }

    let incremental = skip_models || only.is_some();

    let parts = if let Some(p) = only {
        p
    } else if skip_models {
        vec![FlashPart::Kernel, FlashPart::Rootfs]
    } else {
        Vec::new()
    };

    if incremental && parts.is_empty() {
        eprintln!("flash-usb-live: --only vacío");
        usage();
    }

    ParsedArgs {
        device,
        yes,
        incremental,
        parts,
    }
}

fn parse_only_spec(spec: &str) -> Vec<FlashPart> {
    let mut parts = Vec::new();
    for token in spec.split(',') {
        match token.trim().to_ascii_lowercase().as_str() {
            "kernel" | "esp" => {
                if !parts.contains(&FlashPart::Kernel) {
                    parts.push(FlashPart::Kernel);
                }
            }
            "rootfs" | "data" => {
                if !parts.contains(&FlashPart::Rootfs) {
                    parts.push(FlashPart::Rootfs);
                }
            }
            "" => {}
            other => {
                eprintln!("flash-usb-live: componente desconocido en --only: {other}");
                usage();
            }
        }
    }
    if parts.is_empty() {
        eprintln!("flash-usb-live: --only requiere kernel, rootfs o ambos");
        usage();
    }
    parts
}

fn run_full(usb: &Path, disk_bytes: u64) {
    let root = super::project_root();

    super::build_user();
    package_live::run_with_capacity(Some(disk_bytes));

    let live = package_live::live_image_path();
    let out_dir = package_live::out_dir();

    println!("flash-usb-live: escribiendo {} → {}", live.display(), usb.display());
    install_disk::run_cmd(
        Command::new("dd")
            .arg(format!("if={}", live.display()))
            .arg(format!("of={}", usb.display()))
            .args(["bs=4M", "status=progress", "conv=fsync"]),
        "dd",
    );
    install_disk::run_cmd(&mut Command::new("sync"), "sync");

    install_disk::expand_models_partition(usb, &live);
    install_disk::repair_gpt_backup(usb);

    print_flash_summary(usb, &out_dir);
    let _ = root;
}

fn run_incremental(usb: &Path, parts: &[FlashPart]) {
    if let Err(e) = package_live::validate_live_usb(usb) {
        eprintln!("flash-usb-live: {e}");
        exit(1);
    }

    let root = super::project_root();
    let profile = package_live::live_driver_profile();
    let want_kernel = parts.contains(&FlashPart::Kernel);
    let want_rootfs = parts.contains(&FlashPart::Rootfs);

    let wificonf = if want_kernel {
        package_live::read_esp_wificonf(usb)
    } else {
        None
    };

    if want_kernel {
        println!("flash-usb-live: compilando kernel (ESP)…");
        let _ = super::build_image_with_profile(&profile, true);
    }
    if want_rootfs {
        println!("flash-usb-live: empaquetando rootfs…");
        super::build_user();
        let _ = super::mkfs_rootfs_with_profile(true, &profile);
    }

    if want_kernel {
        let uefi = root.join("target/soso-uefi.img");
        if !uefi.exists() {
            eprintln!("flash-usb-live: falta {}", uefi.display());
            exit(1);
        }
        println!(
            "flash-usb-live: actualizando ESP (p1) desde {}…",
            uefi.display()
        );
        if let Err(e) = package_live::dd_partition(&uefi, 1, usb, 1) {
            eprintln!("flash-usb-live: {e}");
            exit(1);
        }
        package_live::create_esp_slots(usb);
        if let Some(ref saved) = wificonf {
            if let Err(e) = package_live::write_esp_wificonf(usb, saved) {
                eprintln!("flash-usb-live: aviso: no pude restaurar SOSOWIFI.TXT: {e}");
            } else {
                println!("flash-usb-live: SOSOWIFI.TXT restaurado");
            }
        }
    }

    if want_rootfs {
        let data = root.join("target/soso-data.img");
        if !data.exists() {
            eprintln!("flash-usb-live: falta {}", data.display());
            exit(1);
        }
        println!(
            "flash-usb-live: actualizando rootfs (p2) desde {}…",
            data.display()
        );
        if let Err(e) = package_live::dd_to_partition(&data, usb, 2) {
            eprintln!("flash-usb-live: {e}");
            exit(1);
        }
    }

    install_disk::run_cmd(&mut Command::new("sync"), "sync");

    let labels: Vec<_> = parts
        .iter()
        .map(|p| match p {
            FlashPart::Kernel => "ESP",
            FlashPart::Rootfs => "rootfs",
        })
        .collect();
    println!(
        "\n✅ USB live actualizado ({}) en {} — modelos (p3) intactos",
        labels.join(" + "),
        usb.display()
    );
}

fn usage() -> ! {
    eprintln!(
        "uso: cargo xtask flash-usb-live <usb> [--yes] [--skip-models] [--only PARTS]\n\
         \n\
         Ejemplo (flash completo):\n\
           lsblk\n\
           sudo cargo xtask flash-usb-live /dev/sde --yes\n\
         \n\
         Actualización incremental (sin reescribir modelos):\n\
           sudo env \"PATH=$PATH\" \"HOME=$HOME\" cargo xtask flash-usb-live /dev/sde --yes --skip-models\n\
           sudo env \"PATH=$PATH\" \"HOME=$HOME\" cargo xtask flash-usb-live /dev/sde --yes --only kernel\n\
           sudo env \"PATH=$PATH\" \"HOME=$HOME\" cargo xtask flash-usb-live /dev/sde --yes --only rootfs\n\
         \n\
         Graba soso-live.img (modelo según tamaño del stick), estira p3 (modelos).\n\
         p4 SOSOINSTALL (install-soso.sh) va en la imagen, tras el rootfs.\n\
         \n\
         Sin descargar modelos (el mayor ya en target/*-model/ que quepa):\n\
           SOSO_LIVE_OFFLINE=1 sudo cargo xtask flash-usb-live /dev/sde --yes"
    );
    exit(2);
}

fn blockdev_sectors(dev: &Path) -> Option<u64> {
    let out = Command::new("blockdev")
        .args(["--getsz"])
        .arg(dev)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .ok()
}

fn blockdev_bytes(dev: &Path) -> Option<u64> {
    blockdev_sectors(dev).map(|s| s.saturating_mul(512))
}

fn print_flash_summary(usb: &Path, out_dir: &Path) {
    println!("\n✅ USB live listo en {}", usb.display());
    println!(
        "\n1) Probar soso: UEFI → arrancar desde USB (F12).\n\
         2) Instalar en un NVMe dedicado (desde el propio soso live, sin Linux):\n\
            soso-install\n\
            soso-install nvme1 --yes\n\
            reinicia con el USB puesto → quita el USB → «soso» en el menú UEFI\n\
         \n\
         Plan B (desde Linux, USB conectado):\n\
            lsblk\n\
            sudo {}/install-soso.sh /dev/nvme1n1 --yes\n\
         \n\
         Artefactos también en {}",
        out_dir.display(),
        out_dir.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Seek, Write};
    use std::process::Command;

    fn require_sgdisk() -> bool {
        Command::new("sgdisk")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn create_test_live_img(path: &Path, p1_mb: u64, p2_mb: u64, p3_mb: u64) {
        let total_mb = 1 + p1_mb + p2_mb + p3_mb + 8;
        {
            let f = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
                .unwrap();
            f.set_len(total_mb * 1024 * 1024).unwrap();
        }
        package_live::run_cmd(
            Command::new("sgdisk").args(["-o", "-a", "1"]).arg(path),
            "sgdisk -o",
        );
        package_live::run_cmd(
            Command::new("sgdisk")
                .arg("-n")
                .arg(format!("1:2048:+{p1_mb}M"))
                .arg("-t")
                .arg("1:EF00")
                .arg("-n")
                .arg(format!("2:0:+{p2_mb}M"))
                .arg("-t")
                .arg("2:8300")
                .arg("-n")
                .arg(format!("3:0:+{p3_mb}M"))
                .arg("-t")
                .arg("3:8300")
                .arg(path),
            "sgdisk parts",
        );
    }

    fn write_pattern_to_p3(img: &Path, byte: u8) {
        let (first, last) = package_live::partition_range(img, 3).unwrap();
        let count = last - first + 1;
        let mut data = vec![byte; (count * 512) as usize];
        data[0] = byte.wrapping_add(1);
        let mut f = fs::OpenOptions::new().write(true).open(img).unwrap();
        f.seek(std::io::SeekFrom::Start(first * 512)).unwrap();
        f.write_all(&data).unwrap();
    }

    fn read_p3_first_byte(img: &Path) -> u8 {
        let (first, _) = package_live::partition_range(img, 3).unwrap();
        let mut b = [0u8; 1];
        let mut f = fs::OpenOptions::new().read(true).open(img).unwrap();
        f.seek(std::io::SeekFrom::Start(first * 512)).unwrap();
        f.read_exact(&mut b).unwrap();
        b[0]
    }

    fn minimal_fat_img(path: &Path, size_mb: u64) {
        let total_mb = size_mb + 16;
        {
            let f = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
                .unwrap();
            f.set_len(total_mb * 1024 * 1024).unwrap();
        }
        package_live::run_cmd(
            Command::new("sgdisk").args(["-o", "-a", "1"]).arg(path),
            "sgdisk -o fat",
        );
        package_live::run_cmd(
            Command::new("sgdisk")
                .arg("-n")
                .arg(format!("1:2048:+{size_mb}M"))
                .arg("-t")
                .arg("1:EF00")
                .arg(path),
            "sgdisk p1",
        );
        let p1 = package_live::partition_first_sector(path, 1).unwrap();
        let p1_end = package_live::partition_range(path, 1).unwrap().1;
        let sectors = p1_end - p1 + 1;
        let mut f = fs::OpenOptions::new().write(true).open(path).unwrap();
        f.seek(std::io::SeekFrom::Start(p1 * 512)).unwrap();
        let mut boot = [0u8; 512];
        boot[510] = 0x55;
        boot[511] = 0xAA;
        boot[11..13].copy_from_slice(&512u16.to_le_bytes());
        boot[13] = 1;
        boot[16] = 2;
        boot[17..19].copy_from_slice(&512u16.to_le_bytes());
        boot[22..24].copy_from_slice(&1u16.to_le_bytes());
        f.write_all(&boot).unwrap();
        f.write_all(&vec![0u8; 512 * 2]).unwrap();
        f.write_all(&vec![0u8; 512 * 32]).unwrap();
        let rest = sectors.saturating_sub(1 + 2 + 32);
        if rest > 0 {
            f.write_all(&vec![0u8; (rest * 512) as usize]).unwrap();
        }
    }

    #[test]
    fn incremental_p1_preserves_p3() {
        if !require_sgdisk() {
            eprintln!("incremental_p1_preserves_p3: sin sgdisk, omito");
            return;
        }
        let dir = std::env::temp_dir().join(format!("soso-flash-inc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let usb = dir.join("usb.img");
        let src = dir.join("esp.img");
        create_test_live_img(&usb, 32, 64, 128);
        write_pattern_to_p3(&usb, 0xAB);
        minimal_fat_img(&src, 32);

        package_live::validate_live_usb(&usb).unwrap();
        package_live::dd_partition(&src, 1, &usb, 1).unwrap();
        package_live::create_esp_slots(&usb);

        assert_eq!(read_p3_first_byte(&usb), 0xAC);
        let p1 = package_live::partition_first_sector(&usb, 1).unwrap();
        let mut drv = [0u8; 11];
        drv[..8].copy_from_slice(b"SOSODRV ");
        drv[8..11].copy_from_slice(b"TXT");
        assert!(
            crate::fat32_write::find_root_entry(&usb, p1, &drv)
                .unwrap()
                .is_some(),
            "hueco ESP SOSODRV no creado"
        );
    }

    #[test]
    fn dd_partition_rejects_smaller_dest() {
        if !require_sgdisk() {
            eprintln!("dd_partition_rejects_smaller_dest: sin sgdisk, omito");
            return;
        }
        let dir = std::env::temp_dir().join(format!("soso-flash-rej-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src = dir.join("big.img");
        let dst = dir.join("small.img");
        create_test_live_img(&src, 64, 32, 32);
        create_test_live_img(&dst, 32, 32, 32);

        let err = package_live::dd_partition(&src, 1, &dst, 1).unwrap_err();
        assert!(err.contains("más pequeña"), "{err}");
    }

    #[test]
    fn parse_only_spec_accepts_aliases() {
        let parts = parse_only_spec("kernel,rootfs");
        assert_eq!(parts, vec![FlashPart::Kernel, FlashPart::Rootfs]);
        let parts = parse_only_spec("esp,data");
        assert_eq!(parts, vec![FlashPart::Kernel, FlashPart::Rootfs]);
    }
}
