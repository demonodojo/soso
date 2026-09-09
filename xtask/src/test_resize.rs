//! `cargo xtask test-resize` — redimensionado GPT recuperable (host + QEMU live).
//!
//! Grow QEMU: modelo sintético + 256 MiB (slide corto). Hace falta
//! `drv-live-disk` (el preset `qemu` no lo lleva).

use std::cell::Cell;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use soso_resize_core::{
    self, Disk512, Error as ResizeError, Journal, PartGeom, read_part, SECTOR,
};

use crate::test::{esperar_en_fichero, ssh_guion};

const SSH_PORT: u16 = 2244;
const MAC: &str = "52:54:00:12:34:44";
/// 64 sectores × 512 B = 32 KiB desplazados = 8 bloques sosofs de 4 KiB.
const DELTA_SECTORS: u64 = 64;

pub fn run() {
    let root = crate::project_root();
    let dir = root.join("target/test-resize");
    std::fs::create_dir_all(&dir).expect("test-resize dir");

    let mut fallos = 0u32;

    if !host_tests(&root) {
        marca("tests host soso-resize-core", false);
        fallos += 1;
    } else {
        marca("tests host soso-resize-core", true);
    }

    unsafe {
        std::env::set_var("SOSO_QEMU_LIVE", "1");
        // Preset `qemu` no incluye live-disk → ENOSYS en soso-resize.
        std::env::set_var(
            "SOSO_DRIVERS",
            "drv-virtio-blk,drv-virtio-net,drv-live-disk,drv-hda",
        );
        std::env::set_var("SOSO_LIVE_OFFLINE", "1");
        if std::env::var_os("SOSO_MODELS_DIR").is_none() {
            std::env::set_var("SOSO_MODELS_DIR", root.join("target/tiny-model"));
        }
        if std::env::var_os("SOSO_MODELS_SIZE").is_none() {
            std::env::set_var("SOSO_MODELS_SIZE", "256M");
        }
    }
    if std::env::var("SOSO_TEST_RESIZE_SKIP_PACKAGE").is_ok_and(|v| v == "1") {
        println!("test-resize: omitiendo package-usb-live (SOSO_TEST_RESIZE_SKIP_PACKAGE=1)");
    } else {
        crate::package_live::run();
    }
    let live = crate::package_live::live_image_path();

    let Some((ovmf_code, ovmf_vars_src)) = crate::ovmf_paths() else {
        eprintln!("test-resize: necesita OVMF (ver cargo xtask test-install)");
        std::process::exit(1);
    };
    let vars = dir.join("OVMF_VARS.fd");
    let key = root.join("target/soso_test_key");

    let serial_grow = dir.join("grow.log");
    reset_ovmf_vars(&vars, &ovmf_vars_src);
    match fase_grow_root(&ovmf_code, &vars, &live, &serial_grow, &key) {
        Ok(()) => marca("QEMU: soso-resize rootfs +32K", true),
        Err(e) => {
            marca(&format!("QEMU: grow — {e}"), false);
            eprintln!("      log serie: {}", serial_grow.display());
            fallos += 1;
        }
    }

    let recovery_img = dir.join("recovery.img");
    crate::copy_sparse(&live, &recovery_img);
    if let Err(e) = borrar_journal(&recovery_img) {
        marca(&format!("host: borrar journal recovery — {e}"), false);
        fallos += 1;
    } else {
    match inyectar_corte_slide(&recovery_img) {
        Ok(()) => marca("host: journal en fase SHRINK_MODELS (corte slide)", true),
        Err(e) => {
            marca(&format!("host: inyección corte — {e}"), false);
            fallos += 1;
        }
    }
    }

    let serial_rec = dir.join("recovery.log");
    reset_ovmf_vars(&vars, &ovmf_vars_src);
    match fase_recovery_arranque(&ovmf_code, &vars, &recovery_img, &serial_rec, &key) {
        Ok(()) => marca("QEMU: recovery tras corte slide+GPT", true),
        Err(e) => {
            marca(&format!("QEMU: recovery — {e}"), false);
            eprintln!("      log serie: {}", serial_rec.display());
            fallos += 1;
        }
    }

    if fallos > 0 {
        eprintln!("\ntest-resize: {fallos} fallo(s)");
        std::process::exit(1);
    }
    println!("\ntest-resize: redimensionado recuperable OK (host + QEMU)");
}

fn ensure_grow_img(live: &Path, grow_img: &Path) -> Result<(), String> {
    let live_mtime = std::fs::metadata(live)
        .and_then(|m| m.modified())
        .map_err(|e| e.to_string())?;
    let recopy = match std::fs::metadata(grow_img).and_then(|m| m.modified()) {
        Ok(grow_mtime) => grow_mtime < live_mtime,
        Err(_) => true,
    };
    if recopy {
        println!("test-resize: copiando imagen live → grow.img…");
        crate::copy_sparse(live, grow_img);
    }
    borrar_journal(grow_img)
}

fn reset_ovmf_vars(dst: &Path, src: &Path) {
    std::fs::copy(src, dst).expect("OVMF_VARS");
}

fn host_tests(root: &Path) -> bool {
    Command::new("cargo")
        .current_dir(root)
        .args(["test", "-p", "soso-resize-core", "-q", "--test", "cuts"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn ssh_resize(guion: &str, limite: Duration, key: &Path) -> Result<String, String> {
    let mut ultimo = String::new();
    for intento in 0..3 {
        match ssh_guion(key, SSH_PORT, guion, limite) {
            Ok(s) => return Ok(s),
            Err(e) if e.contains("ampliable rootfs") || e.contains("hecho") || e.contains("rootfs +") => {
                return Ok(e);
            }
            Err(e) => {
                ultimo = e.clone();
                if intento < 2 && (e.contains("prompt") || e.contains("ConnectTimeout")) {
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
                return Err(e);
            }
        }
    }
    Err(ultimo)
}

fn esperar_red(serial: &Path) -> Result<(), String> {
    esperar_en_fichero(serial, "net: dhcp 10.", Duration::from_secs(180))
}

fn fase_grow_root(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    key: &Path,
) -> Result<(), String> {
    let _ = std::fs::remove_file(serial);
    let grow_img = serial
        .parent()
        .ok_or("serial sin directorio")?
        .join("grow.img");
    ensure_grow_img(live, &grow_img)?;

    let qemu = lanzar_live(code, vars, &grow_img, serial)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(600))?;
    esperar_red(serial)?;

    let grow = ssh_resize(
        "soso-resize rootfs +32K\nexit\n",
        Duration::from_secs(1200),
        key,
    )?;
    let log = std::fs::read_to_string(serial).unwrap_or_default();
    if grow.contains("error") && !grow.contains("hecho") {
        return Err(format!("grow falló: {grow:?}"));
    }
    if !grow.contains("hecho") && !log.contains("fs-resize: rootfs ahora") {
        return Err(format!("grow no completó: ssh={grow:?}"));
    }
    if !log.contains("fs-resize:") {
        return Err("sin traza fs-resize en serie".into());
    }
    Ok(())
}

fn fase_recovery_arranque(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    key: &Path,
) -> Result<(), String> {
    let _ = std::fs::remove_file(serial);
    let qemu = lanzar_live(code, vars, live, serial)?;
    let _guard = Matar(qemu.child);

    let ok_sosh = esperar_en_fichero(serial, "sosh —", Duration::from_secs(600)).is_ok();
    let log = std::fs::read_to_string(serial).unwrap_or_default();
    if !log.contains("fs-resize: recuperando") {
        return Err(format!(
            "sin recovery en serie: {:?}",
            log.lines()
                .filter(|l| l.contains("fs-resize") || l.contains("fs: live"))
                .take(12)
                .collect::<Vec<_>>()
        ));
    }
    if !log.contains("disco recuperado") {
        return Err("recovery no completó slide/GPT".into());
    }
    if log.contains("grow root en finalize fall") {
        return Err("recovery no amplió rootfs en finalize".into());
    }
    if log.contains("journal corrupto") || log.contains("live abortado") {
        return Err("recovery abortó el arranque".into());
    }

    if ok_sosh {
        esperar_red(serial).ok();
        let salida = ssh_resize("echo recovery-ok\nexit\n", Duration::from_secs(120), key)?;
        if !salida.contains("recovery-ok") {
            return Err(format!("ssh tras recovery falló: {salida:?}"));
        }
    }
    Ok(())
}

fn inyectar_corte_slide(img: &Path) -> Result<(), String> {
    let delta = DELTA_SECTORS;
    validate_delta_fits(img, delta)?;

    // Corte tras shrink, antes/durante el slide: es el caso que hay que
    // reanudar sin repetir una copia solapada. INTENT (shrink al arrancar)
    // se cubre en host; en QEMU colgaba el montaje de modelos antes de FS.
    let journal = Journal {
        phase: soso_resize_core::JOURNAL_SHRINK_MODELS,
        delta_sectors: delta,
        slide_done: 0,
    };
    write_journal(img, &journal)?;
    Ok(())
}

fn validate_delta_fits(img: &Path, delta: u64) -> Result<(), String> {
    let p3 = read_part_on(img, soso_resize_core::GPT_MODELS)?;
    if p3.sectors <= delta {
        return Err(format!(
            "partición modelos demasiado pequeña ({}) para delta {delta}",
            p3.sectors
        ));
    }
    let disk = ImgDisk::open(img)?;
    soso_resize_core::validate_geometry(&disk, delta)
        .map_err(|e| format!("validate_geometry: {e:?}"))
}

fn read_part_on(img: &Path, idx: usize) -> Result<PartGeom, String> {
    let disk = ImgDisk::open(img)?;
    read_part(&disk, idx).map_err(|e| format!("read_part {idx}: {e:?}"))
}

fn write_journal(img: &Path, journal: &Journal) -> Result<(), String> {
    let p1 = esp_p1(img);
    let mut sec = [0u8; SECTOR];
    journal.encode(&mut sec);
    let mut file = vec![0u8; 4096];
    file[..SECTOR].copy_from_slice(&sec);
    crate::fat32_write::write_root_file(img, p1, b"SOSORES ", b"TXT", &file)
        .map_err(|e| e.to_string())
}

fn borrar_journal(img: &Path) -> Result<(), String> {
    write_journal(img, &Journal::idle())
}

fn esp_p1(live: &Path) -> u64 {
    crate::package_live::partition_first_sector(live, 1)
        .expect("ESP p1 del live")
}

struct ImgDisk {
    file: std::fs::File,
    fail_after: Cell<Option<u64>>,
    writes: Cell<u64>,
}

impl ImgDisk {
    fn open(path: &Path) -> Result<Self, String> {
        Ok(Self {
            file: std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|e| e.to_string())?,
            fail_after: Cell::new(None),
            writes: Cell::new(0),
        })
    }
}

impl Disk512 for ImgDisk {
    fn read_sector(&self, lba: u64, buf: &mut [u8; SECTOR]) -> Result<(), ResizeError> {
        let mut f = &self.file;
        f.seek(SeekFrom::Start(lba * SECTOR as u64))
            .map_err(|_| ResizeError::Io)?;
        f.read_exact(buf).map_err(|_| ResizeError::Io)
    }

    fn write_sector(&mut self, lba: u64, buf: &[u8; SECTOR]) -> Result<(), ResizeError> {
        let n = self.writes.get() + 1;
        self.writes.set(n);
        if self.fail_after.get().is_some_and(|f| n > f) {
            return Err(ResizeError::Io);
        }
        self.file
            .seek(SeekFrom::Start(lba * SECTOR as u64))
            .map_err(|_| ResizeError::Io)?;
        self.file
            .write_all(buf)
            .map_err(|_| ResizeError::Io)
    }
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

fn lanzar_live(code: &Path, vars: &Path, live: &Path, serial: &Path) -> Result<QemuProc, String> {
    let mut cmd = Command::new("qemu-system-x86_64");
    crate::apply_qemu_accel(&mut cmd);
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
    cmd.args([
        "-drive",
        &format!("file={},format=raw,if=none,id=live0", live.display()),
    ]);
    cmd.args(["-device", "virtio-blk-pci,drive=live0"]);
    crate::apply_qemu_nic_with_ports(&mut cmd, SSH_PORT, SSH_PORT + 1, Some(&MAC.to_string()));
    cmd.args(["-serial", &format!("file:{}", serial.display())])
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_millis(800));
    if let Some(code) = child.try_wait().map_err(|e| e.to_string())? {
        let mut err = String::new();
        if let Some(mut s) = child.stderr.take() {
            let _ = s.read_to_string(&mut err);
        }
        return Err(format!(
            "QEMU salió al arrancar ({code}): {}",
            err.trim()
        ));
    }
    Ok(QemuProc { child })
}

fn marca(msg: &str, ok: bool) {
    let tag = if ok { "OK" } else { "FALLO" };
    println!("test-resize: [{tag}] {msg}");
}
