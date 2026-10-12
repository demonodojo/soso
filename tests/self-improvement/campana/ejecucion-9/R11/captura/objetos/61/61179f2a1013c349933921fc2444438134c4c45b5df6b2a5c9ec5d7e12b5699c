//! Compilar como el usuario de sudo, no como root.
//!
//! `flash-usb-live` / `install-disk` se lanzan **sin** `sudo cargo`: rustup
//! vive en `~/.cargo`. xtask pide sudo solo para el ELF propio al grabar el
//! disco. Si aun así el proceso es root (`sudo cargo` de un hábito viejo),
//! `HOME`/`PATH` salen de `SUDO_UID` (`getent` / `/etc/passwd`), no de `/root`.
//!
//! clang y cargo no deben escribir `target/` como uid 0: el siguiente build
//! sin sudo no podría sobrescribir. Se reclaman los árboles de compilación y
//! los hijos `cargo`/`clang`/`ar` arrancan con ruid=euid=`SUDO_UID`.
//!
//! No se hace `seteuid` en el proceso xtask: con ruid 0 y euid de usuario el
//! dynamic linker activa AT_SECURE y rustc no carga `librustc_driver` desde
//! `$ORIGIN` (`~/.rustup/...`). El padre se queda root para el `dd`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, exit};

/// El proceso elevado (solo escritura al disco) no vuelve a compilar.
pub(crate) const DEVICE_IO_ENV: &str = "SOSO_XTASK_DEVICE_IO";

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
            env::var("SUDO_UID").ok().as_deref(),
            env::var("SUDO_GID").ok().as_deref(),
        )
    }
    #[cfg(not(unix))]
    {
        None
    }
}

pub(crate) fn is_root() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc_geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

pub(crate) fn device_io_phase() -> bool {
    matches!(
        env::var(DEVICE_IO_ENV).as_deref(),
        Ok("1") | Ok("true")
    )
}

fn parse_passwd_home(text: &str, uid: u32) -> Option<PathBuf> {
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut fields = line.split(':');
        let _name = fields.next()?;
        let _pw = fields.next()?;
        let u: u32 = fields.next()?.parse().ok()?;
        if u != uid {
            continue;
        }
        let _gid = fields.next()?;
        let _gecos = fields.next()?;
        let home = fields.next()?;
        if home.is_empty() {
            return None;
        }
        return Some(PathBuf::from(home));
    }
    None
}

fn home_for_uid(uid: u32) -> Option<PathBuf> {
    if let Ok(out) = Command::new("getent")
        .args(["passwd", &uid.to_string()])
        .output()
        && out.status.success()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(home) = parse_passwd_home(&text, uid) {
            return Some(home);
        }
    }
    let text = fs::read_to_string("/etc/passwd").ok()?;
    parse_passwd_home(&text, uid)
}

fn path_with_cargo_bin(path: &str, cargo_bin: &str) -> String {
    if cargo_bin.is_empty() || path.split(':').any(|p| p == cargo_bin) {
        path.to_string()
    } else if path.is_empty() {
        cargo_bin.to_string()
    } else {
        format!("{cargo_bin}:{path}")
    }
}

fn with_yes_flag(mut args: Vec<String>) -> Vec<String> {
    if !args.iter().any(|a| a == "--yes" || a == "-y") {
        args.push("--yes".into());
    }
    args
}

/// `HOME`/`PATH`/`CARGO_HOME`/`RUSTUP_HOME` del invocador de sudo.
/// No-op si no somos root vía sudo.
pub(crate) fn restore_invoking_env() {
    let Some((uid, _)) = invoking_ids() else {
        return;
    };
    let Some(home) = home_for_uid(uid) else {
        return;
    };
    let home_wrong = env::var_os("HOME")
        .map(|h| h == "/root" || PathBuf::from(&h) != home)
        .unwrap_or(true);
    if home_wrong {
        unsafe { env::set_var("HOME", &home) };
    }
    let cargo_bin = home.join(".cargo/bin");
    if cargo_bin.join("cargo").is_file() {
        let path = env::var("PATH").unwrap_or_default();
        let cargo_bin = cargo_bin.to_string_lossy();
        unsafe { env::set_var("PATH", path_with_cargo_bin(&path, cargo_bin.as_ref())) };
    }
    if env::var_os("CARGO_HOME").is_none() {
        unsafe { env::set_var("CARGO_HOME", home.join(".cargo")) };
    }
    if env::var_os("RUSTUP_HOME").is_none() {
        unsafe { env::set_var("RUSTUP_HOME", home.join(".rustup")) };
    }
}

/// Relanza este ELF con sudo (no `cargo`). Pide contraseña en TTY. No retorna.
pub(crate) fn exec_elevated() -> ! {
    let exe = env::current_exe().unwrap_or_else(|e| {
        eprintln!("xtask: no pude resolver el binario: {e}");
        exit(1);
    });
    let mut cmd = Command::new("sudo");
    cmd.arg("env");
    cmd.arg(format!("{DEVICE_IO_ENV}=1"));
    for (k, v) in env::vars() {
        if k.starts_with("SOSO_") && k != DEVICE_IO_ENV {
            cmd.arg(format!("{k}={v}"));
        }
    }
    cmd.arg(&exe);
    cmd.args(with_yes_flag(env::args().skip(1).collect()));
    eprintln!(
        "xtask: sudo {} (solo este binario; rustup no hace falta)",
        exe.display()
    );
    let st = cmd.status().unwrap_or_else(|e| {
        eprintln!("xtask: sudo falló: {e}");
        exit(1);
    });
    exit(st.code().unwrap_or(1));
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
    use super::{parse_passwd_home, parse_sudo_ids, path_with_cargo_bin, with_yes_flag};
    use std::path::PathBuf;

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

    #[test]
    fn parse_passwd_home_reads_uid() {
        let text = "root:x:0:0:root:/root:/bin/bash\n\
jmdiez:x:1000:1000:José:/home/jmdiez:/bin/bash\n";
        assert_eq!(
            parse_passwd_home(text, 1000),
            Some(PathBuf::from("/home/jmdiez"))
        );
        assert_eq!(parse_passwd_home(text, 0), Some(PathBuf::from("/root")));
        assert_eq!(parse_passwd_home(text, 42), None);
    }

    #[test]
    fn path_with_cargo_bin_prepends_once() {
        let bin = "/home/jmdiez/.cargo/bin";
        assert_eq!(path_with_cargo_bin("/usr/bin", bin), format!("{bin}:/usr/bin"));
        assert_eq!(
            path_with_cargo_bin(&format!("{bin}:/usr/bin"), bin),
            format!("{bin}:/usr/bin")
        );
        assert_eq!(path_with_cargo_bin("", bin), bin);
    }

    #[test]
    fn with_yes_flag_is_idempotent() {
        let args = vec!["flash-usb-live".into(), "/dev/sda".into()];
        assert_eq!(with_yes_flag(args.clone()), {
            let mut v = args;
            v.push("--yes".into());
            v
        });
        let already = vec!["flash-usb-live".into(), "--yes".into()];
        assert_eq!(with_yes_flag(already.clone()), already);
    }
}
