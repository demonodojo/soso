//! `cargo xtask test-update` — actualización local de punta a punta en QEMU/OVMF.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use soso_update_core::backup_digest;
use soso_update_core::hash::hex_sha256;
use soso_update_core::kernel_meta::{KernelMeta, KernelPhase, KERNEL_META_SIZE};
use soso_update_core::manifest::Manifest;
use soso_update_core::mailbox::Mailbox;
use soso_update_core::pack::pack_rootfs_con;
use soso_update_core::semver::parse as parse_semver;
use soso_update_core::UPD_KERNEL_SLOT_SIZE;

use crate::test::{esperar_en_fichero, ssh_guion};

const SSH_PORT: u16 = 2243;
const MAC: &str = "52:54:00:12:34:43";
const TEST_VER: &str = "0.2.1-prueba";
const MARCA_NUEVA: &str = "nueva\n";
const MARCA_VIEJA: &str = "vieja\n";

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
    let live_base = dir.join("live-base.img");
    crate::copy_sparse(&live, &live_base);

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

    let serial3 = dir.join("boot-recovery.log");
    match fase_recuperacion_corte(&ovmf_code, &vars, &live_base, &serial3, &key) {
        Ok(()) => marca("arranque 3: recuperación tras corte (meta applying)", true),
        Err(e) => {
            marca(&format!("arranque 3: recuperación — {e}"), false);
            fallos += 1;
        }
    }

    let serial4 = dir.join("boot-manifest.log");
    match fase_manifiesto_invalido(&ovmf_code, &vars, &live, &serial4, &key) {
        Ok(()) => marca("arranque 4: manifiesto inválido rechazado", true),
        Err(e) => {
            marca(&format!("arranque 4: manifiesto — {e}"), false);
            fallos += 1;
        }
    }

    if fallos > 0 {
        eprintln!("\ntest-update: {fallos} fallo(s)");
        std::process::exit(1);
    }
    println!("\ntest-update: actualización local OK (+ recuperación OTA)");
}

fn preparar_release_prueba(root: &Path) {
    let rel_dir = root.join("rootfs/var/actualiza-prueba");
    // De una pasada anterior pueden quedar ~160 MB aquí dentro; el rootfs se
    // empaqueta entero en la imagen y no cabría.
    let _ = std::fs::remove_dir_all(&rel_dir);
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
    // Un fichero que de verdad cambie: el pack lleva "nueva" y el live que se
    // instala lleva "vieja", así que `aplicar` tiene que bajar ese tramo y sólo
    // ese. Sin esto el pack sería idéntico al rootfs instalado y la descarga
    // parcial no se ejercitaría.
    let marca_path = etc.join("actualiza-marca.txt");
    std::fs::write(&marca_path, MARCA_NUEVA).expect("marca nueva");
    // Sin el firmware de la GPU: son 127 MB que el test no necesita y que, al
    // vivir el release dentro del propio rootfs, duplicarían la imagen.
    let (pack_blob, files) = pack_rootfs_con(&root.join("rootfs"), |rel| {
        !rel.starts_with("lib/firmware/")
    })
    .expect("pack");
    std::fs::write(&marca_path, MARCA_VIEJA).expect("marca vieja");
    crate::version::write_soso_release(root);

    // Igual que `cargo xtask release`: el kernel que se publica va sin símbolos
    // de depuración. Así la fase 2 arranca exactamente el ELF que recibiría una
    // placa real, y no un binario distinto al del release.
    let kernel_pub = rel_dir.join("kernel-x86_64");
    std::fs::write(&kernel_pub, &kernel_bytes).expect("kernel copy");
    crate::release::strip_kernel(&kernel_pub);
    let kernel_bytes = std::fs::read(&kernel_pub).expect("kernel stripped");
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
    if !salida.contains("etc/actualiza-marca.txt") {
        return Err(format!("no actualizó el fichero cambiado: {salida:?}"));
    }
    // La gracia de la actualización parcial: se piden unos pocos ficheros, no
    // los 60 y pico del pack.
    if let Some(l) = salida.lines().find(|l| l.trim_start().starts_with("rootfs:"))
        && let Some(n) = l.split_whitespace().nth(1).and_then(|n| n.parse::<usize>().ok())
        && n > 5
    {
        return Err(format!("descargó {n} ficheros, esperaba unos pocos: {l}"));
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
        // El `cat` va primero: la última línea del guion se pierde a veces al
        // cerrar la sesión SSH, y la salida de `estado` sirve de barrera.
        // Termina en `halt`, no en `exit`: `exit` cierra sosh pero deja la
        // sesión SSH abierta y el guion se come el timeout entero.
        "cat /etc/actualiza-marca.txt\nsoso-update estado\nhalt\n",
        Duration::from_secs(120),
    )?;
    // Ojo: `estado` también imprime el fichero de progreso "APLICANDO <ver>"
    // que queda si `aplicar` se cortó a medias, así que exigimos la línea del
    // rootfs; si no, un fallo a mitad se colaría como éxito.
    if !salida.contains(&format!("rootfs: {ver}")) {
        return Err(format!("estado no muestra rootfs {ver}: {salida:?}"));
    }
    if !salida.contains(MARCA_NUEVA.trim()) {
        return Err(format!("el fichero actualizado no sobrevivió al reinicio: {salida:?}"));
    }
    // El USB de este test es un `usb-storage` real de QEMU (no virtio): el
    // disco de arranque debe reconocerse como tal por DISK_KIND_USB.
    if !salida.contains("arranque: USB live") {
        return Err(format!("estado no reconoce el medio de arranque USB: {salida:?}"));
    }
    Ok(())
}

/// Simula un corte tras escribir el backup: meta `applying`, kernel activo corrupto.
fn fase_recuperacion_corte(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    key: &Path,
) -> Result<(), String> {
    let ver_base = crate::version::read_version(&crate::project_root());
    inyectar_corte_backup(live)?;

    let _ = std::fs::remove_file(serial);
    let qemu = lanzar_live(code, vars, live, serial)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(300))?;

    let log = std::fs::read_to_string(serial).unwrap_or_default();
    let p1 = esp_p1(live);
    let mark = crate::fat32_write::read_root_file(live, p1, b"BOOTMARKTXT")
        .unwrap_or_default();
    let mark_s = String::from_utf8_lossy(&mark);
    let ok_serial = log.contains("recuperado tras corte") || log.contains("actualiza: recuperado");
    let ok_mark = mark_s.contains("recuperado tras corte");
    if !ok_serial && !ok_mark {
        return Err(format!(
            "sin recuperación en serial ni BOOTMARK; mark={mark_s:?} serial={:?}",
            log.lines()
                .filter(|l| l.contains("actualiza") || l.contains("soso-shim"))
                .take(8)
                .collect::<Vec<_>>()
        ));
    }

    let salida = ssh_guion(
        key,
        SSH_PORT,
        &format!("soso-update estado\nhalt\n"),
        Duration::from_secs(120),
    )?;
    if !salida.contains(&format!("rootfs: {ver_base}")) {
        return Err(format!(
            "tras recuperar, rootfs debería seguir en {ver_base}: {salida:?}"
        ));
    }
    Ok(())
}

/// Manifiesto con path `..` debe fallar antes de mutar el rootfs.
fn fase_manifiesto_invalido(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    key: &Path,
) -> Result<(), String> {
    let _ = std::fs::remove_file(serial);
    let qemu = lanzar_live(code, vars, live, serial)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(300))?;

    let bad_manifest = format!(
        "{}\nversion=9.9.9\nbuild=x\nfecha=2026-01-01\n\
         kernel {} 10\npack {} 20\nf {} 0 5 ../etc/passwd\n",
        soso_update_core::manifest::MANIFEST_MAGIC,
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
    );
    let guion = format!(
        "cat > /var/actualiza-prueba/manifest.txt <<'EOF'\n{bad_manifest}EOF\n\
         soso-update aplicar --local /var/actualiza-prueba --forzar\n\
         cat /etc/actualiza-marca.txt\nhalt\n"
    );
    let salida = ssh_guion(key, SSH_PORT, &guion, Duration::from_secs(180))?;
    if !salida.contains("manifest") {
        return Err(format!("aplicar no rechazó el manifiesto: {salida:?}"));
    }
    if !salida.contains(MARCA_NUEVA.trim()) {
        return Err(format!(
            "el manifiesto inválido mutó el rootfs (marca cambió): {salida:?}"
        ));
    }
    Ok(())
}

fn inyectar_corte_backup(live: &Path) -> Result<(), String> {
    let p1 = esp_p1(live);
    let ((kname8, kext3), kernel) = esp_read_kernel(live, p1)?;
    let (backup_size, backup_hash) = backup_digest(&kernel);

    let mut slot = kernel.clone();
    slot.resize(UPD_KERNEL_SLOT_SIZE, 0);
    esp_write_root(live, p1, b"SOSOKRN ", b"BIN", &slot)?;

    let meta = KernelMeta {
        phase: KernelPhase::Applying,
        version: String::from(TEST_VER),
        new_size: 0,
        new_hash: String::new(),
        backup_size,
        backup_hash,
    };
    let mut meta_bytes = meta.format();
    meta_bytes.resize(KERNEL_META_SIZE, b'\n');
    esp_write_root(live, p1, b"SOSOKRN ", b"MET", &meta_bytes)?;

    let mut corrupt = kernel.clone();
    let n = corrupt.len().min(4096);
    corrupt[..n].fill(0xA5);
    esp_write_root(live, p1, &kname8, &kext3, &corrupt)?;

    let idle = Mailbox::format_idle();
    esp_write_root(live, p1, b"SOSOUPD ", b"TXT", &idle)?;
    Ok(())
}

fn esp_p1(live: &Path) -> u64 {
    crate::package_live::partition_first_sector(live, 1)
        .expect("ESP p1 del live")
}

fn esp_write_root(live: &Path, p1: u64, name: &[u8; 8], ext: &[u8; 3], data: &[u8]) -> Result<(), String> {
    crate::fat32_write::write_root_file(live, p1, name, ext, data)
}

fn esp_read_kernel(live: &Path, p1: u64) -> Result<(([u8; 8], [u8; 3]), Vec<u8>), String> {
    let files = crate::fat32_write::list_root_files(live, p1)?;
    let mut best: Option<([u8; 8], [u8; 3], u32)> = None;
    for (name11, size) in files {
        let label = String::from_utf8_lossy(&name11);
        if label.starts_with("SOSO") || label.starts_with("BOOTMARK") {
            continue;
        }
        if size < 1024 * 1024 {
            continue;
        }
        let mut n = [0u8; 8];
        let mut e = [0u8; 3];
        n.copy_from_slice(&name11[..8]);
        e.copy_from_slice(&name11[8..11]);
        if best.as_ref().map(|b| size > b.2).unwrap_or(true) {
            best = Some((n, e, size));
        }
    }
    let (n, e, _) = best.ok_or_else(|| "no encontré kernel-x86_64 en la ESP".to_string())?;
    let mut name11 = [0u8; 11];
    name11[..8].copy_from_slice(&n);
    name11[8..11].copy_from_slice(&e);
    let data = crate::fat32_write::read_root_file(live, p1, &name11)?;
    Ok(((n, e), data))
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
