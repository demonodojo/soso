//! `cargo xtask flash-usb-live <usb> [--yes] [--skip-models] [--only …]`
//!
//! Graba `soso-live.img` en un pendrive y estira p3 (sosomfs). p4 SOSOINSTALL
//! ya va en la imagen (FAT entre rootfs y modelos), para que Linux la monte.
//!
//! Con `--skip-models` o `--only` actualiza solo ESP y/o rootfs sin tocar p3.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

use crate::install_disk;
use crate::package_live;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlashPart {
    Kernel,
    Rootfs,
}

pub fn run(args: &[String]) {
    crate::as_user::restore_invoking_env();
    let parsed = parse_args(args);
    let Some(ref usb) = parsed.device else {
        usage();
    };

    let confirm = if parsed.incremental {
        incremental_confirm(&parsed.parts)
    } else {
        install_disk::DeviceConfirm::WipeDisk
    };
    let write_only = crate::as_user::device_io_phase();

    if !crate::as_user::is_root() {
        install_disk::validate_device(&usb, parsed.yes, None, confirm, false);
        prepare(&usb, &parsed);
        confirmar_sudo();
        crate::as_user::exec_elevated();
    }

    crate::as_user::restore_invoking_env();
    if let Err(e) = install_disk::unmount_partitions(&usb) {
        eprintln!("flash-usb-live: {e}");
        exit(1);
    }
    install_disk::validate_device(&usb, parsed.yes, None, confirm, true);
    if !write_only {
        prepare(&usb, &parsed);
    }
    write_usb(&usb, &parsed);
}

/// Pide confirmación justo antes de la contraseña de sudo. El empaquetado
/// tarda minutos y, si la contraseña llega sin nadie delante, sudo se cierra
/// por timeout. Sin TTY (no hay quien responda) no pregunta.
fn confirmar_sudo() {
    use std::io::{BufRead, IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return;
    }
    print!("\nflash-usb-live: el empaquetado ha terminado. ¿Estás listo para escribir la contraseña de sudo? [s/N] ");
    let _ = std::io::stdout().flush();
    let mut linea = String::new();
    let _ = std::io::stdin().lock().read_line(&mut linea);
    if !matches!(linea.trim().to_lowercase().as_str(), "s" | "si" | "sí" | "y" | "yes") {
        eprintln!("flash-usb-live: cancelado antes de grabar; no se ha escrito nada en el pendrive.");
        exit(1);
    }
}

fn prepare(usb: &Path, parsed: &ParsedArgs) {
    if parsed.incremental {
        prepare_incremental(usb, &parsed.parts);
    } else {
        prepare_full(usb);
    }
}

fn write_usb(usb: &Path, parsed: &ParsedArgs) {
    if parsed.incremental {
        write_incremental(usb, &parsed.parts);
    } else {
        write_full(usb);
    }
}

struct ParsedArgs {
    device: Option<PathBuf>,
    yes: bool,
    incremental: bool,
    parts: Vec<FlashPart>,
}

fn parse_args(args: &[String]) -> ParsedArgs {
    let mut device = None;
    let mut yes = false;
    let mut skip_models = false;
    let mut only: Option<Vec<FlashPart>> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--yes" | "-y" => yes = true,
            "--skip-models" => skip_models = true,
            "--only" => {
                i += 1;
                let spec = args.get(i).unwrap_or_else(|| {
                    eprintln!("flash-usb-live: --only requiere kernel,rootfs o ambos");
                    usage();
                });
                only = Some(parse_only_spec(spec));
            }
            s if s.starts_with("--only=") => {
                only = Some(parse_only_spec(s.trim_start_matches("--only=")));
            }
            s if s.starts_with('-') => {
                eprintln!("flash-usb-live: opción desconocida: {s}");
                usage();
            }
            s => {
                if device.is_some() {
                    eprintln!("flash-usb-live: demasiados argumentos");
                    usage();
                }
                device = Some(PathBuf::from(s));
            }
        }
        i += 1;
    }

    let incremental = skip_models || only.is_some();

    let parts = if let Some(p) = only {
        p
    } else if skip_models {
        vec![FlashPart::Kernel, FlashPart::Rootfs]
    } else {
        Vec::new()
    };

    if incremental && parts.is_empty() {
        eprintln!("flash-usb-live: --only vacío");
        usage();
    }

    ParsedArgs {
        device,
        yes,
        incremental,
        parts,
    }
}

fn incremental_confirm(parts: &[FlashPart]) -> install_disk::DeviceConfirm {
    let kernel = parts.contains(&FlashPart::Kernel);
    let rootfs = parts.contains(&FlashPart::Rootfs);
    match (kernel, rootfs) {
        (true, false) => install_disk::DeviceConfirm::LiveUpdateEsp,
        (false, true) => install_disk::DeviceConfirm::LiveUpdateRootfs,
        (true, true) => install_disk::DeviceConfirm::LiveUpdateEspRootfs,
        (false, false) => unreachable!("incremental flash with empty parts"),
    }
}

fn parse_only_spec(spec: &str) -> Vec<FlashPart> {
    let mut parts = Vec::new();
    for token in spec.split(',') {
        match token.trim().to_ascii_lowercase().as_str() {
            "kernel" | "esp" => {
                if !parts.contains(&FlashPart::Kernel) {
                    parts.push(FlashPart::Kernel);
                }
            }
            "rootfs" | "data" => {
                if !parts.contains(&FlashPart::Rootfs) {
                    parts.push(FlashPart::Rootfs);
                }
            }
            "" => {}
            other => {
                eprintln!("flash-usb-live: componente desconocido en --only: {other}");
                usage();
            }
        }
    }
    if parts.is_empty() {
        eprintln!("flash-usb-live: --only requiere kernel, rootfs o ambos");
        usage();
    }
    parts
}

fn prepare_full(usb: &Path) {
    let disk_bytes = blockdev_bytes(usb).unwrap_or_else(|| {
        eprintln!("flash-usb-live: no pude leer el tamaño de {}", usb.display());
        exit(1);
    });
    println!(
        "flash-usb-live: pendrive {} ({})",
        usb.display(),
        crate::live_models::format_bytes(disk_bytes)
    );

    super::build_user();
    let model_pick = crate::live_models::model_pick_bytes_for_flash(disk_bytes);
    if model_pick.is_none() {
        println!(
            "flash-usb-live: modelo demo {} (escalado por USB: SOSO_LIVE_AUTO_MODEL=1 o SOSO_LIVE_CAPACITY=…)",
            crate::live_models::default_live_spec().name
        );
    }
    package_live::run_with_capacity_measured(model_pick, Some(disk_bytes));
}

fn write_full(usb: &Path) {
    let live = package_live::live_image_path();
    let out_dir = package_live::out_dir();

    let wifi_backup = package_live::validate_live_usb(usb)
        .ok()
        .and_then(|_| package_live::read_esp_wificonf(usb))
        .filter(|data| package_live::wificonf_tiene_ssid(data));
    if wifi_backup.is_some() {
        println!("flash-usb-live: conservaré SOSOWIFI.TXT tras el dd");
    }

    println!("flash-usb-live: escribiendo {} → {}", live.display(), usb.display());
    install_disk::run_cmd(
        Command::new("dd")
            .arg(format!("if={}", live.display()))
            .arg(format!("of={}", usb.display()))
            .args([
                "bs=1M",
                "status=progress",
                "iflag=direct",
                "oflag=direct",
                "conv=fsync",
            ]),
        "dd",
    );
    crate::fat32_write::drop_path_cache(usb);
    install_disk::run_cmd(&mut Command::new("sync"), "sync");

    if let Some(data) = wifi_backup.as_deref() {
        match package_live::write_esp_wificonf(usb, data) {
            Ok(()) => println!("flash-usb-live: SOSOWIFI.TXT restaurado"),
            Err(e) => eprintln!("flash-usb-live: aviso: no restauré SOSOWIFI.TXT: {e}"),
        }
        crate::fat32_write::drop_path_cache(usb);
        install_disk::run_cmd(&mut Command::new("sync"), "sync");
    }

    install_disk::expand_models_partition(usb, &live);
    install_disk::repair_gpt_backup(usb);

    print_flash_summary(usb, &out_dir);
    install_disk::release_removable(usb);
}

fn prepare_incremental(_usb: &Path, parts: &[FlashPart]) {
    // La GPT de /dev/sdX no se lee aquí: sin root `sgdisk -v` falla y parece
    // un stick vacío. La comprobación va en write_incremental, ya elevado.
    let profile = package_live::live_driver_profile();
    let want_kernel = parts.contains(&FlashPart::Kernel);
    let want_rootfs = parts.contains(&FlashPart::Rootfs);

    if want_kernel {
        println!("flash-usb-live: compilando kernel (ESP)…");
        // Live no mete SOSOKRN.BIN en soso-uefi.img (64 MiB × fatfs en RAM).
        let _ = super::build_image_with_profile(&profile, false);
    }
    if want_rootfs {
        println!("flash-usb-live: empaquetando rootfs…");
        super::build_user();
        let _ = super::mkfs_rootfs_with_profile(true, &profile, super::RootfsImgMode::PackOnly);
    }
}

fn write_incremental(usb: &Path, parts: &[FlashPart]) {
    if let Err(e) = package_live::validate_live_usb(usb) {
        eprintln!("flash-usb-live: {e}");
        exit(1);
    }

    let root = super::project_root();
    let uefi = root.join("target/soso-uefi.img");
    let data = root.join("target/soso-data.img");
    if let Err(e) = write_incremental_parts(usb, &uefi, &data, parts) {
        eprintln!("flash-usb-live: {e}");
        exit(1);
    }

    install_disk::run_cmd(&mut Command::new("sync"), "sync");

    let labels: Vec<_> = parts
        .iter()
        .map(|p| match p {
            FlashPart::Kernel => "ESP",
            FlashPart::Rootfs => "rootfs",
        })
        .collect();
    println!(
        "\n✅ USB live actualizado ({}) en {} — modelos (p3) intactos",
        labels.join(" + "),
        usb.display()
    );
    install_disk::release_removable(usb);
}

/// Escribe en el pendrive las partes pedidas, pero sólo después de comprobar
/// que todas se pueden escribir. Antes el kernel iba a la ESP y luego el
/// rootfs no cabía en p2: el pendrive quedaba con kernel nuevo y rootfs viejo,
/// e `init` moría al arrancar (docs/DIAGNOSTICO-ROG-2026-11-04.md).
fn write_incremental_parts(
    usb: &Path,
    uefi: &Path,
    data: &Path,
    parts: &[FlashPart],
) -> Result<(), String> {
    let want_kernel = parts.contains(&FlashPart::Kernel);
    let want_rootfs = parts.contains(&FlashPart::Rootfs);

    if want_kernel && !uefi.exists() {
        return Err(format!("falta {}", uefi.display()));
    }
    if want_rootfs {
        if !data.exists() {
            return Err(format!("falta {}", data.display()));
        }
        package_live::check_fits_partition(data, usb, 2)
            .map_err(|e| format!("{e}; no he escrito nada en el pendrive"))?;
    }

    if want_kernel {
        println!(
            "flash-usb-live: actualizando kernel in situ desde {}…",
            uefi.display()
        );
        package_live::update_esp_from_uefi(uefi, usb).map_err(|e| {
            format!(
                "{e}\nflash-usb-live: no pude actualizar el kernel in situ (¿ESP \
                 sin kernel-x86_64 / KERNEL~1?). Un dd de soso-uefi.img p1 \
                 (~31 MiB) sobre una ESP live más grande la deja muda. \
                 Recupera con un flash completo (sin --only)."
            )
        })?;
        crate::fat32_write::drop_path_cache(usb);
    }

    if want_rootfs {
        println!(
            "flash-usb-live: actualizando rootfs (p2) desde {}…",
            data.display()
        );
        package_live::dd_to_partition(data, usb, 2)?;
        crate::fat32_write::drop_path_cache(usb);
    }
    Ok(())
}

fn usage() -> ! {
    eprintln!(
        "uso: cargo xtask flash-usb-live <usb> [--yes] [--skip-models] [--only PARTS]\n\
         \n\
         Ejemplo (flash completo):\n\
           lsblk\n\
           cargo xtask flash-usb-live /dev/sde --yes\n\
         \n\
         Actualización incremental (sin reescribir modelos):\n\
           cargo xtask flash-usb-live /dev/sde --yes --skip-models\n\
           cargo xtask flash-usb-live /dev/sde --yes --only kernel\n\
           cargo xtask flash-usb-live /dev/sde --yes --only rootfs\n\
         \n\
         Graba soso-live.img (modelo según tamaño del stick), estira p3 (modelos).\n\
         p4 SOSOINSTALL (install-soso.sh) va en la imagen, tras el rootfs.\n\
         xtask pide sudo solo para escribir el disco (no hace falta PATH/HOME).\n\
         \n\
         Sin descargar modelos (el mayor ya en target/*-model/ que quepa):\n\
           SOSO_LIVE_OFFLINE=1 cargo xtask flash-usb-live /dev/sde --yes"
    );
    exit(2);
}

fn blockdev_sectors(dev: &Path) -> Option<u64> {
    let out = Command::new("blockdev")
        .args(["--getsz"])
        .arg(dev)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .ok()
}

fn parse_first_u64(stdout: &str) -> Option<u64> {
    stdout.lines().find_map(|l| {
        let t = l.trim();
        if t.is_empty() {
            None
        } else {
            t.parse().ok()
        }
    })
}

fn sysfs_block_bytes(dev: &Path) -> Option<u64> {
    let name = dev.file_name()?.to_str()?;
    let sectors: u64 = std::fs::read_to_string(format!("/sys/block/{name}/size"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(sectors.saturating_mul(512))
}

fn blockdev_bytes(dev: &Path) -> Option<u64> {
    if let Some(s) = blockdev_sectors(dev) {
        return Some(s.saturating_mul(512));
    }
    // `-d`: solo el disco; sin eso SIZE incluye particiones y no parsea.
    let out = Command::new("lsblk")
        .args(["-bdno", "SIZE"])
        .arg(dev)
        .output()
        .ok();
    if let Some(out) = out {
        if out.status.success() {
            if let Some(n) = parse_first_u64(&String::from_utf8_lossy(&out.stdout)) {
                return Some(n);
            }
        }
    }
    sysfs_block_bytes(dev)
}

fn print_flash_summary(usb: &Path, out_dir: &Path) {
    println!("\n✅ USB live listo en {}", usb.display());
    println!(
        "\n1) Probar soso: UEFI → arrancar desde USB (F12).\n\
         2) Instalar en un NVMe dedicado (desde el propio soso live, sin Linux):\n\
            soso-install\n\
            soso-install nvme1 --yes\n\
            reinicia con el USB puesto → quita el USB → «soso» en el menú UEFI\n\
         \n\
         Plan B (desde Linux, USB conectado):\n\
            lsblk\n\
            sudo {}/install-soso.sh /dev/nvme1n1 --yes\n\
         \n\
         Artefactos también en {}",
        out_dir.display(),
        out_dir.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Seek, Write};
    use std::process::Command;

    fn require_sgdisk() -> bool {
        Command::new("sgdisk")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn require_mkfs_vfat() -> bool {
        // `-h` sale con 1 en dosfstools 4.x: el test se saltaba siempre.
        Command::new("mkfs.vfat")
            .arg("--help")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn format_p1_vfat(img: &Path) {
        let (first, last) = package_live::partition_range(img, 1).unwrap();
        let sectors = last - first + 1;
        let fat = img.with_extension("p1fat");
        {
            let f = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&fat)
                .unwrap();
            f.set_len(sectors * 512).unwrap();
        }
        package_live::run_cmd(
            Command::new("mkfs.vfat").args(["-F", "32", "-n", "KERNEL"]).arg(&fat),
            "mkfs.vfat p1",
        );
        package_live::run_cmd(
            Command::new("dd")
                .arg(format!("if={}", fat.display()))
                .arg(format!("of={}", img.display()))
                .args(["bs=512", "conv=notrunc"])
                .arg(format!("seek={first}")),
            "dd fat→p1",
        );
        let _ = fs::remove_file(&fat);
    }

    fn create_test_live_img(path: &Path, p1_mb: u64, p2_mb: u64, p3_mb: u64) {
        let total_mb = 1 + p1_mb + p2_mb + p3_mb + 8;
        {
            let f = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
                .unwrap();
            f.set_len(total_mb * 1024 * 1024).unwrap();
        }
        package_live::run_cmd(
            Command::new("sgdisk").args(["-o", "-a", "1"]).arg(path),
            "sgdisk -o",
        );
        package_live::run_cmd(
            Command::new("sgdisk")
                .arg("-n")
                .arg(format!("1:2048:+{p1_mb}M"))
                .arg("-t")
                .arg("1:EF00")
                .arg("-n")
                .arg(format!("2:0:+{p2_mb}M"))
                .arg("-t")
                .arg("2:8300")
                .arg("-n")
                .arg(format!("3:0:+{p3_mb}M"))
                .arg("-t")
                .arg("3:8300")
                .arg(path),
            "sgdisk parts",
        );
    }

    fn write_pattern_to_p3(img: &Path, byte: u8) {
        let (first, last) = package_live::partition_range(img, 3).unwrap();
        let count = last - first + 1;
        let mut data = vec![byte; (count * 512) as usize];
        data[0] = byte.wrapping_add(1);
        let mut f = fs::OpenOptions::new().write(true).open(img).unwrap();
        f.seek(std::io::SeekFrom::Start(first * 512)).unwrap();
        f.write_all(&data).unwrap();
    }

    fn read_p3_first_byte(img: &Path) -> u8 {
        let (first, _) = package_live::partition_range(img, 3).unwrap();
        let mut b = [0u8; 1];
        let mut f = fs::OpenOptions::new().read(true).open(img).unwrap();
        f.seek(std::io::SeekFrom::Start(first * 512)).unwrap();
        f.read_exact(&mut b).unwrap();
        b[0]
    }

    fn minimal_fat_img(path: &Path, size_mb: u64) {
        let total_mb = size_mb + 16;
        {
            let f = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
                .unwrap();
            f.set_len(total_mb * 1024 * 1024).unwrap();
        }
        package_live::run_cmd(
            Command::new("sgdisk").args(["-o", "-a", "1"]).arg(path),
            "sgdisk -o fat",
        );
        package_live::run_cmd(
            Command::new("sgdisk")
                .arg("-n")
                .arg(format!("1:2048:+{size_mb}M"))
                .arg("-t")
                .arg("1:EF00")
                .arg(path),
            "sgdisk p1",
        );
        let p1 = package_live::partition_first_sector(path, 1).unwrap();
        let p1_end = package_live::partition_range(path, 1).unwrap().1;
        let sectors = p1_end - p1 + 1;
        let mut f = fs::OpenOptions::new().write(true).open(path).unwrap();
        f.seek(std::io::SeekFrom::Start(p1 * 512)).unwrap();
        let mut boot = [0u8; 512];
        boot[510] = 0x55;
        boot[511] = 0xAA;
        boot[11..13].copy_from_slice(&512u16.to_le_bytes());
        boot[13] = 1;
        boot[16] = 2;
        boot[17..19].copy_from_slice(&512u16.to_le_bytes());
        boot[22..24].copy_from_slice(&1u16.to_le_bytes());
        f.write_all(&boot).unwrap();
        f.write_all(&vec![0u8; 512 * 2]).unwrap();
        f.write_all(&vec![0u8; 512 * 32]).unwrap();
        let rest = sectors.saturating_sub(1 + 2 + 32);
        if rest > 0 {
            f.write_all(&vec![0u8; (rest * 512) as usize]).unwrap();
        }
    }

    #[test]
    fn incremental_p1_preserves_p3() {
        if !require_sgdisk() {
            eprintln!("incremental_p1_preserves_p3: sin sgdisk, omito");
            return;
        }
        let dir = std::env::temp_dir().join(format!("soso-flash-inc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let usb = dir.join("usb.img");
        let src = dir.join("esp.img");
        create_test_live_img(&usb, 32, 64, 128);
        write_pattern_to_p3(&usb, 0xAB);
        minimal_fat_img(&src, 32);

        package_live::validate_live_usb(&usb).unwrap();
        package_live::dd_partition(&src, 1, &usb, 1).unwrap();
        package_live::create_esp_slots(&usb);

        assert_eq!(read_p3_first_byte(&usb), 0xAC);
        let p1 = package_live::partition_first_sector(&usb, 1).unwrap();
        let mut drv = [0u8; 11];
        drv[..8].copy_from_slice(b"SOSODRV ");
        drv[8..11].copy_from_slice(b"TXT");
        assert!(
            crate::fat32_write::find_root_entry(&usb, p1, &drv)
                .unwrap()
                .is_some(),
            "hueco ESP SOSODRV no creado"
        );
    }

    #[test]
    fn dd_partition_rejects_smaller_dest() {
        if !require_sgdisk() {
            eprintln!("dd_partition_rejects_smaller_dest: sin sgdisk, omito");
            return;
        }
        let dir = std::env::temp_dir().join(format!("soso-flash-rej-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src = dir.join("big.img");
        let dst = dir.join("small.img");
        create_test_live_img(&src, 64, 32, 32);
        create_test_live_img(&dst, 32, 32, 32);

        let err = package_live::dd_partition(&src, 1, &dst, 1).unwrap_err();
        assert!(err.contains("más pequeña"), "{err}");
    }

    #[test]
    fn only_kernel_in_place_preserves_96m_esp() {
        if !require_sgdisk() || !require_mkfs_vfat() {
            eprintln!("only_kernel_in_place_preserves_96m_esp: sin sgdisk/mkfs.vfat, omito");
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "soso-flash-inplace-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let dest = dir.join("live96.img");
        let src = dir.join("uefi31.img");
        create_test_live_img(&dest, 96, 8, 8);
        create_test_live_img(&src, 31, 8, 8);
        format_p1_vfat(&dest);
        format_p1_vfat(&src);

        let dest_p1 = package_live::partition_first_sector(&dest, 1).unwrap();
        let src_p1 = package_live::partition_first_sector(&src, 1).unwrap();
        let bpb_before = crate::fat32_write::bpb_total_sectors(&dest, dest_p1).unwrap();
        assert!(
            bpb_before > 90 * 2048,
            "BPB destino debería ser ~96 MiB, fue {bpb_before} sectores"
        );

        let mut old_kernel = vec![0x7f, b'E', b'L', b'F'];
        old_kernel.resize(64 * 1024, 0xCC);
        let mut new_kernel = vec![0x7f, b'E', b'L', b'F'];
        new_kernel.resize(32 * 1024, 0xDD);
        let solog = vec![0xAAu8; 256 * 1024];
        let sokrn = vec![0xBBu8; 64 * 1024];

        crate::fat32_write::write_root_file(
            &dest,
            dest_p1,
            b"KERNEL~1",
            b"   ",
            &old_kernel,
        )
        .unwrap();
        crate::fat32_write::write_root_file(&dest, dest_p1, b"SOSOLOG ", b"TXT", &solog)
            .unwrap();
        crate::fat32_write::write_root_file(&dest, dest_p1, b"SOSOKRN ", b"BIN", &sokrn)
            .unwrap();
        crate::fat32_write::write_root_file(&src, src_p1, b"KERNEL~1", b"   ", &new_kernel)
            .unwrap();

        package_live::update_esp_from_uefi(&src, &dest).unwrap();

        let bpb_after = crate::fat32_write::bpb_total_sectors(&dest, dest_p1).unwrap();
        assert_eq!(bpb_before, bpb_after, "el BPB de 96 MiB no debe cambiar");

        let log = crate::fat32_write::read_root_file(&dest, dest_p1, b"SOSOLOG TXT").unwrap();
        assert!(log.iter().all(|&b| b == 0xAA), "SOSOLOG pisado");
        let krn = crate::fat32_write::read_root_file(&dest, dest_p1, b"SOSOKRN BIN").unwrap();
        assert!(krn.iter().all(|&b| b == 0xBB), "SOSOKRN pisado");

        let kern = crate::fat32_write::read_root_file(&dest, dest_p1, &crate::fat32_write::KERNEL_8_3)
            .unwrap();
        assert_eq!(&kern[..4], b"\x7fELF");
        assert_eq!(kern[4], 0xDD, "el kernel no se actualizó");
        assert!(kern[32 * 1024..].iter().all(|&b| b == 0), "hueco no rellenado");

        let _ = fs::remove_dir_all(&dir);
    }

    /// Pendrive live de prueba (p2 de `p2_mb`) con un kernel viejo (0xCC) en la
    /// ESP, y una imagen UEFI con un kernel nuevo (0xDD).
    fn live_con_kernel_viejo(dir: &Path, p2_mb: u64) -> (PathBuf, PathBuf) {
        let usb = dir.join("usb.img");
        let uefi = dir.join("uefi.img");
        create_test_live_img(&usb, 40, p2_mb, 8);
        create_test_live_img(&uefi, 40, 8, 8);
        format_p1_vfat(&usb);
        format_p1_vfat(&uefi);
        let usb_p1 = package_live::partition_first_sector(&usb, 1).unwrap();
        let uefi_p1 = package_live::partition_first_sector(&uefi, 1).unwrap();
        let mut viejo = vec![0x7f, b'E', b'L', b'F'];
        viejo.resize(64 * 1024, 0xCC);
        let mut nuevo = vec![0x7f, b'E', b'L', b'F'];
        nuevo.resize(32 * 1024, 0xDD);
        crate::fat32_write::write_root_file(&usb, usb_p1, b"KERNEL~1", b"   ", &viejo).unwrap();
        crate::fat32_write::write_root_file(&uefi, uefi_p1, b"KERNEL~1", b"   ", &nuevo)
            .unwrap();
        (usb, uefi)
    }

    fn byte_del_kernel(usb: &Path) -> u8 {
        let p1 = package_live::partition_first_sector(usb, 1).unwrap();
        let k = crate::fat32_write::read_root_file(usb, p1, &crate::fat32_write::KERNEL_8_3)
            .unwrap();
        k[4]
    }

    fn rootfs_de(dir: &Path, mb: u64) -> PathBuf {
        let data = dir.join("soso-data.img");
        fs::write(&data, vec![0x5Au8; (mb * 1024 * 1024) as usize]).unwrap();
        data
    }

    #[test]
    fn skip_models_sin_sitio_para_el_rootfs_no_toca_la_esp() {
        if !require_sgdisk() || !require_mkfs_vfat() {
            eprintln!("skip_models_sin_sitio_para_el_rootfs_no_toca_la_esp: sin sgdisk/mkfs.vfat, omito");
            return;
        }
        let dir = std::env::temp_dir().join(format!("soso-flash-nocabe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let (usb, uefi) = live_con_kernel_viejo(&dir, 8);
        let data = rootfs_de(&dir, 16);
        let antes = fs::read(&usb).unwrap();

        let err = write_incremental_parts(
            &usb,
            &uefi,
            &data,
            &[FlashPart::Kernel, FlashPart::Rootfs],
        )
        .unwrap_err();
        assert!(err.contains("no cabe"), "{err}");
        assert!(err.contains("no he escrito nada"), "{err}");
        assert_eq!(byte_del_kernel(&usb), 0xCC, "el kernel viejo se pisó");
        assert!(fs::read(&usb).unwrap() == antes, "el pendrive cambió");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skip_models_con_sitio_escribe_kernel_y_rootfs() {
        if !require_sgdisk() || !require_mkfs_vfat() {
            eprintln!("skip_models_con_sitio_escribe_kernel_y_rootfs: sin sgdisk/mkfs.vfat, omito");
            return;
        }
        let dir = std::env::temp_dir().join(format!("soso-flash-cabe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let (usb, uefi) = live_con_kernel_viejo(&dir, 16);
        let data = rootfs_de(&dir, 8);

        write_incremental_parts(&usb, &uefi, &data, &[FlashPart::Kernel, FlashPart::Rootfs])
            .unwrap();
        assert_eq!(byte_del_kernel(&usb), 0xDD, "el kernel no se actualizó");
        let (p2, _) = package_live::partition_range(&usb, 2).unwrap();
        let mut f = fs::File::open(&usb).unwrap();
        f.seek(std::io::SeekFrom::Start(p2 * 512)).unwrap();
        let mut b = vec![0u8; 8 * 1024 * 1024];
        f.read_exact(&mut b).unwrap();
        assert!(b.iter().all(|&x| x == 0x5A), "el rootfs no llegó a p2");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_first_u64_toma_disco_no_particiones() {
        let lsblk = "536870912000\n   201326592\n   402653184\n536199789056\n    67108864\n";
        assert_eq!(parse_first_u64(lsblk), Some(536870912000));
        assert_eq!(parse_first_u64("536870912000\n"), Some(536870912000));
        assert_eq!(parse_first_u64(""), None);
    }

    #[test]
    fn parse_only_spec_accepts_aliases() {
        let parts = parse_only_spec("kernel,rootfs");
        assert_eq!(parts, vec![FlashPart::Kernel, FlashPart::Rootfs]);
        let parts = parse_only_spec("esp,data");
        assert_eq!(parts, vec![FlashPart::Kernel, FlashPart::Rootfs]);
    }

    #[test]
    fn incremental_confirm_matches_parts() {
        assert_eq!(
            incremental_confirm(&[FlashPart::Kernel]),
            install_disk::DeviceConfirm::LiveUpdateEsp
        );
        assert_eq!(
            incremental_confirm(&[FlashPart::Rootfs]),
            install_disk::DeviceConfirm::LiveUpdateRootfs
        );
        assert_eq!(
            incremental_confirm(&[FlashPart::Kernel, FlashPart::Rootfs]),
            install_disk::DeviceConfirm::LiveUpdateEspRootfs
        );
    }

    #[test]
    fn release_removable_ignores_image_file() {
        let path = std::env::temp_dir().join(format!("soso-release-{}", std::process::id()));
        fs::write(&path, b"x").unwrap();
        install_disk::release_removable(&path);
        assert!(path.exists(), "no debe borrar una imagen-fichero");
        let _ = fs::remove_file(&path);
    }
}
