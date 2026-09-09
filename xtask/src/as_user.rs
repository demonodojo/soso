//! Compilar como el usuario de sudo, no como root.
//!
//! `sudo cargo xtask flash-usb-live` necesita root para el `dd`, pero clang y
//! cargo no: si escriben `target/` como uid 0, el siguiente build sin sudo no
//! puede sobrescribir. Se reclaman los árboles de compilación y los hijos
//! `cargo`/`clang`/`ar` arrancan con ruid=euid=`SUDO_UID`.
//!
//! No se hace `seteuid` en el proceso xtask: con ruid 0 y euid de usuario el
//! dynamic linker activa AT_SECURE y rustc no carga `librustc_driver` desde
//! `$ORIGIN` (`~/.rustup/...`). El padre se queda root para el `dd`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Si este proceso es root *porque* lo lanzó sudo, uid/gid del invocador.
/// Sin SUDO_UID (root de verdad) no se toca nada.
fn parse_sudo_ids(euid: u32, sudo_uid: Option<&str>, sudo_gid: Option<&str>) -> Option<(u32, u32)> {
    if euid != 0 {
        return None;
    }
    let uid: u32 = sudo_uid?.parse().ok()?;
    if uid == 0 {
        return None;
    }
    let gid: u32 = sudo_gid.and_then(|s| s.parse().ok()).unwrap_or(uid);
    Some((uid, gid))
}

fn invoking_ids() -> Option<(u32, u32)> {
    #[cfg(unix)]
    {
        parse_sudo_ids(
            unsafe { libc_geteuid() },
            std::env::var("SUDO_UID").ok().as_deref(),
            std::env::var("SUDO_GID").ok().as_deref(),
        )
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Lanza el hijo como el invocador de sudo (`ruid == euid`). No-op sin sudo.
pub(crate) fn apply_invoking_user(cmd: &mut Command) {
    #[cfg(unix)]
    if let Some((uid, gid)) = invoking_ids() {
        use std::os::unix::process::CommandExt;
        cmd.uid(uid).gid(gid);
    }
}

pub(crate) struct AsInvokingUser {
    spec: Option<String>,
    files: Vec<PathBuf>,
    recursive: Vec<PathBuf>,
}

/// Reclama `target/{kernel,user,lxdde,…}` y las imágenes de disco. No-op sin
/// sudo. No hace `chown -R` de `target/` entero: ahí viven los modelos
/// (`*-model/`) y no hay que recorrerlos. Al soltar el guard se vuelve a
/// `chown` lo que el padre haya escrito como root (imágenes, stubs).
pub(crate) fn as_invoking_user_for_build(root: &Path) -> AsInvokingUser {
    let target = root.join("target");
    let _ = fs::create_dir_all(&target);
    let mut recursive = Vec::new();
    for name in [
        "lxdde",
        "kernel",
        "user",
        "boot-shim",
        "usb-live",
        "usb-package",
    ] {
        recursive.push(target.join(name));
    }
    let mut files = vec![
        target.clone(),
        root.join("lxdde/shim/src/generated_dummies.c"),
    ];
    for name in [
        "soso-bios.img",
        "soso-uefi.img",
        "soso-data.img",
        "soso-models.img",
        "soso-models-live.img",
        "OVMF_VARS.fd",
    ] {
        files.push(target.join(name));
    }
    reclaim_build_tree(files, recursive)
}

fn reclaim_build_tree(files: Vec<PathBuf>, recursive: Vec<PathBuf>) -> AsInvokingUser {
    let Some((uid, gid)) = invoking_ids() else {
        return AsInvokingUser {
            spec: None,
            files,
            recursive,
        };
    };
    let spec = format!("{uid}:{gid}");
    for p in &files {
        chown_to(&spec, p, false);
    }
    for p in &recursive {
        chown_to(&spec, p, true);
    }
    AsInvokingUser {
        spec: Some(spec),
        files,
        recursive,
    }
}

fn chown_to(spec: &str, path: &Path, recursive: bool) {
    if !path.exists() {
        return;
    }
    let mut cmd = Command::new("chown");
    if recursive {
        cmd.arg("-R");
    }
    let _ = cmd.arg(spec).arg(path).status();
}

impl Drop for AsInvokingUser {
    fn drop(&mut self) {
        let Some(spec) = &self.spec else {
            return;
        };
        for p in &self.files {
            chown_to(spec, p, false);
        }
        for p in &self.recursive {
            chown_to(spec, p, true);
        }
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn geteuid() -> u32;
}

#[cfg(unix)]
unsafe fn libc_geteuid() -> u32 {
    unsafe { geteuid() }
}

#[cfg(test)]
mod tests {
    use super::parse_sudo_ids;

    #[test]
    fn parse_sudo_ids_ignores_non_root() {
        assert_eq!(parse_sudo_ids(1000, Some("1000"), Some("1000")), None);
    }

    #[test]
    fn parse_sudo_ids_requires_sudo_uid() {
        assert_eq!(parse_sudo_ids(0, None, Some("1000")), None);
        assert_eq!(parse_sudo_ids(0, Some("0"), Some("0")), None);
        assert_eq!(
            parse_sudo_ids(0, Some("1000"), Some("1000")),
            Some((1000, 1000))
        );
        assert_eq!(parse_sudo_ids(0, Some("1000"), None), Some((1000, 1000)));
    }
}
