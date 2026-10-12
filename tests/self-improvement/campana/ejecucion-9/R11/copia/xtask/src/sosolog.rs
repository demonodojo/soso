//! `cargo xtask sosolog [dispositivo]`
//!
//! Monta la ESP del USB live, imprime `SOSOLOG.TXT` y desmonta.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, exit};

use crate::install_disk;

const LOG_NAME: &str = "SOSOLOG.TXT";
/// Informe hwscan que vuelca `drivers::drvlog` en cada arranque del live.
const DRV_NAME: &str = "SOSODRV.TXT";

pub fn run(args: &[String]) {
    let mut device: Option<PathBuf> = None;
    let mut file = LOG_NAME;
    for a in args {
        match a.as_str() {
            "-h" | "--help" => usage(),
            "--drv" | "--hwscan" => file = DRV_NAME,
            s if s.starts_with('-') => {
                eprintln!("sosolog: opción desconocida: {s}");
                usage();
            }
            s => {
                if device.is_some() {
                    eprintln!("sosolog: demasiados argumentos");
                    usage();
                }
                device = Some(PathBuf::from(s));
            }
        }
    }

    let esp = match device {
        Some(dev) => resolve_esp(&dev),
        None => auto_detect_esp(),
    };

    match dump_log(&esp, file) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("sosolog: {e}");
            exit(1);
        }
    }
}

fn usage() -> ! {
    eprintln!(
        "uso: cargo xtask sosolog [--drv] [dispositivo]\n\
         \n\
         Monta la ESP (partición 1) del USB live, muestra SOSOLOG.TXT y desmonta.\n\
         Sin argumento, busca un pendrive soso (ESP FAT, no el disco de Linux).\n\
         \n\
         --drv (= --hwscan)  muestra SOSODRV.TXT: el informe hwscan del último\n\
         arranque, una línea por dispositivo PCI con su driver y su estado.\n\
         \n\
         Ejemplos:\n\
           cargo xtask sosolog\n\
           cargo xtask sosolog --drv\n\
           cargo xtask sosolog /dev/sdX\n\
           cargo xtask sosolog --drv /dev/sdX1"
    );
    exit(2);
}

fn resolve_esp(dev: &Path) -> PathBuf {
    if !dev.exists() {
        eprintln!("sosolog: no existe {}", dev.display());
        exit(1);
    }
    if looks_like_partition(dev) {
        return dev.to_path_buf();
    }
    install_disk::esp_partition_path(dev)
}

fn looks_like_partition(dev: &Path) -> bool {
    let s = dev.to_string_lossy();
    if s.contains('p') && s.chars().last().is_some_and(|c| c.is_ascii_digit()) {
        let base = s.rsplit_once('p').map(|(b, _)| b).unwrap_or(&s);
        if base.contains("nvme") || base.contains("mmcblk") {
            return true;
        }
    }
    s.chars().last().is_some_and(|c| c.is_ascii_digit())
        && !s.contains("nvme")
        && !s.ends_with("n1")
}

fn auto_detect_esp() -> PathBuf {
    let linux = install_disk::linux_root_disk();
    let mut found = Vec::new();
    for part in list_vfat_parts() {
        if part.label.eq_ignore_ascii_case("SOSOINSTALL") {
            continue;
        }
        if let Some(ref root) = linux {
            if let Some(pk) = install_disk::parent_disk(&part.name.to_string_lossy()) {
                if install_disk::same_disk(root, &pk) {
                    continue;
                }
            }
        }
        let efi = part.parttype.to_ascii_lowercase().contains("efi");
        let first = part
            .name
            .to_string_lossy()
            .ends_with('1');
        if efi || first {
            found.push(part);
        }
    }

    if found.is_empty() {
        eprintln!(
            "sosolog: no encontré una ESP de USB live.\n\
             Pasa el dispositivo: cargo xtask sosolog /dev/sdX\n\
             (lsblk — la ESP es la partición 1, tipo EFI / vfat)"
        );
        exit(1);
    }
    if found.len() > 1 {
        found.sort_by_key(|p| !p.removable);
        let names: Vec<_> = found.iter().map(|p| p.name.display().to_string()).collect();
        eprintln!(
            "sosolog: varios candidatos ({}); uso {}",
            names.join(", "),
            found[0].name.display()
        );
    }
    found[0].name.clone()
}

struct Part {
    name: PathBuf,
    label: String,
    parttype: String,
    removable: bool,
}

fn list_vfat_parts() -> Vec<Part> {
    let out = Command::new("lsblk")
        .args(["-Ppo", "NAME,TYPE,FSTYPE,LABEL,PARTTYPENAME,RM"])
        .output();
    let Ok(out) = out else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut parts = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let typ = lsblk_field(line, "TYPE");
        if typ != "part" {
            continue;
        }
        let fstype = lsblk_field(line, "FSTYPE");
        if fstype != "vfat" && fstype != "fat32" && fstype != "msdos" {
            continue;
        }
        let name = lsblk_field(line, "NAME");
        if name.is_empty() {
            continue;
        }
        parts.push(Part {
            name: PathBuf::from(name),
            label: lsblk_field(line, "LABEL"),
            parttype: lsblk_field(line, "PARTTYPENAME"),
            removable: lsblk_field(line, "RM") == "1",
        });
    }
    parts
}

fn lsblk_field(line: &str, key: &str) -> String {
    let needle = format!("{key}=\"");
    let rest = match line.find(&needle) {
        Some(i) => &line[i + needle.len()..],
        None => return String::new(),
    };
    match rest.find('"') {
        Some(end) => rest[..end].to_string(),
        None => String::new(),
    }
}

fn dump_log(esp: &Path, file: &str) -> Result<(), String> {
    if !esp.exists() {
        return Err(format!("no existe {}", esp.display()));
    }
    let mount = EspMount::acquire(esp)?;
    let path = mount.point.join(file);
    if !path.exists() {
        return Err(format!(
            "no está {file} en {} (¿imagen live antigua?)",
            esp.display()
        ));
    }
    let raw = fs::read(&path).map_err(|e| format!("leer {}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&raw);
    let body = text.trim_end_matches(['\n', '\r', '\0']);
    eprintln!("sosolog: {} → {file}", esp.display());
    if body.is_empty() {
        eprintln!(
            "sosolog: el fichero existe pero está vacío (el kernel no llegó a volcar el log)"
        );
        return Ok(());
    }
    let mut out = io::stdout().lock();
    out.write_all(body.as_bytes())
        .and_then(|_| out.write_all(b"\n"))
        .map_err(|e| format!("stdout: {e}"))?;
    Ok(())
}

struct EspMount {
    point: PathBuf,
    owned: bool,
}

impl EspMount {
    fn acquire(esp: &Path) -> Result<Self, String> {
        if let Some(existing) = findmnt_target(esp) {
            return Ok(Self {
                point: existing,
                owned: false,
            });
        }
        let point = std::env::temp_dir().join(format!("soso-esp-{}", std::process::id()));
        fs::create_dir_all(&point).map_err(|e| format!("mkdir {}: {e}", point.display()))?;
        let opts = mount_options();
        let st = priv_command("mount")
            .args(["-o", &opts])
            .arg(esp)
            .arg(&point)
            .status()
            .map_err(|e| format!("mount: {e}"))?;
        if !st.success() {
            let _ = fs::remove_dir(&point);
            return Err(format!(
                "no pude montar {} en {}",
                esp.display(),
                point.display()
            ));
        }
        Ok(Self { point, owned: true })
    }
}

impl Drop for EspMount {
    fn drop(&mut self) {
        if !self.owned {
            return;
        }
        let _ = priv_command("umount").arg(&self.point).status();
        let _ = fs::remove_dir(&self.point);
    }
}

fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| s.trim() == "0")
}

fn priv_command(prog: &str) -> Command {
    if is_root() {
        Command::new(prog)
    } else {
        let mut c = Command::new("sudo");
        c.arg(prog);
        c
    }
}

fn mount_options() -> String {
    if is_root() {
        return "ro".into();
    }
    let uid = id_num("-u").unwrap_or(0);
    let gid = id_num("-g").unwrap_or(0);
    format!("ro,uid={uid},gid={gid}")
}

fn id_num(flag: &str) -> Option<u32> {
    Command::new("id")
        .arg(flag)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
}

fn findmnt_target(dev: &Path) -> Option<PathBuf> {
    let out = Command::new("findmnt")
        .args(["-n", "-o", "TARGET"])
        .arg(dev)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}
