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

/// Texto de confirmación antes de escribir en un disco de bloques.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceConfirm {
    #[default]
    WipeDisk,
    LiveUpdateEsp,
    LiveUpdateRootfs,
    LiveUpdateEspRootfs,
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
    validate_device(
        dev,
        opts.yes,
        opts.exclude_disk.as_deref(),
        DeviceConfirm::WipeDisk,
    );

    println!("install-disk: escribiendo {} → {}", live.display(), dev.display());
    run_cmd(
        Command::new("dd")
            .arg(format!("if={}", live.display()))
            .arg(format!("of={}", dev.display()))
            .args([
                "bs=1M",
                "status=progress",
                "iflag=direct",
                "oflag=direct",
                "conv=fsync",
            ]),
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

pub(crate) fn validate_device(
    dev: &Path,
    yes: bool,
    exclude: Option<&Path>,
    confirm: DeviceConfirm,
) {
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
        let (tool, msg) = match confirm {
            DeviceConfirm::WipeDisk => (
                "install-disk",
                format!(
                    "ATENCIÓN — se borrará TODO el contenido de {}",
                    dev.display()
                ),
            ),
            DeviceConfirm::LiveUpdateEsp => (
                "flash-usb-live",
                format!(
                    "actualizar solo la ESP (kernel) en {} — rootfs y modelos (p3) no se tocan",
                    dev.display()
                ),
            ),
            DeviceConfirm::LiveUpdateRootfs => (
                "flash-usb-live",
                format!(
                    "actualizar solo rootfs (p2) en {} — ESP y modelos (p3) no se tocan",
                    dev.display()
                ),
            ),
            DeviceConfirm::LiveUpdateEspRootfs => (
                "flash-usb-live",
                format!(
                    "actualizar ESP y rootfs en {} — modelos (p3) no se tocan",
                    dev.display()
                ),
            ),
        };
        eprintln!("{tool}: {msg}");
        eprint!("¿Continuar? [y/N] ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_err() {
            exit(1);
        }
        let ok = matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes" | "s" | "si");
        if !ok {
            eprintln!("{tool}: cancelado");
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

/// Desmonta particiones del disco (p. ej. SOSOINSTALL que Linux remonta solo).
pub(crate) fn unmount_partitions(dev: &Path) -> Result<(), String> {
    let Some(mounted) = mounted_partitions(dev) else {
        return Ok(());
    };
    for line in &mounted {
        let Some((part, mp)) = line.split_once(" → ") else {
            continue;
        };
        let ok = run_ok(Command::new("umount").arg(mp))
            || run_ok(Command::new("umount").arg(part))
            || run_ok(
                Command::new("udisksctl")
                    .args(["unmount", "-b", part, "--no-user-interaction"]),
            );
        if !ok {
            return Err(format!("no pude desmontar {part} ({mp})"));
        }
        println!("desmontado {part} ({mp})");
    }
    if let Some(still) = mounted_partitions(dev) {
        return Err(format!("siguen montadas: {}", still.join(", ")));
    }
    Ok(())
}

/// Tras el `dd`, Linux remonta p4 y udisks no puede expulsar (EBUSY al fsync).
/// Desmonta, vacía caché y apaga el USB; si udisks falla, lo quita del kernel.
pub(crate) fn release_removable(dev: &Path) {
    if !is_real_block(dev) {
        return;
    }
    let _ = Command::new("udevadm")
        .args(["settle", "--timeout=8"])
        .status();
    if let Err(e) = unmount_partitions(dev) {
        eprintln!("aviso: {e}");
    }
    let _ = Command::new("sync").status();
    let _ = Command::new("blockdev")
        .args(["--flushbufs"])
        .arg(dev)
        .status();
    if let Err(e) = unmount_partitions(dev) {
        eprintln!("aviso: {e}");
    }

    if run_ok(
        Command::new("udisksctl")
            .args(["power-off", "-b"])
            .arg(dev)
            .args(["--no-user-interaction"]),
    ) {
        println!("USB apagado — ya puedes desenchufarlo");
        return;
    }
    let _ = Command::new("eject").arg(dev).status();
    if !dev.exists() {
        println!("USB expulsado — ya puedes desenchufarlo");
        return;
    }

    let Some(name) = dev.file_name() else {
        warn_eject(dev);
        return;
    };
    let sys = PathBuf::from(format!(
        "/sys/block/{}/device/delete",
        name.to_string_lossy()
    ));
    if sys.exists() && std::fs::write(&sys, b"1\n").is_ok() && !dev.exists() {
        println!("USB liberado — ya puedes desenchufarlo");
        return;
    }
    warn_eject(dev);
}

fn warn_eject(dev: &Path) {
    let name = dev.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    eprintln!(
        "aviso: no pude expulsar {}. Datos sincronizados.\n\
         Si el escritorio no lo suelta:\n\
           echo 1 | sudo tee /sys/block/{name}/device/delete",
        dev.display()
    );
}

fn is_real_block(dev: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(dev)
        .map(|m| m.file_type().is_block_device())
        .unwrap_or(false)
}

fn run_ok(cmd: &mut Command) -> bool {
    cmd.status().map(|s| s.success()).unwrap_or(false)
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
    parse_lsblk_mounts(&String::from_utf8_lossy(&out.stdout), &base)
}

fn parse_lsblk_mounts(stdout: &str, base: &str) -> Option<Vec<String>> {
    let mut mounted = Vec::new();
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else {
            continue;
        };
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
    // El inicio EXACTO de p3, leído antes de borrarla.
    //
    // AVERÍA (2026-08-17, dos pendrives): `-d 3 -n 3:0:…` y sgdisk elige el
    // primer sector libre **realineado** — 583714 → 583720. La partición
    // quedaba 6 sectores por delante de los datos del `dd` y sosomfs arrancaba
    // `Corrupt`. Recrearla en su sitio no basta: hace falta `-a 1`.
    let Some(inicio) = crate::package_live::partition_first_sector(dev, 3) else {
        eprintln!("install-disk: no pude leer el inicio de p3; no la amplío");
        return;
    };
    println!(
        "install-disk: ampliando partición 3 hasta el final del disco ({img_sectors} sectores)"
    );
    run_cmd(
        Command::new("sgdisk")
            .arg("-e")
            .arg(dev)
            .arg("-d")
            .arg("3")
            .arg("-a")
            .arg("1")
            .arg("-n")
            .arg(format!("3:{inicio}:0"))
            .arg("-t")
            .arg("3:8300"),
        "sgdisk expand",
    );
}

/// Reubica la GPT de respaldo al final del disco (pendrives > tamaño de soso-live.img).
pub(crate) fn repair_gpt_backup(dev: &Path) {
    run_cmd(
        Command::new("sgdisk").arg("-e").arg(dev),
        "sgdisk repair",
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lsblk_finds_sosoinstall() {
        let out = "sdd\n\
sdd1\n\
sdd4 /media/jmdiez/SOSOINSTALL\n";
        let mounted = parse_lsblk_mounts(out, "sdd").unwrap();
        assert_eq!(mounted, vec!["/dev/sdd4 → /media/jmdiez/SOSOINSTALL"]);
    }

    #[test]
    fn parse_lsblk_ignores_disk_without_mounts() {
        assert!(parse_lsblk_mounts("sdd\nsdd1\nsdd4\n", "sdd").is_none());
    }
}
