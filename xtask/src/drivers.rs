//! Perfiles de drivers, fit-drivers y ports externos.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, exit};

/// Perfil de drivers para empaquetar el kernel y rootfs.
#[derive(Clone, Debug, Default)]
pub struct DriverProfile {
    pub kernel_features: Vec<String>,
    pub lxdde_ports: Vec<String>,
    pub lxdde_mode: Option<String>,
    /// Patrones glob de firmware a excluir del rootfs (sin barra inicial).
    pub firmware_exclude: Vec<String>,
}

const NATIVE_FEATURES: &[&str] = &[
    "drv-virtio-blk",
    "drv-virtio-net",
    "drv-e1000e",
    "drv-nvme",
    "drv-usb",
    "drv-gpu-nvidia",
    "drv-live-disk",
];

const LX_PORT_BY_DRIVER: &[(&str, &str)] = &[
    ("lx-e1000e", "e1000e"),
    ("lx-iwlwifi", "iwlwifi"),
    ("lx-nouveau", "nouveau"),
];

const DRIVER_BY_FEATURE: &[(&str, &str)] = &[
    ("drv-virtio-blk", "virtio-blk"),
    ("drv-virtio-net", "virtio-net"),
    ("drv-e1000e", "e1000e"),
    ("drv-nvme", "nvme"),
    ("drv-usb", "usb-xhci"),
    ("drv-gpu-nvidia", "gpu-nvidia"),
    ("drv-live-disk", "live-disk"),
];

/// Presets conocidos o lista separada por comas de features/preset.
pub fn profile_from_arg(spec: &str) -> DriverProfile {
    match spec.trim().to_ascii_lowercase().as_str() {
        "" | "all" => preset_all(),
        "qemu" | "minimal" => preset_qemu(),
        "live-usb" | "live" => preset_live_usb(),
        other => profile_from_list(other),
    }
}

pub fn preset_all() -> DriverProfile {
    DriverProfile {
        kernel_features: vec!["drv-all".into()],
        lxdde_ports: vec![],
        lxdde_mode: None,
        firmware_exclude: vec![],
    }
}

pub fn preset_qemu() -> DriverProfile {
    DriverProfile {
        kernel_features: vec!["drv-virtio-blk".into(), "drv-virtio-net".into()],
        lxdde_ports: vec![],
        lxdde_mode: None,
        firmware_exclude: vec![
            "lib/firmware/iwlwifi-*".into(),
            "lib/firmware/nvidia/**".into(),
        ],
    }
}

pub fn preset_live_usb() -> DriverProfile {
    DriverProfile {
        kernel_features: vec![
            "drv-virtio-blk".into(),
            "drv-virtio-net".into(),
            "drv-nvme".into(),
            "drv-usb".into(),
            "drv-live-disk".into(),
            "drv-gpu-nvidia".into(),
        ],
        lxdde_ports: vec!["nouveau".into(), "iwlwifi".into()],
        lxdde_mode: Some("nouveau,iwlwifi".into()),
        firmware_exclude: vec![],
    }
}

fn profile_from_list(spec: &str) -> DriverProfile {
    let mut feats = BTreeSet::new();
    let mut ports = BTreeSet::new();
    for part in spec.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if p.starts_with("lx-") || p == "lxdde" {
            if p == "lxdde" {
                feats.insert("lxdde".into());
            } else if let Some((_, port)) = LX_PORT_BY_DRIVER.iter().find(|(n, _)| *n == p) {
                feats.insert("lxdde".into());
                ports.insert((*port).into());
            }
            continue;
        }
        if p == "drv-all" || p == "all" {
            return preset_all();
        }
        if NATIVE_FEATURES.contains(&p) {
            feats.insert(p.into());
        } else {
            eprintln!("xtask: feature de driver desconocida: {p}");
            exit(2);
        }
    }
    DriverProfile {
        kernel_features: feats.into_iter().collect(),
        lxdde_ports: ports.into_iter().collect(),
        lxdde_mode: None,
        firmware_exclude: vec![],
    }
}

/// Lee `--drivers` de los argumentos del proceso o `SOSO_DRIVERS`.
pub fn profile_from_env_or_args() -> DriverProfile {
    let args: Vec<String> = std::env::args().collect();
    for (i, a) in args.iter().enumerate() {
        if a == "--drivers" {
            if let Some(v) = args.get(i + 1) {
                return profile_from_arg(v);
            }
        }
    }
    if let Ok(v) = std::env::var("SOSO_DRIVERS") {
        if !v.is_empty() {
            return profile_from_arg(&v);
        }
    }
    preset_all()
}

pub fn kernel_feature_args(profile: &DriverProfile) -> Vec<String> {
    let mut feats = profile.kernel_features.clone();
    if feats.is_empty() {
        feats.push("drv-all".into());
    }
    if !profile.lxdde_ports.is_empty() && !feats.iter().any(|f| f == "lxdde") {
        feats.push("lxdde".into());
    }
    if super::lxdde_enabled() && !feats.iter().any(|f| f == "lxdde") {
        feats.push("lxdde".into());
    }
    feats.sort();
    feats.dedup();
    feats
}

pub fn lx_ports_for_build(profile: &DriverProfile) -> Vec<String> {
    if !profile.lxdde_ports.is_empty() {
        return profile.lxdde_ports.clone();
    }
    if super::lxdde_enabled() {
        if let Some(mode) = super::lxdde_mode_env() {
            return vec![mode];
        }
        return vec!["all".into()];
    }
    vec![]
}

/// Excluye firmware del árbol rootfs antes de mkfs.
pub fn filter_rootfs_firmware(rootfs: &Path, profile: &DriverProfile) {
    if profile.firmware_exclude.is_empty() {
        return;
    }
    let fw = rootfs.join("lib/firmware");
    if !fw.exists() {
        return;
    }
    for pat in &profile.firmware_exclude {
        remove_glob(&fw, pat);
    }
}

fn remove_glob(base: &Path, pattern: &str) {
    let rel = pattern.strip_prefix("lib/firmware/").unwrap_or(pattern);
    if rel.contains('*') {
        if let Ok(entries) = glob_paths(base, rel) {
            for p in entries {
                let _ = std::fs::remove_file(&p);
                let _ = std::fs::remove_dir_all(&p);
            }
        }
    } else {
        let p = base.join(rel);
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_dir_all(&p);
    }
}

fn glob_paths(base: &Path, pattern: &str) -> Result<Vec<PathBuf>, ()> {
    let mut out = Vec::new();
    if pattern.ends_with("/**") {
        let dir = pattern.trim_end_matches("/**");
        let p = base.join(dir);
        if p.is_dir() {
            let _ = std::fs::remove_dir_all(&p);
        }
        return Ok(out);
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        let dir = base.join(prefix.trim_end_matches('/'));
        if dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    out.push(e.path());
                }
            }
        }
        return Ok(out);
    }
    Ok(out)
}

/// Entrada parseada de una línea hwscan.
#[derive(Clone, Debug)]
pub struct ScanLine {
    pub driver: String,
    pub status: String,
}

pub fn parse_hwscan_line(line: &str) -> Option<ScanLine> {
    let line = line.trim();
    if !line.starts_with("drv:") {
        return None;
    }
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }
    Some(ScanLine {
        driver: parts[3].into(),
        status: parts[4].into(),
    })
}

pub fn parse_hwscan_file(path: &Path) -> Vec<ScanLine> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(parse_hwscan_line)
        .collect()
}

/// Calcula perfil a partir de un informe hwscan (solo entradas ausentes o todas).
pub fn profile_from_hwscan(lines: &[ScanLine], only_missing: bool) -> DriverProfile {
    let mut feats: BTreeSet<String> = BTreeSet::new();
    let mut ports: BTreeSet<String> = BTreeSet::new();
    for line in lines {
        if only_missing && line.status != "ausente" {
            continue;
        }
        if let Some((_, port)) = LX_PORT_BY_DRIVER.iter().find(|(n, _)| *n == line.driver.as_str())
        {
            feats.insert("lxdde".into());
            ports.insert((*port).into());
            continue;
        }
        if let Some((feat, _)) = DRIVER_BY_FEATURE
            .iter()
            .find(|(_, name)| *name == line.driver.as_str())
        {
            feats.insert((*feat).into());
        }
    }
    if feats.contains("drv-live-disk") {
        feats.insert("drv-virtio-blk".into());
        feats.insert("drv-nvme".into());
        feats.insert("drv-usb".into());
    }
    let lxdde_mode = if ports.len() == 1 {
        Some(ports.iter().next().unwrap().clone())
    } else {
        None
    };
    DriverProfile {
        kernel_features: feats.into_iter().collect(),
        lxdde_ports: ports.into_iter().collect(),
        lxdde_mode,
        firmware_exclude: vec![],
    }
}

pub fn merge_profiles(base: &DriverProfile, extra: &DriverProfile) -> DriverProfile {
    let mut feats: BTreeSet<_> = base.kernel_features.iter().cloned().collect();
    feats.extend(extra.kernel_features.iter().cloned());
    let mut ports: BTreeSet<_> = base.lxdde_ports.iter().cloned().collect();
    ports.extend(extra.lxdde_ports.iter().cloned());
    DriverProfile {
        kernel_features: feats.into_iter().collect(),
        lxdde_ports: ports.into_iter().collect(),
        lxdde_mode: base.lxdde_mode.clone().or_else(|| extra.lxdde_mode.clone()),
        firmware_exclude: base.firmware_exclude.clone(),
    }
}

pub fn run_fit_drivers(args: &[String]) {
    let root = super::project_root();
    let mut informe: Option<PathBuf> = None;
    let mut esp: Option<PathBuf> = None;
    let mut preset = preset_all();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--esp" => {
                i += 1;
                esp = args.get(i).cloned().map(PathBuf::from);
            }
            "--drivers" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    preset = profile_from_arg(v);
                }
            }
            other if !other.starts_with('-') => {
                informe = Some(PathBuf::from(other));
            }
            _ => {}
        }
        i += 1;
    }

    let scan_path = informe.unwrap_or_else(|| root.join("target/SOSODRV.TXT"));
    let lines = parse_hwscan_file(&scan_path);
    if lines.is_empty() {
        eprintln!(
            "fit-drivers: sin entradas en {} (¿hwscan en la máquina destino?)",
            scan_path.display()
        );
        exit(1);
    }

    let needed = profile_from_hwscan(&lines, true);
    let merged = merge_profiles(&preset, &needed);
    println!(
        "fit-drivers: features={:?} lxdde_ports={:?}",
        merged.kernel_features, merged.lxdde_ports
    );

    let _ = super::build_image_with_profile(&merged);
    super::build_user();
    let _ = super::mkfs_rootfs_with_profile(true, &merged);

    if let Some(esp_path) = esp {
        update_esp_kernel(&esp_path, &root.join("target/soso-uefi.img"));
    }

    println!("fit-drivers: kernel reempaquetado con drivers necesarios");
}

fn update_esp_kernel(esp: &Path, uefi_img: &Path) {
    if !uefi_img.exists() {
        eprintln!("fit-drivers: falta {}", uefi_img.display());
        exit(1);
    }
    let st = Command::new("dd")
        .arg(format!("if={}", uefi_img.display()))
        .arg(format!("of={}", esp.display()))
        .args(["bs=512", "conv=notrunc"])
        .status()
        .expect("dd");
    if !st.success() {
        eprintln!("fit-drivers: fallo dd al actualizar ESP");
        exit(st.code().unwrap_or(1));
    }
    println!("fit-drivers: imagen UEFI escrita en {}", esp.display());
}

pub fn run_driver_add(args: &[String]) {
    let url = args.first().expect("uso: driver-add <git-url> [nombre]");
    let name = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| infer_name_from_url(url));
    let root = super::project_root();
    let dest = root.join("lxdde/ports-extern").join(&name);
    if dest.exists() {
        eprintln!("driver-add: ya existe {}", dest.display());
        exit(1);
    }
    std::fs::create_dir_all(root.join("lxdde/ports-extern")).expect("ports-extern");
    let st = Command::new("git")
        .args(["clone", "--depth", "1", url, dest.to_str().unwrap()])
        .status()
        .expect("git clone");
    if !st.success() {
        exit(st.code().unwrap_or(1));
    }
    register_external_port(&root, &name);
    copy_external_firmware(&root, &dest);
    println!("driver-add: port externo '{name}' registrado en drivers-extern.toml");
}

fn infer_name_from_url(url: &str) -> String {
    url.rsplit('/')
        .next()
        .unwrap_or("extern")
        .trim_end_matches(".git")
        .to_string()
}

fn register_external_port(root: &Path, name: &str) {
    let cfg = root.join("drivers-extern.toml");
    let mut table = if cfg.exists() {
        std::fs::read_to_string(&cfg).unwrap_or_default()
    } else {
        String::from("# Ports de drivers externos (git clone via cargo xtask driver-add)\n\n")
    };
    if !table.contains(&format!("name = \"{name}\"")) {
        table.push_str(&format!(
            "\n[[port]]\nname = \"{name}\"\npath = \"lxdde/ports-extern/{name}\"\n"
        ));
        std::fs::write(&cfg, table).expect("drivers-extern.toml");
    }
}

fn copy_external_firmware(root: &Path, port_dir: &Path) {
    let fw_src = port_dir.join("firmware");
    if !fw_src.is_dir() {
        return;
    }
    let fw_dst = root.join("rootfs/lib/firmware");
    std::fs::create_dir_all(&fw_dst).ok();
    copy_dir_recursive(&fw_src, &fw_dst);
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    if let Ok(entries) = std::fs::read_dir(src) {
        for e in entries.flatten() {
            let ty = e.file_type().ok();
            let to = dst.join(e.file_name());
            if ty.as_ref().map(|t| t.is_dir()).unwrap_or(false) {
                std::fs::create_dir_all(&to).ok();
                copy_dir_recursive(&e.path(), &to);
            } else {
                std::fs::copy(e.path(), &to).ok();
            }
        }
    }
}
