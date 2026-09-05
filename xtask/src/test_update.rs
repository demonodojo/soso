//! `cargo xtask test-update` — actualización local de punta a punta en QEMU/OVMF.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use soso_update_core::hash::hex_sha256;
use soso_update_core::manifest::Manifest;
use soso_update_core::pack::pack_rootfs;
use soso_update_core::semver::parse as parse_semver;

use crate::test::{esperar_en_fichero, ssh_guion};

const SSH_PORT: u16 = 2243;
const MAC: &str = "52:54:00:12:34:43";
const TEST_VER: &str = "0.2.1-prueba";

pub fn run() {
    let root = crate::project_root();
    let dir = root.join("target/test-update");
    std::fs::create_dir_all(&dir).expect("test-update dir");

    preparar_release_prueba(&root);

    unsafe {
        std::env::set_var("SOSO_QEMU_LIVE", "1");
        std::env::set_var("SOSO_QEMU_LIVE_USB", "1");
    }
    crate::package_live::run();
    let live = crate::package_live::live_image_path();

    let Some((ovmf_code, ovmf_vars_src)) = crate::ovmf_paths() else {
        eprintln!("test-update: necesita OVMF (cargo xtask test-install)");
        std::process::exit(1);
    };
    let vars = dir.join("OVMF_VARS.fd");
    std::fs::copy(&ovmf_vars_src, &vars).expect("OVMF_VARS");

    let key = root.join("target/soso_test_key");
    let mut fallos = 0u32;

    let serial1 = dir.join("boot1.log");
    match fase_aplicar(&ovmf_code, &vars, &live, &serial1, &key) {
        Ok(s) => {
            marca("arranque 1: soso-update aplicar --local", true);
            println!("{}", sangrar(&s));
        }
        Err(e) => {
            marca(&format!("arranque 1: aplicar — {e}"), false);
            fallos += 1;
        }
    }

    let serial2 = dir.join("boot2.log");
    match fase_comprobar_version(&ovmf_code, &vars, &live, &serial2, &key, TEST_VER) {
        Ok(()) => marca(&format!("arranque 2: versión {TEST_VER} visible"), true),
        Err(e) => {
            marca(&format!("arranque 2: versión — {e}"), false);
            fallos += 1;
        }
    }

    if fallos > 0 {
        eprintln!("\ntest-update: {fallos} fallo(s)");
        std::process::exit(1);
    }
    println!("\ntest-update: actualización local OK");
}

fn preparar_release_prueba(root: &Path) {
    let rel_dir = root.join("rootfs/var/actualiza-prueba");
    std::fs::create_dir_all(&rel_dir).expect("actualiza-prueba");

    crate::build_user();

    let kernel_src = root.join("target/kernel/x86_64-soso/debug/kernel");
    if !kernel_src.exists() {
        let profile = crate::drivers::preset_live_usb();
        let _ = crate::build_image_with_profile(&profile, true);
    }
    let kernel_bytes = std::fs::read(&kernel_src).expect("kernel");

    // El pack lleva la versión nueva; el live se empaqueta con la versión actual (VERSION).
    let etc = root.join("rootfs/etc");
    std::fs::write(
        etc.join("soso-release"),
        format!("version={TEST_VER}\nbuild=prueba\nfecha=2026-09-04\n"),
    )
    .expect("soso-release prueba");
    let (pack_blob, files) = pack_rootfs(&root.join("rootfs")).expect("pack");
    crate::version::write_soso_release(root);

    std::fs::write(rel_dir.join("kernel-x86_64"), &kernel_bytes).expect("kernel copy");
    std::fs::write(rel_dir.join("rootfs.pack"), &pack_blob).expect("pack");

    let manifest = Manifest {
        version: parse_semver(TEST_VER).unwrap(),
        version_raw: TEST_VER.into(),
        build: "prueba".into(),
        fecha: "2026-09-04".into(),
        kernel_hash: hex_sha256(&kernel_bytes),
        kernel_size: kernel_bytes.len() as u64,
        pack_hash: hex_sha256(&pack_blob),
        pack_size: pack_blob.len() as u64,
        files,
    };
    std::fs::write(rel_dir.join("manifest.txt"), manifest.format()).expect("manifest");
    println!("test-update: release de prueba en rootfs/var/actualiza-prueba/");
}

fn fase_aplicar(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    key: &Path,
) -> Result<String, String> {
    let _ = std::fs::remove_file(serial);
    let qemu = lanzar_live(code, vars, live, serial)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(300))?;
    let salida = ssh_guion(
        key,
        SSH_PORT,
        "soso-update aplicar --local /var/actualiza-prueba --forzar\nhalt\n",
        Duration::from_secs(600),
    )?;
    if !salida.contains("listo") {
        return Err(format!("aplicar no terminó bien: {salida:?}"));
    }
    Ok(salida)
}

fn fase_comprobar_version(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    key: &Path,
    ver: &str,
) -> Result<(), String> {
    let _ = std::fs::remove_file(serial);
    let qemu = lanzar_live(code, vars, live, serial)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(300))?;
    let salida = ssh_guion(
        key,
        SSH_PORT,
        "soso-update estado\nexit\n",
        Duration::from_secs(120),
    )?;
    if !salida.contains(ver) {
        return Err(format!("estado no muestra {ver}: {salida:?}"));
    }
    Ok(())
}

struct QemuProc {
    child: std::process::Child,
}

struct Matar(std::process::Child);

impl Drop for Matar {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn lanzar_live(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
) -> Result<QemuProc, String> {
    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args(["-machine", "q35", "-cpu", "max"])
        .args(["-m", "2048M"])
        .args(["-smp", "2"])
        .arg("-no-reboot")
        .args(["-display", "none"])
        .stdout(Stdio::null());
    cmd.args([
        "-drive",
        &format!("if=pflash,format=raw,readonly=on,file={}", code.display()),
    ]);
    cmd.args([
        "-drive",
        &format!("if=pflash,format=raw,file={}", vars.display()),
    ]);
    cmd.args(["-device", "qemu-xhci,id=xhci"]);
    cmd.args([
        "-drive",
        &format!("file={},format=raw,if=none,id=live0", live.display()),
    ]);
    cmd.args(["-device", "usb-storage,bus=xhci.0,port=1,drive=live0"]);
    crate::apply_qemu_nic_with_ports(&mut cmd, SSH_PORT, SSH_PORT + 1, Some(&MAC.to_string()));
    cmd.args(["-serial", &format!("file:{}", serial.display())])
        .stderr(Stdio::null());
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    Ok(QemuProc { child })
}

fn marca(msg: &str, ok: bool) {
    let tag = if ok { "OK" } else { "FALLO" };
    println!("test-update: [{tag}] {msg}");
}

fn sangrar(s: &str) -> String {
    s.lines()
        .map(|l| format!("      {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
