//! `cargo xtask flash-usb-live <usb> [--yes]`
//!
//! Graba `soso-live.img` en un pendrive y estira p3 (sosomfs). p4 SOSOINSTALL
//! ya va en la imagen (FAT entre rootfs y modelos), para que Linux la monte.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

use crate::install_disk;
use crate::package_live;

pub fn run(args: &[String]) {
    let mut device: Option<PathBuf> = None;
    let mut yes = false;

    for a in args {
        match a.as_str() {
            "--yes" | "-y" => yes = true,
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
    }

    let Some(usb) = device else {
        usage();
    };

    install_disk::validate_device(&usb, yes, None);

    let root = super::project_root();
    let disk_bytes = blockdev_bytes(&usb).unwrap_or_else(|| {
        eprintln!("flash-usb-live: no pude leer el tamaño de {}", usb.display());
        exit(1);
    });
    println!(
        "flash-usb-live: pendrive {} ({})",
        usb.display(),
        crate::live_models::format_bytes(disk_bytes)
    );

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

    install_disk::expand_models_partition(&usb, &live);
    install_disk::repair_gpt_backup(&usb);

    print_flash_summary(&usb, &out_dir);
    let _ = root;
}

fn usage() -> ! {
    eprintln!(
        "uso: cargo xtask flash-usb-live <usb> [--yes]\n\
         \n\
         Ejemplo:\n\
           lsblk\n\
           sudo cargo xtask flash-usb-live /dev/sde --yes\n\
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
         2) Instalar en disco interno (Linux en marcha, USB conectado):\n\
            lsblk\n\
            sudo {}/install-soso.sh /dev/nvme1n1 --yes\n\
         \n\
         Linux monta sola p4 SOSOINSTALL (FAT tras el rootfs, no la ESP).\n\
         Si no aparece: sudo mount /dev/sdX4 /mnt && /mnt/install-soso.sh …\n\
         \n\
         Artefactos también en {}",
        out_dir.display(),
        out_dir.display()
    );
}
