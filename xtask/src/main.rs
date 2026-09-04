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
        "fetch-hf" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            fetch_hf::run(&args);
        }
        "fetch-whisper" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            fetch_whisper::run(&args);
        }
        "package-usb" => {
            package_usb();
        }
        "package-usb-live" => {
            package_live::run();
        }
        "install-disk" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            install_disk::run(&args);
        }
        "flash-usb-live" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            flash_usb_live::run(&args);
        }
        "sosolog" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            sosolog::run(&args);
        }
        "lx-build" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            lx_build::run(&args);
        }
        "fit-drivers" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            drivers::run_fit_drivers(&args);
        }
        "driver-add" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            drivers::run_driver_add(&args);
        }
        "bench-llm" => {
            bench::run();
        }
        "g1-check" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            g1_check::run(&args);
        }
        "g3-check" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            g3_check::run(&args);
        }
        "test-distributed-llm" => {
            test_distributed::run();
        }
        "test-distributed-llm-3" => {
            test_distributed::run_3();
        }
        "test-usb" => {
            test::run_usb();
        }
        "test-install" => {
            test_install::run();
        }
        "test-update" => {
            test_update::run();
        }
        "release" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            release::run(&args);
        }
        "sosomfs-check" => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            sosomfs_check::run(&args);
        }
        other => {
            eprintln!(
                "comando desconocido: {other} \
                 (usa build | run | gdb | mkfs | test | test-usb | test-install | test-update | release | sosomfs-check | test-distributed-llm | test-distributed-llm-3 | convert-gguf | fetch-hf | fetch-whisper | package-usb | package-usb-live | install-disk | flash-usb-live | sosolog | lx-build | fit-drivers | driver-add | bench-llm | g1-check | g3-check)"
            );
            exit(2);
        }
    }
}

mod bench;
mod drivers;
mod fat32_write;
mod fetch_hf;
mod fetch_whisper;
mod flash_usb_live;
mod g1_check;
mod g3_check;
mod install_disk;
mod live_models;
mod lx_build;
mod package_live;
mod release;
mod sosolog;
mod test;
mod test_distributed;
mod sosomfs_check;
mod test_install;
mod test_update;
mod version;

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
pub(crate) fn ovmf_paths() -> Option<(PathBuf, PathBuf)> {
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
    build_image_with_profile(&drivers::profile_from_env_or_args(), false)
}

pub(crate) fn build_image_with_profile(
    profile: &drivers::DriverProfile,
    reserve_update_slots: bool,
) -> PathBuf {
    let root = project_root();
    let ports = drivers::lx_ports_for_build(profile);
    if !ports.is_empty() {
        lx_build::run(&ports);
    } else if lxdde_enabled() {
        lx_build::run(&["all".into()]);
    }
    let target = root.join("kernel/x86_64-soso.json");
    let feats = drivers::kernel_feature_args(profile);
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root.join("kernel"))
        .args([
            "build",
            "--target",
            target.to_str().unwrap(),
            "--target-dir",
            root.join("target/kernel").to_str().unwrap(),
            "--features",
            &feats.join(","),
        ]);
    if feats.iter().any(|f| f == "lxdde") {
        if let Some(mode) = profile.lxdde_mode.clone().or_else(lxdde_mode_env) {
            cmd.env("SOSO_LXDDE_MODE", mode);
        }
    }
    cmd.env("SOSO_VERSION", version::read_version(&root));
    cmd.env("SOSO_BUILD", version::git_build(&root));
    let status = cmd.status().expect("no se pudo ejecutar cargo");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }

    let kernel_elf = root.join("target/kernel/x86_64-soso/debug/kernel");
    let mut builder = bootloader::DiskImageBuilder::new(kernel_elf);

    // Shim UEFI de diagnóstico (boot-shim/): el loader real viaja como
    // efi/boot/bootsoso.efi y BOOTMARK.TXT recoge la marca de que el firmware
    // llegó a ejecutarnos. bootx64.efi se sustituye por el shim tras generar
    // la imagen (el builder no permite pisar sus ficheros internos).
    let shim = build_boot_shim(&root);
    if shim.is_some() {
        if let Some(loader) = uefi_loader_bytes(&root) {
            builder.set_file_contents("efi/boot/bootsoso.efi".into(), loader);
            builder.set_file_contents("bootmark.txt".into(), vec![b'\n'; 4096]);
        }
    }
    if reserve_update_slots {
        builder.set_file_contents("SOSOUPD.TXT".into(), vec![b'\n'; soso_abi::UPD_MAILBOX_SIZE]);
        builder.set_file_contents(
            "SOSOKRN.BIN".into(),
            vec![0u8; soso_update_core::UPD_KERNEL_SLOT_SIZE],
        );
        println!(
            "ESP: huecos de actualización (SOSOUPD.TXT + SOSOKRN.BIN {} MiB)",
            soso_update_core::UPD_KERNEL_SLOT_SIZE / (1024 * 1024)
        );
    }

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

    if let Some(shim) = shim {
        install_boot_shim(&uefi, &shim);
    }

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

/// Compila `boot-shim/` para x86_64-unknown-uefi y devuelve la ruta del .efi.
/// Best-effort: sin el target instalado avisa y la imagen queda estándar.
fn build_boot_shim(root: &Path) -> Option<PathBuf> {
    let target_dir = root.join("target/boot-shim");
    let status = Command::new("cargo")
        .args([
            "build",
            "--manifest-path",
            root.join("boot-shim/Cargo.toml").to_str().unwrap(),
            "--release",
            "--target",
            "x86_64-unknown-uefi",
            "--target-dir",
            target_dir.to_str().unwrap(),
        ])
        .status();
    if !matches!(&status, Ok(s) if s.success()) {
        eprintln!(
            "xtask: aviso: boot-shim no compiló (¿rustup target add x86_64-unknown-uefi?); \
             imagen UEFI sin shim/BOOTMARK"
        );
        return None;
    }
    let efi = target_dir.join("x86_64-unknown-uefi/release/boot-shim.efi");
    efi.exists().then_some(efi)
}

/// Extrae los bytes del bootloader UEFI embebido en el crate `bootloader`
/// (no hay API pública directa; la carpeta TFTP lo escribe como `bootloader`).
fn uefi_loader_bytes(root: &Path) -> Option<Vec<u8>> {
    let dir = root.join("target/boot-shim/tftp");
    std::fs::create_dir_all(&dir).ok()?;
    bootloader::DiskImageBuilder::empty()
        .create_uefi_tftp_folder(&dir)
        .ok()?;
    std::fs::read(dir.join("bootloader")).ok()
}

/// Sustituye in situ el contenido de efi/boot/bootx64.efi de la imagen UEFI
/// por el shim (los clusters sobrantes quedan a cero; LoadImage usa las
/// cabeceras PE, no el tamaño del fichero).
fn install_boot_shim(uefi_img: &Path, shim: &Path) {
    let data = std::fs::read(shim).expect("leer boot-shim.efi");
    let Some(p1) = gpt_first_partition_lba(uefi_img) else {
        eprintln!("xtask: aviso: sin GPT en la imagen UEFI; shim no instalado");
        return;
    };
    match fat32_write::overwrite_in_dir(
        uefi_img,
        p1,
        &[*b"EFI        ", *b"BOOT       "],
        b"BOOTX64 EFI",
        &data,
    ) {
        Ok(orig) => println!(
            "shim UEFI: bootx64.efi ← boot-shim ({} B; loader real {orig} B en efi/boot/bootsoso.efi)",
            data.len()
        ),
        Err(e) => eprintln!("xtask: aviso: no pude instalar el shim UEFI: {e}"),
    }
}

/// Primer LBA de la partición 1 leyendo la cabecera GPT de la imagen.
pub(crate) fn gpt_first_partition_lba(img: &Path) -> Option<u64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(img).ok()?;
    let mut hdr = [0u8; 512];
    f.seek(SeekFrom::Start(512)).ok()?;
    f.read_exact(&mut hdr).ok()?;
    if &hdr[0..8] != b"EFI PART" {
        return None;
    }
    let parts_lba = u64::from_le_bytes(hdr[72..80].try_into().ok()?);
    let entry_size = u32::from_le_bytes(hdr[84..88].try_into().ok()?) as usize;
    let mut ent = vec![0u8; entry_size.max(128)];
    f.seek(SeekFrom::Start(parts_lba * 512)).ok()?;
    f.read_exact(&mut ent).ok()?;
    if ent[0..16].iter().all(|&b| b == 0) {
        return None;
    }
    Some(u64::from_le_bytes(ent[32..40].try_into().ok()?))
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
    version::write_soso_release(&root);
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
    for prog in [
        "init",
        "sosh",
        "ls",
        "cat",
        "echo",
        "mkdir",
        "rm",
        "hexdump",
        "halt",
        "soso-llm",
        "soso-install",
        "soso-hf",
        "soso-voz",
        "soso-web",
        "ask-modelo",
        "soso-update",
    ] {
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
    mkfs_rootfs_with_profile(force, &drivers::profile_from_env_or_args())
}

pub(crate) fn mkfs_rootfs_with_profile(
    force: bool,
    profile: &drivers::DriverProfile,
) -> PathBuf {
    let root = project_root();
    if profile.kernel_features.iter().any(|f| f == "drv-gpu-nvidia")
        || profile.kernel_features.iter().any(|f| f == "drv-all")
    {
        pack_nvidia_firmware(&root);
    }
    drivers::filter_rootfs_firmware(&root.join("rootfs"), profile);
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
    let fw_gb205 = root.join("rootfs/lib/firmware/nvidia/gb205/gsp/bootloader-570.144.bin");
    let fw_ga102 = root.join("rootfs/lib/firmware/nvidia/ga102/gsp/bootloader-570.144.bin");
    let fw_ga107 = root.join("rootfs/lib/firmware/nvidia/ga107/gsp/bootloader-570.144.bin");
    let ampere = fw_ga102.exists() || fw_ga107.exists();
    let blackwell = fw_gb205.exists();
    // ga102 + gb205 duplican ~60 MiB de ucode; 128 MiB no basta para las dos familias.
    let disk_mib: u64 = match (ampere, blackwell) {
        (true, true) => 256,
        (true, false) | (false, true) => 128,
        (false, false) => 64,
    };
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["run", "-q", "-p", "mkfs-soso", "--"])
        .arg(root.join("rootfs"))
        .arg(&path)
        .arg(disk_mib.to_string())
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
        // tiny-moe en la misma imagen de modelos (Mixtral-style MoE para E2E).
        let moe_src = root.join("target/tiny-moe-model");
        let status = Command::new("cargo")
            .current_dir(&root)
            .args(["run", "-q", "--release", "-p", "mkmodel-soso", "--"])
            .args(["--moe", moe_src.to_str().unwrap()])
            .status()
            .expect("mkmodel-soso tiny-moe");
        if !status.success() {
            exit(status.code().unwrap_or(1));
        }
        // tiny-mla: smoke MLA + KV latente en el disco de modelos por defecto.
        let mla_src = root.join("target/tiny-mla-model");
        let status = Command::new("cargo")
            .current_dir(&root)
            .args(["run", "-q", "--release", "-p", "mkmodel-soso", "--"])
            .args([
                "--attn",
                "mla",
                "--name",
                "tiny-mla",
                "--layers",
                "1",
                mla_src.to_str().unwrap(),
            ])
            .status()
            .expect("mkmodel-soso tiny-mla");
        if !status.success() {
            exit(status.code().unwrap_or(1));
        }
        // tiny-q4k: el ÚNICO modelo cuantizado de la imagen, y sin él todo el
        // camino de pesos Q4_K en el dispositivo —subida en crudo y comando
        // `MATVQ`— quedaría sin ejercitar en la suite. Q4_K exige hidden y ffn
        // múltiplos de 256 (una fila = número entero de superbloques).
        let q4k_src = root.join("target/tiny-q4k-model");
        let status = Command::new("cargo")
            .current_dir(&root)
            .args(["run", "-q", "--release", "-p", "mkmodel-soso", "--"])
            .args([
                "--quant",
                "q4_k",
                "--name",
                "tiny-q4k",
                "--layers",
                "2",
                "--hidden",
                "256",
                "--ffn",
                "512",
                q4k_src.to_str().unwrap(),
            ])
            .status()
            .expect("mkmodel-soso tiny-q4k");
        if !status.success() {
            exit(status.code().unwrap_or(1));
        }
        // tiny-latent-moe: MoE con FFN latente en la imagen por defecto.
        let latent_moe_src = root.join("target/tiny-latent-moe-model");
        let status = Command::new("cargo")
            .current_dir(&root)
            .args(["run", "-q", "--release", "-p", "mkmodel-soso", "--"])
            .args([
                "--moe",
                "--ffn-kind",
                "latent-moe",
                "--name",
                "tiny-latent-moe",
                "--layers",
                "1",
                latent_moe_src.to_str().unwrap(),
            ])
            .status()
            .expect("mkmodel-soso tiny-latent-moe");
        if !status.success() {
            exit(status.code().unwrap_or(1));
        }
        let asr_src = fetch_whisper::asr_model_dir(&root);
        if !asr_src.join("manifest.som").exists() {
            let status = Command::new("cargo")
                .current_dir(&root)
                .args(["run", "-q", "--release", "-p", "mkmodel-soso", "--"])
                .args(["--asr", root.join("target/tiny-asr-model").to_str().unwrap()])
                .status()
                .expect("mkmodel-soso tiny-asr");
            if !status.success() {
                exit(status.code().unwrap_or(1));
            }
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
    let mut mkfs_args = vec![
        "run".to_string(),
        "-q".to_string(),
        "--release".to_string(),
        "-p".to_string(),
        "mkfs-sosomfs".to_string(),
        "--".to_string(),
        model_src.to_string_lossy().into_owned(),
    ];
    if custom.is_none() {
        mkfs_args.push(root.join("target/tiny-moe-model").to_string_lossy().into_owned());
        mkfs_args.push(root.join("target/tiny-mla-model").to_string_lossy().into_owned());
        mkfs_args.push(
            root.join("target/tiny-latent-moe-model")
                .to_string_lossy()
                .into_owned(),
        );
        mkfs_args.push(root.join("target/tiny-q4k-model").to_string_lossy().into_owned());
        let asr_dir = fetch_whisper::asr_model_dir(&root);
        mkfs_args.push(asr_dir.to_string_lossy().into_owned());
    }
    mkfs_args.push(path.to_string_lossy().into_owned());
    mkfs_args.push("--size".into());
    mkfs_args.push(size);
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(&mkfs_args)
        .status()
        .expect("mkfs-sosomfs");
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
    path
}

/// Disco de modelos para el live USB.
///
/// `primary` va delante de `tiny` en sosomfs (el demo usa el primero de `/models`).
/// `SOSO_MODELS_SIZE` puede forzar el tamaño de imagen; si no, se calcula del árbol.
pub(crate) fn mkfs_models_live_for_dirs(
    force: bool,
    primary: &Path,
    tiny: Option<&Path>,
) -> PathBuf {
    let root = project_root();
    let path = root.join("target/soso-models-live.img");

    if !primary.join("manifest.som").exists() {
        eprintln!(
            "xtask: {} no contiene manifest.som",
            primary.display()
        );
        exit(1);
    }
    if let Some(t) = tiny {
        if !t.join("manifest.som").exists() {
            eprintln!("xtask: {} no contiene manifest.som", t.display());
            exit(1);
        }
    }

    let size = std::env::var("SOSO_MODELS_SIZE").unwrap_or_else(|_| {
        if let Some(t) = tiny {
            live_models::suggest_models_image_size(primary, t)
        } else {
            live_models::suggest_models_image_size(primary, primary)
        }
    });

    let vieja = path.metadata().and_then(|m| m.modified()).map(|img| {
        newest_mtime(primary) > img || tiny.map(|t| newest_mtime(t) > img).unwrap_or(false)
    }).unwrap_or(true);
    if path.exists() && !force && !vieja {
        return path;
    }

    let label = match tiny {
        Some(t) => format!(
            "{} + {} ({})",
            primary.file_name().unwrap().to_string_lossy(),
            t.file_name().unwrap().to_string_lossy(),
            size
        ),
        None => format!("{} ({})", primary.display(), size),
    };
    println!("package-usb-live: modelos {label}");

    let mut cmd = Command::new("cargo");
    cmd.current_dir(&root)
        .args(["run", "-q", "--release", "-p", "mkfs-sosomfs", "--"])
        .arg(primary);
    if let Some(t) = tiny {
        cmd.arg(t);
    }
    if let Some(asr) = std::env::var_os("SOSO_ASR_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            let w = root.join("target/whisper-tiny-model");
            w.join("manifest.som").exists().then_some(w)
        })
    {
        if asr.join("manifest.som").exists() {
            cmd.arg(&asr);
            println!("package-usb-live: SOSO_ASR_DIR={}", asr.display());
        }
    }
    cmd.arg(&path).arg("--size").arg(&size);
    let status = cmd.status().expect("mkfs-sosomfs live");
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

/// `/dev/kvm` legible por el usuario actual.
pub(crate) fn kvm_usable() -> bool {
    let kvm = Path::new("/dev/kvm");
    kvm.exists()
        && std::fs::OpenOptions::new()
            .read(true)
            .open(kvm)
            .is_ok()
}

/// Acelerador QEMU: auto (`kvm` si hay `/dev/kvm`, si no TCG), o
/// `SOSO_QEMU_ACCEL=kvm|tcg`.
pub(crate) fn qemu_accel_mode() -> &'static str {
    match std::env::var("SOSO_QEMU_ACCEL").as_deref() {
        Ok("tcg") => "tcg",
        Ok("kvm") => {
            if kvm_usable() {
                "kvm"
            } else {
                eprintln!("xtask: SOSO_QEMU_ACCEL=kvm pero /dev/kvm no usable; TCG");
                "tcg"
            }
        }
        Ok(other) => {
            eprintln!("xtask: SOSO_QEMU_ACCEL={other} desconocido; auto");
            if kvm_usable() {
                "kvm"
            } else {
                "tcg"
            }
        }
        Err(_) => {
            if kvm_usable() {
                "kvm"
            } else {
                "tcg"
            }
        }
    }
}

pub(crate) fn apply_qemu_accel(qemu: &mut Command) {
    if qemu_accel_mode() == "kvm" {
        qemu.args(["-accel", "kvm"]);
    }
}

/// Default de `SOSO_TEST_JOBS`: 4 con KVM, 2 en TCG.
pub(crate) fn test_jobs_default() -> usize {
    if qemu_accel_mode() == "kvm" {
        4
    } else {
        2
    }
}

/// `SOSO_QEMU_NVME=1`: añade un NVMe con la imagen de modelos (además de virtio).
/// `SOSO_QEMU_NVME_IMG=<ruta>`: imagen raw para ese NVMe (p. ej. disco fake Linux).
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
/// `SOSO_QEMU_LIVE_USB=1`: mismo disco vía xHCI + usb-storage (prueba BOT).
/// `SOSO_QEMU_XHCI=qemu|nec`: modelo del controlador (default `qemu`).
/// `SOSO_QEMU_USB_KBD=1`: añade `usb-kbd` al bus xHCI (HID boot).
/// `SOSO_QEMU_USB_HOST=VID:PID`: passthrough de dispositivo USB real (requiere acceso a `/dev/bus/usb`).
/// `SOSO_QEMU_TRACE_USB=1`: trazas `usb_xhci_*` en `target/qemu-usb-trace.log`.
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

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// `SOSO_QEMU_XHCI=qemu|nec` — default `qemu`.
pub(crate) fn qemu_xhci_model() -> String {
    std::env::var("SOSO_QEMU_XHCI").unwrap_or_else(|_| "qemu".into())
}

/// `SOSO_QEMU_USB_KBD=1`: teclado HID emulado en el bus xHCI.
pub(crate) fn qemu_usb_kbd() -> bool {
    env_flag("SOSO_QEMU_USB_KBD")
}

/// `SOSO_QEMU_USB_HUB=1`: storage/kbd detrás de `usb-hub` en xHCI.
pub(crate) fn qemu_usb_hub() -> bool {
    env_flag("SOSO_QEMU_USB_HUB")
}

/// `SOSO_QEMU_USB_HOST=VID:PID` — passthrough USB (hex, con o sin `0x`).
pub(crate) fn qemu_usb_host() -> Option<(u16, u16)> {
    let spec = std::env::var("SOSO_QEMU_USB_HOST").ok()?;
    let (vid_s, pid_s) = spec.split_once(':')?;
    let parse = |s: &str| u16::from_str_radix(s.trim().trim_start_matches("0x").trim_start_matches("0X"), 16).ok();
    Some((parse(vid_s)?, parse(pid_s)?))
}

/// `SOSO_QEMU_TRACE_USB=1`: volcado de trazas xHCI de QEMU.
pub(crate) fn qemu_trace_usb() -> bool {
    env_flag("SOSO_QEMU_TRACE_USB")
}

/// `SOSO_QEMU_NIC=e1000e|lx-e1000e` sustituye virtio-net; default virtio.
pub(crate) fn qemu_nic() -> String {
    std::env::var("SOSO_QEMU_NIC").unwrap_or_else(|_| "virtio".into())
}

pub(crate) fn lxdde_enabled() -> bool {
    matches!(
        std::env::var("SOSO_LXDDE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    ) || matches!(
        qemu_nic().to_ascii_lowercase().as_str(),
        "lx-e1000e" | "lx_e1000e"
    ) || matches!(
        std::env::var("SOSO_LXDDE_MODE").as_deref(),
        Ok(m) if m.contains("nouveau") || m.contains("iwlwifi")
    ) || std::env::var("SOSO_QEMU_GPU").is_ok()
        || std::env::var("SOSO_QEMU_NIC")
            .map(|v| v.starts_with("vfio:"))
            .unwrap_or(false)
}

pub(crate) fn lxdde_mode_env() -> Option<String> {
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
    let fw_gb205 = root.join("rootfs/lib/firmware/nvidia/gb205/gsp/bootloader-570.144.bin");
    let fw_ga107 = root.join("rootfs/lib/firmware/nvidia/ga107/gsp/bootloader-570.144.bin");
    if fw_gb205.exists() || fw_ga107.exists() {
        return;
    }
    let status = Command::new("bash")
        .arg(&script)
        .current_dir(root)
        .status();
    match status {
        Ok(s) if s.success() => {}
        _ => println!(
            "xtask: aviso — ejecutar ./scripts/l6-pack-firmware.sh para GSP (gb205/ga107)"
        ),
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
            println!("xtask: modo live USB (drive) → {}", live.display());
        } else {
            qemu.args(["-device", "virtio-blk-pci,drive=live0"]);
            println!("xtask: modo live virtio → {}", live.display());
        }
        if qemu_nvme() {
            let nvme_img = std::env::var("SOSO_QEMU_NVME_IMG")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| nvme_copy(models, "soso-models-nvme.img"));
            qemu.args([
                "-drive",
                &format!("file={},format=raw,if=none,id=nvme0", nvme_img.display()),
            ]);
            qemu.args(["-device", "nvme,serial=soso,drive=nvme0"]);
            println!("xtask: NVMe extra → {}", nvme_img.display());
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
        let nvme_img = std::env::var("SOSO_QEMU_NVME_IMG")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| nvme_copy(models, "soso-models-nvme.img"));
        qemu.args([
            "-drive",
            &format!("file={},format=raw,if=none,id=nvme0", nvme_img.display()),
        ]);
        qemu.args(["-device", "nvme,serial=soso,drive=nvme0"]);
    }
}

pub(crate) fn apply_qemu_nic(qemu: &mut Command) {
    apply_qemu_nic_with_ports(qemu, 2222, 7777, None);
}

/// NIC slirp con reenvío SSH/echo configurables (tests en paralelo).
pub(crate) fn apply_qemu_nic_with_ports(
    qemu: &mut Command,
    ssh_port: u16,
    echo_port: u16,
    mac: Option<&str>,
) {
    let mac = mac
        .map(String::from)
        .or_else(|| std::env::var("SOSO_QEMU_MAC").ok())
        .unwrap_or_else(|| "52:54:00:12:34:15".into());
    match qemu_nic().to_ascii_lowercase().as_str() {
        s if s.starts_with("vfio:") => {
            let bdf = s.strip_prefix("vfio:").unwrap_or("");
            println!("xtask: NIC VFIO passthrough {bdf} (sin slirp)");
            qemu.args(["-device", &format!("vfio-pci,host={bdf}")]);
        }
        "e1000e" | "e1000" => {
            qemu.args([
                "-netdev",
                &format!(
                    "user,id=net0,hostfwd=tcp::{echo_port}-:7,hostfwd=tcp::{ssh_port}-:22"
                ),
            ]);
            qemu.args(["-device", &format!("e1000e,netdev=net0,mac={mac}")]);
        }
        "lx-e1000e" | "lx_e1000e" => {
            qemu.args([
                "-netdev",
                &format!(
                    "user,id=net0,hostfwd=tcp::{echo_port}-:7,hostfwd=tcp::{ssh_port}-:22"
                ),
            ]);
            qemu.args(["-device", &format!("e1000e,netdev=net0,mac={mac}")]);
        }
        _ => {
            qemu.args([
                "-netdev",
                &format!(
                    "user,id=net0,hostfwd=tcp::{echo_port}-:7,hostfwd=tcp::{ssh_port}-:22"
                ),
            ]);
            qemu.args(["-device", &format!("virtio-net-pci,netdev=net0,mac={mac}")]);
        }
    }
}

/// Controlador xHCI y dispositivos USB (storage live, teclado, passthrough, trazas).
pub(crate) fn apply_qemu_usb(qemu: &mut Command) {
    let live_storage = qemu_live_usb() && qemu_live();
    let host = qemu_usb_host();
    let kbd = qemu_usb_kbd();
    let hub = qemu_usb_hub();
    let need_xhci = live_storage || kbd || host.is_some();

    if need_xhci {
        let model = qemu_xhci_model().to_ascii_lowercase();
        let dev = match model.as_str() {
            "nec" | "nec-usb-xhci" => "nec-usb-xhci,id=xhci",
            _ => "qemu-xhci,id=xhci",
        };
        qemu.args(["-device", dev]);
        println!("xtask: xHCI ({model})");

        let (storage_bus, storage_port) = if hub {
            qemu.args(["-device", "usb-hub,bus=xhci.0,port=1"]);
            println!("xtask: usb-hub en puerto 1 de xHCI");
            ("xhci.0", "1.1")
        } else {
            ("xhci.0", "1")
        };

        if live_storage {
            qemu.args([
                "-device",
                &format!("usb-storage,bus={storage_bus},port={storage_port},drive=live0"),
            ]);
            let live = package_live::live_image_path();
            println!("xtask: usb-storage BOT → {}", live.display());
        }
        if kbd {
            let kbd_port = if hub { "1.2" } else { "2" };
            qemu.args([
                "-device",
                &format!("usb-kbd,bus={storage_bus},port={kbd_port}"),
            ]);
            println!(
                "xtask: usb-kbd en bus xHCI{}",
                if hub { " (hub puerto 2)" } else { "" }
            );
        }
        if let Some((vid, pid)) = host {
            qemu.args([
                "-device",
                &format!(
                    "usb-host,bus=xhci.0,vendorid=0x{vid:04x},productid=0x{pid:04x}"
                ),
            ]);
            println!("xtask: usb-host passthrough {vid:04x}:{pid:04x} (requiere /dev/bus/usb)");
        }
    }

    if qemu_trace_usb() {
        let trace_log = project_root().join("target/qemu-usb-trace.log");
        if let Some(parent) = trace_log.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        qemu.args([
            "-trace",
            "usb_xhci_*",
            "-D",
            &trace_log.display().to_string(),
        ]);
        println!("xtask: trazas USB → {}", trace_log.display());
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

/// `SOSO_QEMU_AUDIO=1` añade Intel HDA + duplex para captura en QEMU.
pub(crate) fn apply_qemu_audio(qemu: &mut Command) {
    if std::env::var("SOSO_QEMU_AUDIO").ok().as_deref() != Some("1") {
        return;
    }
    let backend = std::env::var("SOSO_QEMU_AUDIO_BACKEND").unwrap_or_else(|_| "pa".into());
    qemu.args([
        "-audiodev",
        &format!("{backend},id=snd0"),
        "-device",
        "intel-hda",
        "-device",
        "hda-duplex,audiodev=snd0",
    ]);
    println!("xtask: audio QEMU (intel-hda + hda-duplex, backend={backend})");
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

pub(crate) fn run_qemu(img: &Path, gdb: bool) {
    build_user();
    let (data, models) = mkfs(false);
    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-machine", "q35"])
        // -cpu max: expone RDRAND, que la cripto de sunset (getrandom con
        // backend rdrand) necesita; la CPU por defecto de QEMU no lo trae.
        .args(["-cpu", "max"])
        .args(["-m", &qemu_mem()])
        .args(["-smp", &qemu_smp()]);
    apply_qemu_accel(&mut qemu);
    apply_firmware(&mut qemu, img);
    qemu.args(["-drive", &format!("format=raw,file={}", img.display())]);
    apply_qemu_disks(&mut qemu, &data, &models);
    apply_qemu_usb(&mut qemu);
    apply_qemu_nic(&mut qemu);
    apply_qemu_gpu(&mut qemu);
    apply_qemu_audio(&mut qemu);
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
