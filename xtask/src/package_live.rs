//! `cargo xtask package-usb-live` — imagen GPT única para arranque live (sin tocar NVMe interno).

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

pub fn run() {
    let root = super::project_root();
    super::build_user();
    let _ = super::build_image();

    let uefi = root.join("target/soso-uefi.img");
    let data = super::mkfs_rootfs(true);
    let models = super::mkfs_models(true);

    let data_len = std::fs::metadata(&data).expect("data").len();
    let models_len = std::fs::metadata(&models).expect("models").len();
    let uefi_len = std::fs::metadata(&uefi).expect("uefi").len();

    let align = 1024 * 1024;
    let header = (uefi_len + align - 1) / align * align;
    let p2_size = (data_len + align - 1) / align * align;
    let p3_size = (models_len + align - 1) / align * align;
    let total = header + p2_size + p3_size + align;

    let out_dir = root.join("target/usb-live");
    std::fs::create_dir_all(&out_dir).expect("usb-live dir");
    let live = out_dir.join("soso-live.img");

    std::fs::write(&live, vec![0u8; total as usize]).expect("truncate live img");

    run_cmd(
        Command::new("dd")
            .arg(format!("if={}", uefi.display()))
            .arg(format!("of={}", live.display()))
            .args(["bs=512", "conv=notrunc"]),
        "dd uefi",
    );

    run_cmd(Command::new("sgdisk").arg("-e").arg(&live), "sgdisk -e");

    let p2_mb = (p2_size / (1024 * 1024)).max(1);
    run_cmd(
        Command::new("sgdisk")
            .arg("-n")
            .arg(format!("2:0:+{p2_mb}M"))
            .arg("-t")
            .arg("2:8300")
            .arg("-n")
            .arg("3:0:0")
            .arg("-t")
            .arg("3:8300")
            .arg(&live),
        "sgdisk add",
    );

    let p2_start = partition_first_sector(&live, 2).expect("part2 lba");
    let p3_start = partition_first_sector(&live, 3).expect("part3 lba");

    run_cmd(
        Command::new("dd")
            .arg(format!("if={}", data.display()))
            .arg(format!("of={}", live.display()))
            .arg("bs=512")
            .arg(format!("seek={p2_start}"))
            .arg("conv=notrunc"),
        "dd data",
    );
    run_cmd(
        Command::new("dd")
            .arg(format!("if={}", models.display()))
            .arg(format!("of={}", live.display()))
            .arg("bs=512")
            .arg(format!("seek={p3_start}"))
            .arg("conv=notrunc"),
        "dd models",
    );

    write_flash(&out_dir, &live, data_len, models_len, total);
    println!("package-usb-live: {}", live.display());
    println!("\n✅ Live USB listo en {}", out_dir.display());
}

fn write_flash(out_dir: &Path, _live: &Path, data_len: u64, models_len: u64, total: u64) {
    let flash = out_dir.join("FLASH-LIVE.txt");
    let body = format!(
        r#"soso — arranque LIVE desde USB (no modifica el disco interno)

Archivo: soso-live.img ({:.1} GiB)
  Partición 1 — ESP UEFI (kernel)
  Partición 2 — rootfs sosofs (~{:.0} MiB)
  Partición 3 — modelos sosomfs (~{:.1} GiB)

1) Escribir SOLO en el pendrive (identifica con lsblk, ej. /dev/sdX):

   sudo dd if=soso-live.img of=/dev/sdX bs=4M status=progress conv=fsync

2) UEFI → arrancar una vez desde USB (F12 / Boot menu).

3) Consola serie / GOP: live: GPT … | fs: sosofs live | ssh soso@<ip>

4) Apagar, quitar USB, arrancar disco habitual → Linux intacto.

QEMU: SOSO_QEMU_LIVE=1 cargo xtask run

Generado: {gen}
"#,
        total as f64 / (1024.0 * 1024.0 * 1024.0),
        data_len as f64 / (1024.0 * 1024.0),
        models_len as f64 / (1024.0 * 1024.0),
        gen = chrono_now()
    );
    std::fs::write(&flash, body).expect("FLASH-LIVE.txt");
    println!("package-usb-live: {}", flash.display());
}

fn chrono_now() -> String {
    Command::new("date")
        .arg("-Iseconds")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn run_cmd(cmd: &mut Command, label: &str) {
    let st = cmd.status().unwrap_or_else(|e| panic!("{label}: {e}"));
    if !st.success() {
        eprintln!("xtask: {label} falló");
        exit(st.code().unwrap_or(1));
    }
}

fn partition_first_sector(img: &Path, part: u32) -> Option<u64> {
    let out = Command::new("sgdisk")
        .args(["-i", &part.to_string()])
        .arg(img)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if line.contains("First sector:") {
            return line
                .split(':')
                .nth(1)?
                .trim()
                .split_whitespace()
                .next()?
                .parse()
                .ok();
        }
    }
    None
}

pub fn live_image_path() -> PathBuf {
    super::project_root().join("target/usb-live/soso-live.img")
}

pub fn ensure_live_image() {
    let p = live_image_path();
    if !p.exists() {
        run();
    }
}
