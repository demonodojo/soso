//! `cargo xtask install-disk <dispositivo> [--yes] [--no-grub]`
//!
//! Escribe `soso-live.img` en un disco vacío y añade entrada GRUB para dual-boot UEFI.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, exit};

use crate::package_live;

pub struct InstallOpts {
    pub yes: bool,
    pub no_grub: bool,
    /// Disco a excluir como destino (p. ej. el USB de origen).
    pub exclude_disk: Option<PathBuf>,
}

pub fn run(args: &[String]) {
    let mut device: Option<PathBuf> = None;
    let mut yes = false;
    let mut no_grub = false;

    for a in args {
        match a.as_str() {
            "--yes" | "-y" => yes = true,
            "--no-grub" => no_grub = true,
            s if s.starts_with('-') => {
                eprintln!("install-disk: opción desconocida: {s}");
                usage();
            }
            s => {
                if device.is_some() {
                    eprintln!("install-disk: demasiados argumentos");
                    usage();
                }
                device = Some(PathBuf::from(s));
            }
        }
    }

    let Some(dev) = device else {
        usage();
    };

    let root = super::project_root();
    super::build_user();
    let _ = super::build_image();
    package_live::run();

    let live = package_live::live_image_path();
    if !live.exists() {
        eprintln!("install-disk: falta {}", live.display());
        exit(1);
    }

    install_from_image(
        &live,
        &dev,
        &root,
        InstallOpts {
            yes,
            no_grub,
            exclude_disk: None,
        },
    );
}

pub fn install_from_image(live: &Path, dev: &Path, root: &Path, opts: InstallOpts) {
    validate_device(dev, opts.yes, opts.exclude_disk.as_deref());

    println!("install-disk: escribiendo {} → {}", live.display(), dev.display());
    run_cmd(
        Command::new("dd")
            .arg(format!("if={}", live.display()))
            .arg(format!("of={}", dev.display()))
            .args(["bs=4M", "status=progress", "conv=fsync"]),
        "dd",
    );
    run_cmd(&mut Command::new("sync"), "sync");

    expand_models_partition(dev, live);

    let esp_path = esp_partition_path(dev);
    let uuid = esp_uuid(&esp_path).unwrap_or_else(|| {
        eprintln!(
            "install-disk: aviso: no pude leer UUID de {}",
            esp_path.display()
        );
        String::new()
    });

    if opts.no_grub {
        print_manual_grub(root, &uuid);
        print_summary(dev, &uuid, false);
        return;
    }

    if !install_grub(&uuid) {
        print_manual_grub(root, &uuid);
        print_summary(dev, &uuid, false);
        return;
    }

    print_summary(dev, &uuid, true);
}

fn usage() -> ! {
    eprintln!(
        "uso: cargo xtask install-disk <dispositivo> [--yes] [--no-grub]\n\
         \n\
         Ejemplo:\n\
           lsblk\n\
           sudo cargo xtask install-disk /dev/nvme1n1 --yes\n\
         \n\
         Escribe soso-live.img en el disco indicado y añade una entrada GRUB \"soso\".\n\
         Requiere UEFI, sgdisk, dd, blkid y permisos root para GRUB."
    );
    exit(2);
}

pub(crate) fn validate_device(dev: &Path, yes: bool, exclude: Option<&Path>) {
    if !dev.exists() {
        eprintln!("install-disk: no existe {}", dev.display());
        exit(1);
    }
    if is_partition_path(dev) {
        eprintln!(
            "install-disk: pasa el disco entero (p. ej. /dev/nvme1n1), no una partición"
        );
        exit(1);
    }

    if exclude.is_some_and(|e| same_disk(e, dev)) {
        eprintln!(
            "install-disk: {} es el disco de origen (USB); elige otro destino",
            dev.display()
        );
        exit(1);
    }

    let root_disk = linux_root_disk();
    if root_disk.as_ref().is_some_and(|r| same_disk(r, dev)) {
        eprintln!(
            "install-disk: {} parece el disco de Linux ({})",
            dev.display(),
            root_disk.as_ref().unwrap().display()
        );
        exit(1);
    }

    if let Some(mounted) = mounted_partitions(dev) {
        eprintln!("install-disk: particiones montadas en {}:", dev.display());
        for m in mounted {
            eprintln!("  {m}");
        }
        exit(1);
    }

    if !yes {
        eprintln!(
            "install-disk: ATENCIÓN — se borrará TODO el contenido de {}",
            dev.display()
        );
        eprint!("¿Continuar? [y/N] ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_err() {
            exit(1);
        }
        let ok = matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes" | "s" | "si");
        if !ok {
            eprintln!("install-disk: cancelado");
            exit(1);
        }
    }
}

fn is_partition_path(dev: &Path) -> bool {
    let s = dev.to_string_lossy();
    if s.contains("p") && s.chars().last().is_some_and(|c| c.is_ascii_digit()) {
        let base = s.rsplit_once('p').map(|(b, _)| b).unwrap_or(&s);
        return base.contains("nvme") || base.contains("mmcblk");
    }
    s.chars().last().is_some_and(|c| c.is_ascii_digit())
        && !s.contains("nvme")
        && !s.ends_with("n1")
}

pub(crate) fn linux_root_disk() -> Option<PathBuf> {
    for mp in ["/", "/boot", "/boot/efi"] {
        if let Some(src) = findmnt_source(mp) {
            if let Some(disk) = parent_disk(&src) {
                return Some(disk);
            }
        }
    }
    None
}

fn findmnt_source(mountpoint: &str) -> Option<String> {
    let out = Command::new("findmnt")
        .args(["-n", "-o", "SOURCE"])
        .arg(mountpoint)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

pub(crate) fn parent_disk(dev: &str) -> Option<PathBuf> {
    let out = Command::new("lsblk")
        .args(["-no", "PKNAME"])
        .arg(dev)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let pk = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if pk.is_empty() {
        Some(PathBuf::from(dev))
    } else {
        Some(PathBuf::from(format!("/dev/{pk}")))
    }
}

pub(crate) fn same_disk(a: &Path, b: &Path) -> bool {
    canonical(a) == canonical(b)
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn mounted_partitions(dev: &Path) -> Option<Vec<String>> {
    let out = Command::new("lsblk")
        .args(["-rn", "-o", "NAME,MOUNTPOINT"])
        .arg(dev)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let base = dev.file_name()?.to_string_lossy();
    let mut mounted = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.split_whitespace();
        let name = parts.next()?;
        let mp = parts.next().unwrap_or("");
        if !mp.is_empty() && name != base {
            mounted.push(format!("/dev/{name} → {mp}"));
        }
    }
    if mounted.is_empty() { None } else { Some(mounted) }
}

pub(crate) fn expand_models_partition(dev: &Path, live: &Path) {
    let img_sectors = disk_sectors(dev);
    let live_sectors = std::fs::metadata(live).map(|m| m.len() / 512).unwrap_or(0);
    if img_sectors <= live_sectors {
        return;
    }
    println!(
        "install-disk: ampliando partición 3 hasta el final del disco ({img_sectors} sectores)"
    );
    run_cmd(
        Command::new("sgdisk")
            .arg("-e")
            .arg(dev)
            .arg("-d")
            .arg("3")
            .arg("-n")
            .arg("3:0:0")
            .arg("-t")
            .arg("3:8300"),
        "sgdisk expand",
    );
}

/// Estira p3 (sosomfs) dejando `install_reserve_sectors` al final para p4 (SOSOINSTALL).
pub(crate) fn expand_models_leave_install(dev: &Path, install_reserve_sectors: u64) {
    let disk_sectors = disk_sectors(dev);
    if disk_sectors <= install_reserve_sectors + 64 {
        return;
    }
    let tail = install_reserve_sectors + 34;
    println!(
        "flash-usb-live: ampliando p3 (reservando {} MiB para instalador)",
        install_reserve_sectors * 512 / (1024 * 1024)
    );
    run_cmd(Command::new("sgdisk").arg("-e").arg(dev), "sgdisk -e");
    run_cmd(
        Command::new("sgdisk")
            .arg("-d")
            .arg("3")
            .arg("-n")
            .arg(format!("3:0:-{tail}S"))
            .arg("-t")
            .arg("3:8300")
            .arg(dev),
        "sgdisk expand p3",
    );
}

/// Crea p4 SOSOINSTALL en los últimos `install_sectors` del disco.
pub(crate) fn create_install_partition_tail(dev: &Path, install_sectors: u64) -> Option<String> {
    run_cmd(
        Command::new("sgdisk")
            .arg("-n")
            .arg(format!("4:-{install_sectors}S:0"))
            .arg("-t")
            .arg("4:0700")
            .arg("-c")
            .arg("4:SOSOINSTALL")
            .arg(dev),
        "sgdisk part4",
    );
    install_partition_node(dev, 4)
}

pub(crate) fn install_partition_node(disk: &Path, num: u32) -> Option<String> {
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

fn disk_sectors(dev: &Path) -> u64 {
    let out = Command::new("blockdev")
        .args(["--getsz"])
        .arg(dev)
        .output()
        .ok();
    if let Some(o) = out.filter(|o| o.status.success()) {
        if let Ok(s) = String::from_utf8(o.stdout) {
            if let Ok(n) = s.trim().parse::<u64>() {
                return n;
            }
        }
    }
    0
}

pub(crate) fn esp_partition_path(dev: &Path) -> PathBuf {
    let esp = format!("{}p1", dev.display());
    if Path::new(&esp).exists() {
        PathBuf::from(esp)
    } else {
        PathBuf::from(format!("{}1", dev.display()))
    }
}

pub(crate) fn esp_uuid(esp: &Path) -> Option<String> {
    let out = Command::new("blkid")
        .args(["-s", "UUID", "-o", "value"])
        .arg(esp)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let u = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if u.is_empty() { None } else { Some(u) }
}

pub(crate) fn grub_snippet(uuid: &str) -> String {
    if uuid.is_empty() {
        return r#"#!/bin/sh
exec tail -n +3 "$0"
menuentry "soso" {
    insmod part_gpt
    insmod fat
    # Sustituye SOSO_ESP_UUID por el UUID de la ESP (partición 1) del disco soso:
    #   sudo blkid /dev/nvme1n1p1
    search --no-floppy --fs-uuid --set=root SOSO_ESP_UUID
    chainloader /EFI/BOOT/BOOTX64.EFI
}
"#
        .to_string();
    }
    format!(
        r#"#!/bin/sh
exec tail -n +3 "$0"
menuentry "soso" {{
    insmod part_gpt
    insmod fat
    search --no-floppy --fs-uuid --set=root {uuid}
    chainloader /EFI/BOOT/BOOTX64.EFI
}}
"#
    )
}

pub(crate) fn install_grub(uuid: &str) -> bool {
    let snippet = grub_snippet(uuid);
    let target = PathBuf::from("/etc/grub.d/41_soso");
    if std::fs::write(&target, &snippet).is_err() {
        eprintln!("install-disk: no pude escribir {} (¿root?)", target.display());
        return false;
    }
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755));
    }
    println!("install-disk: {}", target.display());

    if Command::new("update-grub").status().map(|s| s.success()).unwrap_or(false) {
        println!("install-disk: update-grub OK");
        return true;
    }
    if Command::new("grub-mkconfig")
        .args(["-o", "/boot/grub/grub.cfg"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        println!("install-disk: grub-mkconfig OK");
        return true;
    }
    eprintln!("install-disk: aviso: no pude regenerar grub.cfg");
    false
}

fn print_manual_grub(root: &Path, uuid: &str) {
    let out_dir = root.join("target/install-disk");
    let _ = std::fs::create_dir_all(&out_dir);
    let path = out_dir.join("41_soso");
    let body = grub_snippet(uuid);
    if std::fs::write(&path, &body).is_ok() {
        println!(
            "install-disk: snippet GRUB en {}\n\
             Instálalo con:\n\
               sudo cp {} /etc/grub.d/41_soso\n\
               sudo chmod 755 /etc/grub.d/41_soso\n\
               sudo update-grub",
            path.display(),
            path.display()
        );
    }
}

fn print_summary(dev: &Path, uuid: &str, grub_ok: bool) {
    println!("\n✅ soso instalado en {}", dev.display());
    if !uuid.is_empty() {
        println!("   ESP UUID: {uuid}");
    }
    if grub_ok {
        println!("   Reinicia y elige \"soso\" en el menú GRUB.");
    } else {
        println!("   Completa la entrada GRUB (ver arriba) y reinicia.");
    }
    println!(
        "\nDesinstalar:\n\
           sudo rm /etc/grub.d/41_soso && sudo update-grub\n\
           (el disco soso puede borrarse o reutilizarse aparte)"
    );
}

pub(crate) fn run_cmd(cmd: &mut Command, label: &str) {
    let st = cmd.status().unwrap_or_else(|e| panic!("{label}: {e}"));
    if !st.success() {
        eprintln!("install-disk: {label} falló");
        exit(st.code().unwrap_or(1));
    }
}
