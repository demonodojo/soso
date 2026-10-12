//! Metadatos de versión leídos de `VERSION` y git.

use std::path::Path;
use std::process::Command;

pub fn read_version(root: &Path) -> String {
    std::fs::read_to_string(root.join("VERSION"))
        .unwrap_or_else(|_| "0.0.0".into())
        .trim()
        .to_string()
}

pub fn git_build(root: &Path) -> String {
    Command::new("git")
        .args(["-C", root.to_str().unwrap(), "describe", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub fn build_date() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub fn write_soso_release(root: &Path) {
    let etc = root.join("rootfs/etc");
    std::fs::create_dir_all(&etc).expect("rootfs/etc");
    let body = format!(
        "version={}\nbuild={}\nfecha={}\n",
        read_version(root),
        git_build(root),
        build_date(),
    );
    std::fs::write(etc.join("soso-release"), body).expect("soso-release");
}
