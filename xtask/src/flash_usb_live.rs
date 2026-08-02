//! `cargo xtask flash-usb-live <usb> [--yes]`
//!
//! Graba `soso-live.img` en un pendrive y añade partición FAT con el instalador.

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
    super::build_user();
    let _ = super::build_image();
    package_live::run();

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

    if let Some(part4) = add_install_partition(&usb) {
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
    } else {
        eprintln!(
            "flash-usb-live: aviso: USB sin espacio libre para partición instalador;\n\
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
         Graba soso-live.img en el pendrive y añade partición SOSO_INSTALL\n\
         con install-soso.sh para dual-boot desde Linux."
    );
    exit(2);
}

fn add_install_partition(usb: &Path) -> Option<String> {
    let img_sectors = std::fs::metadata(package_live::live_image_path())
        .ok()?
        .len()
        / 512;
    let disk_sectors = blockdev_sectors(usb)?;
    // Al menos 32 MiB libres para la partición instalador.
    if disk_sectors < img_sectors + 65536 {
        return None;
    }

    install_disk::run_cmd(Command::new("sgdisk").arg("-e").arg(usb), "sgdisk -e");
    install_disk::run_cmd(
        Command::new("sgdisk")
            .arg("-n")
            .arg("4:0:0")
            .arg("-t")
            .arg("4:0700")
            .arg("-c")
            .arg("4:SOSO_INSTALL")
            .arg(usb),
        "sgdisk part4",
    );

    let part = install_partition_node(usb, 4)?;
    install_disk::run_cmd(
        Command::new("mkfs.vfat")
            .args(["-F", "32", "-n", "SOSO_INSTALL"])
            .arg(&part),
        "mkfs.vfat",
    );
    Some(part)
}

fn install_partition_node(disk: &Path, num: u32) -> Option<String> {
    let base = disk.to_string_lossy();
    let candidate = if base.contains("nvme") || base.contains("mmcblk") {
        format!("{base}p{num}")
    } else {
        format!("{base}{num}")
    };
    if Path::new(&candidate).exists() {
        Some(candidate)
    } else {
        None
    }
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
         (Si la partición SOSO_INSTALL no se montó sola: sudo mount /dev/sdX4 /mnt\n\
          y ejecuta /mnt/install-soso.sh …)\n\
         \n\
         Artefactos también en {}",
        out_dir.display(),
        out_dir.display()
    );
}
