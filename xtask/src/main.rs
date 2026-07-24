//! Herramienta de construcción y ejecución de soso.
//!
//! Uso: `cargo xtask <build|run|gdb>`
//!
//! - `build`: compila el kernel y genera la imagen de disco BIOS.
//! - `run`:   build + lanza QEMU (q35, consola por serie en stdio).
//! - `gdb`:   como `run` pero congelado en el arranque con stub GDB en :1234.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "run".into());
    match cmd.as_str() {
        "build" => {
            build_image();
        }
        "run" => {
            let img = build_image();
            run_qemu(&img, false);
        }
        "gdb" => {
            let img = build_image();
            run_qemu(&img, true);
        }
        "mkfs" => {
            build_user();
            mkfs(true);
        }
        "test" => {
            test::run();
            if matches!(
                std::env::var("SOSO_LXDDE_TEST").as_deref(),
                Ok("1") | Ok("true")
            ) {
                test::run_lx_e1000e_smoke();
            }
        }
        "convert-gguf" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            convert_gguf(&args);
        }
        "package-usb" => {
            package_usb();
        }
        "package-usb-live" => {
            package_live::run();
        }
        "lx-build" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            lx_build::run(&args);
        }
        "bench-llm" => {
            bench::run();
        }
        "g1-check" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            g1_check::run(&args);
        }
        "test-distributed-llm" => {
            test_distributed::run();
        }
        "test-distributed-llm-3" => {
            test_distributed::run_3();
        }
        other => {
            eprintln!(
                "comando desconocido: {other} \
                 (usa build | run | gdb | mkfs | test | test-distributed-llm | test-distributed-llm-3 | convert-gguf | package-usb | package-usb-live | lx-build | bench-llm | g1-check)"
            );
            exit(2);
        }
    }
}

mod bench;
mod g1_check;
mod lx_build;
mod package_live;
mod test;
mod test_distributed;

fn convert_gguf(args: &[String]) {
    let root = project_root();
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&root)
        .args(["run", "-q", "-p", "convert-gguf", "--"]);
    for a in args {
        cmd.arg(a);
    }
    let status = cmd.status().expect("convert-gguf");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
}

fn project_root() -> PathBuf {
    // xtask vive en <root>/xtask
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// Firmware de arranque: `SOSO_FIRMWARE=bios|uefi` (default bios).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Firmware {
    Bios,
    Uefi,
}

pub(crate) fn firmware() -> Firmware {
    match std::env::var("SOSO_FIRMWARE").unwrap_or_default().to_ascii_lowercase().as_str() {
        "uefi" => Firmware::Uefi,
        _ => Firmware::Bios,
    }
}

/// Rutas típicas de OVMF; override con `SOSO_OVMF_CODE` / `SOSO_OVMF_VARS`.
fn ovmf_paths() -> Option<(PathBuf, PathBuf)> {
    if let (Some(code), Some(vars)) = (
        std::env::var_os("SOSO_OVMF_CODE"),
        std::env::var_os("SOSO_OVMF_VARS"),
    ) {
        let code = PathBuf::from(code);
        let vars = PathBuf::from(vars);
        if code.exists() && vars.exists() {
            return Some((code, vars));
        }
    }
    const CANDIDATES: &[(&str, &str)] = &[
        (
            "/usr/share/OVMF/OVMF_CODE_4M.fd",
            "/usr/share/OVMF/OVMF_VARS_4M.fd",
        ),
        (
            "/usr/share/OVMF/OVMF_CODE.fd",
            "/usr/share/OVMF/OVMF_VARS.fd",
        ),
        (
            "/usr/share/edk2/ovmf/OVMF_CODE.fd",
            "/usr/share/edk2/ovmf/OVMF_VARS.fd",
        ),
        (
            "/usr/share/edk2-ovmf/x64/OVMF_CODE.fd",
            "/usr/share/edk2-ovmf/x64/OVMF_VARS.fd",
        ),
    ];
    for &(code, vars) in CANDIDATES {
        let code = PathBuf::from(code);
        let vars = PathBuf::from(vars);
        if code.exists() && vars.exists() {
            return Some((code, vars));
        }
    }
    None
}

/// Copia escribible de OVMF_VARS (QEMU la muta).
fn ovmf_vars_writable(src: &Path) -> PathBuf {
    let dst = project_root().join("target/OVMF_VARS.fd");
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::copy(src, &dst).expect("no se pudo copiar OVMF_VARS");
    dst
}

/// Compila el kernel y genera imágenes BIOS + UEFI. Devuelve la que
/// corresponda a `SOSO_FIRMWARE` (BIOS por defecto; si se pide UEFI y no
/// hay OVMF, avisa y cae a BIOS).
pub(crate) fn build_image() -> PathBuf {
    let root = project_root();
    if lxdde_enabled() {
        lx_build::run(&["all".into()]);
    }
    let target = root.join("kernel/x86_64-soso.json");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root.join("kernel"))
        .args([
            "build",
            "--target",
            target.to_str().unwrap(),
            "--target-dir",
            root.join("target/kernel").to_str().unwrap(),
        ]);
    if lxdde_enabled() {
        cmd.arg("--features").arg("lxdde");
        if let Some(mode) = lxdde_mode_env() {
            cmd.env("SOSO_LXDDE_MODE", mode);
        }
    }
    let status = cmd.status().expect("no se pudo ejecutar cargo");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }

    let kernel_elf = root.join("target/kernel/x86_64-soso/debug/kernel");
    let builder = bootloader::DiskImageBuilder::new(kernel_elf);

    let bios = root.join("target/soso-bios.img");
    builder
        .create_bios_image(&bios)
        .expect("fallo creando la imagen BIOS");
    println!("imagen BIOS: {}", bios.display());

    let uefi = root.join("target/soso-uefi.img");
    builder
        .create_uefi_image(&uefi)
        .expect("fallo creando la imagen UEFI");
    println!("imagen UEFI: {}", uefi.display());

    match firmware() {
        Firmware::Uefi => {
            if ovmf_paths().is_none() {
                eprintln!(
                    "xtask: SOSO_FIRMWARE=uefi pero no se encontró OVMF \
                     (instala ovmf o define SOSO_OVMF_CODE/SOSO_OVMF_VARS); \
                     usando BIOS"
                );
                bios
            } else {
                uefi
            }
        }
        Firmware::Bios => bios,
    }
}

/// Añade a `qemu` los drives de firmware (pflash OVMF) si la imagen es UEFI.
pub(crate) fn apply_firmware(qemu: &mut Command, img: &Path) {
    let is_uefi = img
        .file_name()
        .and_then(|n| n.to_str())
        == Some("soso-uefi.img");
    if !is_uefi {
        return;
    }
    let Some((code, vars_src)) = ovmf_paths() else {
        return;
    };
    let vars = ovmf_vars_writable(&vars_src);
    let code_arg = format!(
        "if=pflash,format=raw,readonly=on,file={}",
        code.display()
    );
    let vars_arg = format!("if=pflash,format=raw,file={}", vars.display());
    qemu.args(["-drive", &code_arg]);
    qemu.args(["-drive", &vars_arg]);
}

/// Compila el workspace user/ (release) y copia los ELF a rootfs/bin.
pub(crate) fn build_user() -> bool {
    let root = project_root();
    let status = Command::new("cargo")
        .current_dir(root.join("user"))
        .args(["build", "--release", "--target-dir"])
        .arg(root.join("target/user"))
        .status()
        .expect("no se pudo compilar user/");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
    let out = root.join("target/user/x86_64-soso-user/release");
    let bin = root.join("rootfs/bin");
    std::fs::create_dir_all(&bin).expect("no se pudo crear rootfs/bin");
    let mut cambiado = false;
    for prog in ["init", "sosh", "ls", "cat", "echo", "mkdir", "rm", "hexdump", "halt", "soso-llm"] {
        let src = out.join(prog);
        let dst = bin.join(prog);
        let igual = std::fs::read(&src).ok() == std::fs::read(&dst).ok();
        if !igual {
            std::fs::copy(&src, &dst).expect("no se pudo copiar el binario");
            cambiado = true;
        }
    }
    cambiado
}

/// mtime más reciente de un árbol de directorios.
fn newest_mtime(dir: &Path) -> std::time::SystemTime {
    let mut newest = std::time::SystemTime::UNIX_EPOCH;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let t = if e.path().is_dir() {
                newest_mtime(&e.path())
            } else {
                e.metadata().and_then(|m| m.modified()).unwrap_or(newest)
            };
            newest = newest.max(t);
        }
    }
    newest
}

/// Disco de datos persistente (virtio-blk 0) con sosofs desde rootfs/.
pub(crate) fn mkfs_rootfs(force: bool) -> PathBuf {
    let root = project_root();
    pack_nvidia_firmware(&root);
    let path = root.join("target/soso-data.img");
    let vieja = path
        .metadata()
        .and_then(|m| m.modified())
        .map(|img| newest_mtime(&root.join("rootfs")) > img)
        .unwrap_or(true);
    if path.exists() && !force && !vieja {
        return path;
    }
    let pubkey = client_pubkey();
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["run", "-q", "-p", "mkfs-soso", "--"])
        .arg(root.join("rootfs"))
        .arg(&path)
        .arg("64")
        .arg(&pubkey)
        .status()
        .expect("no se pudo ejecutar mkfs-soso");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
    path
}

/// Disco de modelos (virtio-blk 1) con sosomfs. Por defecto empaqueta el
/// modelo sintético tiny; `SOSO_MODELS_DIR=<dir>` usa un modelo propio
/// (p. ej. el resultado de `cargo xtask convert-gguf`).
pub(crate) fn mkfs_models(force: bool) -> PathBuf {
    let root = project_root();
    let path = root.join("target/soso-models.img");
    let custom = std::env::var_os("SOSO_MODELS_DIR").map(PathBuf::from);
    let model_src = custom
        .clone()
        .unwrap_or_else(|| root.join("target/tiny-model"));
    if custom.is_some() {
        if !model_src.join("manifest.som").exists() {
            eprintln!(
                "xtask: SOSO_MODELS_DIR={} no contiene manifest.som",
                model_src.display()
            );
            exit(1);
        }
    } else {
        // regenerar siempre: un tiny-model viejo puede tener un formato
        // .som anterior y es barato de reconstruir
        let status = Command::new("cargo")
            .current_dir(&root)
            .args(["run", "-q", "--release", "-p", "mkmodel-soso", "--"])
            .arg(&model_src)
            .status()
            .expect("mkmodel-soso");
        if !status.success() {
            exit(status.code().unwrap_or(1));
        }
    }
    let vieja = path
        .metadata()
        .and_then(|m| m.modified())
        .map(|img| newest_mtime(&model_src) > img)
        .unwrap_or(true);
    // con modelo propio se reconstruye siempre: la imagen puede venir de otro dir
    if path.exists() && !force && !vieja && custom.is_none() {
        return path;
    }
    let size = std::env::var("SOSO_MODELS_SIZE").unwrap_or_else(|_| "8G".into());
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["run", "-q", "--release", "-p", "mkfs-sosomfs", "--"])
        .arg(&model_src)
        .arg(&path)
        .arg("--size")
        .arg(&size)
        .status()
        .expect("mkfs-sosomfs");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
    path
}

/// Regenera ambos discos (rootfs + modelos).
fn mkfs(force: bool) -> (PathBuf, PathBuf) {
    (mkfs_rootfs(force), mkfs_models(force))
}

/// Clave pública ed25519 a autorizar en el FS. Usa ~/.ssh/id_ed25519.pub
/// si existe; si no, genera un par de test dedicado en target/ con
/// ssh-keygen (para que `cargo xtask test` pueda conectar).
fn client_pubkey() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let user = PathBuf::from(&home).join(".ssh/id_ed25519.pub");
        if user.exists() {
            return user;
        }
    }
    let key = project_root().join("target/soso_test_key");
    let pubk = project_root().join("target/soso_test_key.pub");
    if !pubk.exists() {
        let _ = std::fs::remove_file(&key);
        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-C", "soso-test", "-f"])
            .arg(&key)
            .status()
            .expect("no se pudo ejecutar ssh-keygen para la clave de test");
        if !status.success() {
            exit(status.code().unwrap_or(1));
        }
        println!("xtask: clave de test generada en {}", key.display());
    }
    pubk
}

/// Memoria y nº de CPUs de QEMU, configurables por entorno:
/// `SOSO_QEMU_MEM=64G SOSO_QEMU_SMP=8 cargo xtask run`.
pub(crate) fn qemu_mem() -> String {
    std::env::var("SOSO_QEMU_MEM").unwrap_or_else(|_| "2G".into())
}

pub(crate) fn qemu_smp() -> String {
    std::env::var("SOSO_QEMU_SMP").unwrap_or_else(|_| "1".into())
}

/// `SOSO_QEMU_NVME=1`: añade un NVMe con la imagen de modelos (además de virtio).
pub(crate) fn qemu_nvme() -> bool {
    matches!(
        std::env::var("SOSO_QEMU_NVME").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// `SOSO_QEMU_NVME_ROOT=1`: rootfs en NVMe ctrl 0; omite virtio-blk0; models en NVMe ctrl 1.
pub(crate) fn qemu_nvme_root() -> bool {
    matches!(
        std::env::var("SOSO_QEMU_NVME_ROOT").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// `SOSO_QEMU_LIVE=1`: un solo disco con `soso-live.img` (GPT part2/3).
/// `SOSO_QEMU_LIVE_USB=1`: mismo disco vía qemu-xhci + usb-storage (prueba BOT).
pub(crate) fn qemu_live() -> bool {
    matches!(
        std::env::var("SOSO_QEMU_LIVE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    ) || qemu_live_usb()
}

pub(crate) fn qemu_live_usb() -> bool {
    matches!(
        std::env::var("SOSO_QEMU_LIVE_USB").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// `SOSO_QEMU_NIC=e1000e|lx-e1000e` sustituye virtio-net; default virtio.
pub(crate) fn qemu_nic() -> String {
    std::env::var("SOSO_QEMU_NIC").unwrap_or_else(|_| "virtio".into())
}

fn lxdde_enabled() -> bool {
    matches!(
        std::env::var("SOSO_LXDDE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    ) || matches!(
        qemu_nic().to_ascii_lowercase().as_str(),
        "lx-e1000e" | "lx_e1000e"
    ) || matches!(
        std::env::var("SOSO_LXDDE_MODE").as_deref(),
        Ok("nouveau")
    ) || std::env::var("SOSO_QEMU_GPU").is_ok()
}

fn lxdde_mode_env() -> Option<String> {
    if let Ok(m) = std::env::var("SOSO_LXDDE_MODE") {
        return Some(m);
    }
    if std::env::var("SOSO_QEMU_GPU").is_ok() {
        return Some("nouveau".into());
    }
    match qemu_nic().to_ascii_lowercase().as_str() {
        "lx-e1000e" | "lx_e1000e" => Some("e1000e".into()),
        _ => None,
    }
}

fn pack_nvidia_firmware(root: &Path) {
    let script = root.join("scripts/l6-pack-firmware.sh");
    if !script.exists() {
        return;
    }
    let fw_dst = root.join("rootfs/lib/firmware/nvidia");
    if fw_dst.exists() && fw_dst.read_dir().map(|mut d| d.next().is_some()).unwrap_or(false) {
        return;
    }
    let status = Command::new("bash")
        .arg(&script)
        .current_dir(root)
        .status();
    match status {
        Ok(s) if s.success() => {}
        _ => println!("xtask: aviso — ejecutar ./scripts/l6-pack-firmware.sh para GSP gb205"),
    }
}

fn nvme_copy(src: &Path, dst_name: &str) -> PathBuf {
    let dst = project_root().join("target").join(dst_name);
    std::fs::copy(src, &dst).unwrap_or_else(|e| panic!("copiar {} → {}: {e}", src.display(), dst.display()));
    dst
}

/// Discos de QEMU: virtio (default), NVMe extra para modelos, o solo NVMe para root+models.
pub(crate) fn apply_qemu_disks(qemu: &mut Command, data: &Path, models: &Path) {
    if qemu_live() {
        package_live::ensure_live_image();
        let live = package_live::live_image_path();
        qemu.args([
            "-drive",
            &format!("file={},format=raw,if=none,id=live0", live.display()),
        ]);
        if qemu_live_usb() {
            qemu.args(["-device", "qemu-xhci,id=xhci"]);
            qemu.args(["-device", "usb-storage,bus=xhci.0,drive=live0"]);
            println!("xtask: modo live USB → {}", live.display());
        } else {
            qemu.args(["-device", "virtio-blk-pci,drive=live0"]);
            println!("xtask: modo live virtio → {}", live.display());
        }
        return;
    }

    if qemu_nvme_root() {
        let nvme_root = nvme_copy(data, "soso-data-nvme.img");
        let nvme_models = nvme_copy(models, "soso-models-nvme.img");
        qemu.args([
            "-drive",
            &format!("file={},format=raw,if=none,id=nvme0", nvme_root.display()),
        ]);
        qemu.args(["-device", "nvme,serial=soso-root,drive=nvme0"]);
        qemu.args([
            "-drive",
            &format!("file={},format=raw,if=none,id=nvme1", nvme_models.display()),
        ]);
        qemu.args(["-device", "nvme,serial=soso-models,drive=nvme1"]);
        return;
    }

    qemu.args(["-drive", &format!("file={},format=raw,if=none,id=data0", data.display())])
        .args(["-device", "virtio-blk-pci,drive=data0"])
        .args(["-drive", &format!("file={},format=raw,if=none,id=data1", models.display())])
        .args(["-device", "virtio-blk-pci,drive=data1"]);

    if qemu_nvme() {
        let nvme_img = nvme_copy(models, "soso-models-nvme.img");
        qemu.args([
            "-drive",
            &format!("file={},format=raw,if=none,id=nvme0", nvme_img.display()),
        ]);
        qemu.args(["-device", "nvme,serial=soso,drive=nvme0"]);
    }
}

pub(crate) fn apply_qemu_nic(qemu: &mut Command) {
    qemu.args([
        "-netdev",
        "user,id=net0,hostfwd=tcp::7777-:7,hostfwd=tcp::2222-:22",
    ]);
    let mac = std::env::var("SOSO_QEMU_MAC").unwrap_or_else(|_| "52:54:00:12:34:15".into());
    match qemu_nic().to_ascii_lowercase().as_str() {
        "e1000e" | "e1000" => {
            qemu.args(["-device", &format!("e1000e,netdev=net0,mac={mac}")]);
        }
        "lx-e1000e" | "lx_e1000e" => {
            qemu.args(["-device", &format!("e1000e,netdev=net0,mac={mac}")]);
        }
        _ => {
            qemu.args(["-device", &format!("virtio-net-pci,netdev=net0,mac={mac}")]);
        }
    }
}

/// `SOSO_QEMU_GPU=vfio:BB:DD.F` — passthrough VFIO de GPU NVIDIA (G1).
pub(crate) fn apply_qemu_gpu(qemu: &mut Command) {
    let Ok(spec) = std::env::var("SOSO_QEMU_GPU") else {
        return;
    };
    let Some(bdf) = spec.strip_prefix("vfio:") else {
        eprintln!("xtask: SOSO_QEMU_GPU debe ser vfio:BB:DD.F (got {spec})");
        return;
    };
    let arg = format!("vfio-pci,host={bdf}");
    qemu.args(["-device", &arg]);
    println!("xtask: GPU VFIO {arg}");
}

fn package_usb() {
    build_user();
    let root = project_root();
    let _ = build_image();
    let uefi = root.join("target/soso-uefi.img");
    if !uefi.exists() {
        eprintln!("xtask: falta {}", uefi.display());
        exit(1);
    }
    let data = mkfs_rootfs(true);
    let models = mkfs_models(true);
    let out = root.join("target/usb-package");
    std::fs::create_dir_all(&out).expect("crear target/usb-package");
    for (src, name) in [
        (&uefi, "soso-uefi.img"),
        (&data, "soso-data.img"),
        (&models, "soso-models.img"),
    ] {
        let dst = out.join(name);
        std::fs::copy(src, &dst).unwrap_or_else(|e| {
            panic!("copiar {} → {}: {e}", src.display(), dst.display());
        });
        println!("package-usb: {}", dst.display());
    }

    let flash = out.join("FLASH.txt");
    let flash_body = format!(
        r#"soso — paquete UEFI + discos para bring-up en placa (L5c)

Artefactos en este directorio:
  soso-uefi.img   — imagen de arranque UEFI (kernel en ESP)
  soso-data.img   — rootfs sosofs (virtio-blk o NVMe según hw)
  soso-models.img — modelos sosomfs (segundo NVMe o virtio-blk1)

1) USB de arranque (ESP)
   Identifica el stick (ej. /dev/sdX). ¡DESTRUYE el contenido del dispositivo!

   sudo dd if=soso-uefi.img of=/dev/sdX bs=4M status=progress conv=fsync

   Arranca la placa con UEFI desde USB.

2) Rootfs en NVMe del host (cuando no hay virtio-blk)
   sudo dd if=soso-data.img of=/dev/nvme0n1 bs=4M status=progress conv=fsync

3) Modelos en segundo NVMe (o disco dedicado)
   sudo dd if=soso-models.img of=/dev/nvme1n1 bs=4M status=progress conv=fsync

   Si solo hay un NVMe: copia rootfs y models en particiones distintas
   (mkfs/parted manual) o usa un SSD USB para modelos.

4) Verificación en la placa
   - Consola serie o GOP: log MCFG, nvme[0]/nvme[1], NIC PCI ID
   - DHCP, ssh root@<ip>, soso-llm run <modelo>

Generado: {gen}
"#,
        gen = chrono_lite_now()
    );
    std::fs::write(&flash, flash_body).expect("escribir FLASH.txt");
    println!("package-usb: {}", flash.display());
    println!("\n✅ Paquete listo en {}", out.display());
}

fn chrono_lite_now() -> String {
    use std::process::Command;
    Command::new("date")
        .arg("-Iseconds")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn run_qemu(img: &Path, gdb: bool) {
    build_user();
    let (data, models) = mkfs(false);
    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-machine", "q35"])
        // -cpu max: expone RDRAND, que la cripto de sunset (getrandom con
        // backend rdrand) necesita; la CPU por defecto de QEMU no lo trae.
        .args(["-cpu", "max"])
        .args(["-m", &qemu_mem()])
        .args(["-smp", &qemu_smp()]);
    apply_firmware(&mut qemu, img);
    qemu.args(["-drive", &format!("format=raw,file={}", img.display())]);
    apply_qemu_disks(&mut qemu, &data, &models);
    apply_qemu_nic(&mut qemu);
    apply_qemu_gpu(&mut qemu);
    // mon:stdio multiplexa monitor y serie: Ctrl-A X sale, Ctrl-A C monitor
    qemu.args(["-serial", "mon:stdio"])
        .args(["-display", "none"])
        // Dispositivo de salida para tests: out 0xf4 -> exit((valor << 1) | 1)
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .arg("-no-reboot");
    if gdb {
        // Congelado en arranque; conectar con: gdb -ex 'target remote :1234'
        qemu.args(["-s", "-S"]);
    }
    let status = qemu.status().expect("no se pudo ejecutar qemu-system-x86_64 (¿está instalado?)");
    exit(status.code().unwrap_or(0));
}
