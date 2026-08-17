//! `cargo xtask flash-usb-live <usb> [--yes]`
//!
//! Graba `soso-live.img` en un pendrive, estira p3 (sosomfs) y añade p4 SOSOINSTALL.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

use crate::install_disk;
use crate::package_live;

const INSTALL_SECTORS: u64 = 65536; // 32 MiB

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

    let img_sectors = std::fs::metadata(&live).map(|m| m.len() / 512).unwrap_or(0);
    let disk_sectors = blockdev_sectors(&usb).unwrap_or(0);

    if disk_sectors >= img_sectors + INSTALL_SECTORS {
        install_disk::expand_models_leave_install(&usb, INSTALL_SECTORS);
        if let Some(part4) = install_disk::create_install_partition_tail(&usb, INSTALL_SECTORS) {
            install_disk::run_cmd(
                Command::new("mkfs.vfat")
                    .args(["-F", "32", "-n", "SOSOINSTALL"])
                    .arg(&part4),
                "mkfs.vfat",
            );
            if mount_install_partition(&part4) {
                copy_installer_files(&out_dir, "/mnt/soso-install");
                let _ = Command::new("umount").arg("/mnt/soso-install").status();
                println!("flash-usb-live: instalador en {part4}");
            } else {
                eprintln!(
                    "flash-usb-live: aviso: no pude montar {part4}; copia manual desde {}",
                    out_dir.display()
                );
            }
        }
    } else {
        eprintln!(
            "flash-usb-live: aviso: USB sin espacio para p4 ({INSTALL_SECTORS} sectores);\n\
             copia {} a un directorio accesible desde Linux",
            out_dir.display()
        );
    }

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
         Graba soso-live.img (modelo según tamaño del stick), estira p3 (modelos)\n\
         y añade p4 SOSOINSTALL\n\
         con install-soso.sh para dual-boot desde Linux."
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

fn mount_install_partition(part: &str) -> bool {
    let _ = std::fs::create_dir_all("/mnt/soso-install");
    Command::new("mount")
        .arg(part)
        .arg("/mnt/soso-install")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn copy_installer_files(src_dir: &Path, dst: &str) {
    for name in [
        "install-soso.sh",
        "soso-live.bytes",
        "INSTALL.txt",
    ] {
        let src = src_dir.join(name);
        if src.exists() {
            let _ = std::fs::copy(&src, Path::new(dst).join(name));
        }
    }
    let sh = Path::new(dst).join("install-soso.sh");
    if sh.exists() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755));
    }
}

fn print_flash_summary(usb: &Path, out_dir: &Path) {
    println!("\n✅ USB live listo en {}", usb.display());
    println!(
        "\n1) Probar soso: UEFI → arrancar desde USB (F12).\n\
         2) Instalar en disco interno (Linux en marcha, USB conectado):\n\
            lsblk\n\
            sudo {}/install-soso.sh /dev/nvme1n1 --yes\n\
         \n\
         (Si la partición SOSOINSTALL no se montó sola: sudo mount /dev/sdX4 /mnt\n\
          y ejecuta /mnt/install-soso.sh …)\n\
         \n\
         Artefactos también en {}",
        out_dir.display(),
        out_dir.display()
    );
}
