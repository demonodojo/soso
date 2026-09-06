//! `cargo xtask package-usb-live` — imagen GPT única para arranque live (sin tocar NVMe interno).

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

use crate::drivers::{self, DriverProfile};
use crate::live_models;

/// Perfil live: `SOSO_DRIVERS` si está definido; si no, `live-usb` (GPU GA107 incluida).
pub(crate) fn live_driver_profile() -> DriverProfile {
    if std::env::var("SOSO_DRIVERS")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        drivers::profile_from_env_or_args()
    } else {
        drivers::preset_live_usb()
    }
}

pub fn run() {
    let capacity = std::env::var("SOSO_LIVE_CAPACITY")
        .ok()
        .map(|v| {
            live_models::parse_capacity_env(&v).unwrap_or_else(|e| {
                eprintln!("package-usb-live: {e}");
                exit(1);
            })
        });
    run_with_capacity(capacity);
}

/// Empaqueta el live USB. `usb_bytes` fija el presupuesto de modelos (p. ej. tamaño del pendrive).
pub fn run_with_capacity(usb_bytes: Option<u64>) {
    let root = super::project_root();
    let profile = live_driver_profile();
    preflight_gpu_firmware(&root, &profile);
    print_profile_summary(&profile);

    super::build_user();
    let _ = super::build_image_with_profile(&profile, true);

    let uefi = root.join("target/soso-uefi.img");
    let mut data = super::mkfs_rootfs_with_profile(true, &profile);

    let align = 1024 * 1024;
    let data_len = std::fs::metadata(&data).expect("data").len();
    let uefi_len = std::fs::metadata(&uefi).expect("uefi").len();
    let esp_aligned = (uefi_len + align - 1) / align * align;
    let rootfs_aligned = (data_len + align - 1) / align * align;

    let selection = resolve_live_models(&root, usb_bytes, esp_aligned, rootfs_aligned);
    crate::fetch_hf::require_valid_model(&root, &selection.primary_dir);
    if let Some(tiny) = selection.tiny_dir.as_deref() {
        crate::fetch_hf::require_valid_model(&root, tiny);
    }
    let _llm_conf = LiveLlmConf::set_model(&root, &selection.llm_name);
    let models = super::mkfs_models_live_for_dirs(
        true,
        &selection.primary_dir,
        selection.tiny_dir.as_deref(),
    );

    let mut total = live_image_bytes(&data, &models, &uefi);
    for _ in 0..2 {
        write_rootfs_install_meta(&root, total);
        data = super::mkfs_rootfs_with_profile(true, &profile);
        total = live_image_bytes(&data, &models, &uefi);
    }

    let data_len = std::fs::metadata(&data).expect("data").len();
    let models_len = std::fs::metadata(&models).expect("models").len();
    let total = live_image_bytes(&data, &models, &uefi);
    let p2_size = (data_len + align - 1) / align * align;

    let out_dir = root.join("target/usb-live");
    std::fs::create_dir_all(&out_dir).expect("usb-live dir");
    let live = out_dir.join("soso-live.img");

    {
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&live)
            .expect("live img");
        f.set_len(total).expect("truncate live img");
    }

    run_cmd(
        Command::new("dd")
            .arg(format!("if={}", uefi.display()))
            .arg(format!("of={}", live.display()))
            .args(["bs=512", "conv=notrunc"]),
        "dd uefi",
    );

    run_cmd(Command::new("sgdisk").arg("-e").arg(&live), "sgdisk -e");

    let p2_mb = (p2_size / (1024 * 1024)).max(1);
    let p4_mb = (live_models::P4_INSTALL_BYTES / (1024 * 1024)).max(1);
    // p4 SOSOINSTALL va *entre* rootfs y modelos (primeros ~350 MiB del stick).
    // Al final del pendrive Linux no monta: tras dd+sgdisk p4 quedaba en un
    // LBA que al leer volvía ceros (tabla vieja o capacidad inflada).
    run_cmd(
        Command::new("sgdisk")
            .arg("-a")
            .arg("1")
            .arg("-n")
            .arg(format!("2:0:+{p2_mb}M"))
            .arg("-t")
            .arg("2:8300")
            .arg("-n")
            .arg(format!("4:0:+{p4_mb}M"))
            .arg("-t")
            .arg("4:0700")
            .arg("-c")
            .arg("4:SOSOINSTALL")
            .arg("-n")
            .arg("3:0:0")
            .arg("-t")
            .arg("3:8300")
            .arg(&live),
        "sgdisk add",
    );

    let p2_start = partition_first_sector(&live, 2).expect("part2 lba");
    let p3_start = partition_first_sector(&live, 3).expect("part3 lba");
    let p4_start = partition_first_sector(&live, 4).expect("part4 lba");

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

    write_installer_bundle(&out_dir, total);
    let fat = build_install_fat(&out_dir);
    run_cmd(
        Command::new("dd")
            .arg(format!("if={}", fat.display()))
            .arg(format!("of={}", live.display()))
            .arg("bs=512")
            .arg(format!("seek={p4_start}"))
            .arg("conv=notrunc"),
        "dd SOSOINSTALL",
    );

    create_esp_slots(&live);

    write_flash(
        &out_dir,
        &live,
        data_len,
        models_len,
        total,
        usb_bytes,
        &selection,
    );
    write_installer_bundle(&out_dir, total);
    println!("package-usb-live: {}", live.display());
    print_profile_summary(&profile);
    println!("\n✅ Live USB listo en {}", out_dir.display());
}

struct LiveModelSelection {
    primary_dir: PathBuf,
    tiny_dir: Option<PathBuf>,
    llm_name: String,
}

fn resolve_live_models(
    root: &Path,
    usb_bytes: Option<u64>,
    esp_aligned: u64,
    rootfs_aligned: u64,
) -> LiveModelSelection {
    if let Some(custom) = std::env::var_os("SOSO_MODELS_DIR").map(PathBuf::from) {
        if !custom.join("manifest.som").exists() {
            eprintln!(
                "xtask: SOSO_MODELS_DIR={} no contiene manifest.som",
                custom.display()
            );
            exit(1);
        }
        let llm_name = std::env::var("SOSO_LIVE_MODEL")
            .ok()
            .or_else(|| infer_model_name_from_dir(&custom))
            .unwrap_or_else(|| "tinyllama".into());
        println!(
            "package-usb-live: SOSO_MODELS_DIR={} (modelo={llm_name})",
            custom.display()
        );
        return LiveModelSelection {
            primary_dir: custom,
            tiny_dir: None,
            llm_name,
        };
    }

    let spec = if live_models::offline_mode() {
        println!("package-usb-live: SOSO_LIVE_OFFLINE=1 (sin descargas HF)");
        if let Some(usb) = usb_bytes {
            live_models::pick_materialized_for_usb(usb, esp_aligned, rootfs_aligned, root)
        } else {
            live_models::pick_largest_materialized(root)
        }
        .unwrap_or_else(|| {
            let have = live_models::list_materialized(root);
            if have.is_empty() {
                eprintln!(
                    "live-models: SOSO_LIVE_OFFLINE=1 pero no hay modelos en target/*-model/\n\
                     Descarga uno con: cargo xtask fetch-hf <org/repo> --name <nombre> --out target/<nombre>-model"
                );
            } else if usb_bytes.is_some() {
                eprintln!(
                    "live-models: SOSO_LIVE_OFFLINE=1 pero ninguno de [{}] cabe en este USB",
                    have.join(", ")
                );
            } else {
                eprintln!("live-models: SOSO_LIVE_OFFLINE=1 pero no hay modelos materializados");
            }
            exit(1);
        })
    } else if let Some(usb) = usb_bytes {
        println!(
            "package-usb-live: USB {} → presupuesto modelos {}",
            live_models::format_bytes(usb),
            live_models::format_bytes(live_models::models_budget_from_layout(
                usb,
                esp_aligned,
                rootfs_aligned,
            ))
        );
        let chosen = live_models::pick_for_usb(usb, esp_aligned, rootfs_aligned, root);
        println!(
            "package-usb-live: modelo demo {} (need {})",
            chosen.name,
            chosen.format_need(root)
        );
        chosen
    } else {
        let chosen = live_models::default_live_spec();
        println!(
            "package-usb-live: sin capacidad USB → {}",
            chosen.name
        );
        chosen
    };

    if live_models::offline_mode() {
        println!(
            "package-usb-live: modelo offline {} (need {})",
            spec.name,
            spec.format_need(root)
        );
        live_models::require_materialized(root, &spec);
    } else {
        live_models::ensure_materialized(root, &spec);
    }
    let tiny = live_models::ensure_tiny(root);
    LiveModelSelection {
        primary_dir: spec.resolved_dir(root),
        tiny_dir: Some(tiny),
        llm_name: spec.name.to_string(),
    }
}

fn infer_model_name_from_dir(dir: &Path) -> Option<String> {
    let base = dir.file_name()?.to_string_lossy();
    let name = base.strip_suffix("-model").unwrap_or(&base);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn preflight_gpu_firmware(root: &Path, profile: &DriverProfile) {
    let gpu = profile
        .kernel_features
        .iter()
        .any(|f| f == "drv-gpu-nvidia" || f == "drv-all");
    if !gpu {
        return;
    }

    let fw = root.join("rootfs/lib/firmware/nvidia");
    let ga107_bl = fw.join("ga107/gsp/bootloader-570.144.bin");
    let ga102_bl = fw.join("ga102/gsp/bootloader-570.144.bin");
    let gb205_bl = fw.join("gb205/gsp/bootloader-570.144.bin");
    let ga107_acr = fw.join("ga107/acr/ucode_ahesasc.bin");
    let ga102_acr = fw.join("ga102/acr/ucode_ahesasc.bin");

    println!("package-usb-live: preflight firmware NVIDIA (Ampere + Blackwell)");

    if ga107_bl.is_file() {
        println!("   OK    ga107/gsp (RTX 3050 Mobile)");
    } else if ga102_bl.is_file() {
        println!("   WARN  falta ga107/gsp — ga102 presente (fallback válido para RTX 3050 Mobile)");
    } else {
        eprintln!("   WARN  sin blobs GSP Ampere — el stick no servirá en GA10x");
        eprintln!("         ejecuta: ./scripts/l6-pack-firmware.sh");
    }

    if gb205_bl.is_file() {
        println!("   OK    gb205/gsp (RTX 5070 Ti Mobile / Blackwell)");
    } else {
        eprintln!("   WARN  sin blobs GSP gb205 — el stick no servirá en Blackwell");
        eprintln!("         ejecuta: ./scripts/l6-pack-firmware.sh");
    }

    if !ga107_bl.is_file() && !ga102_bl.is_file() && !gb205_bl.is_file() {
        eprintln!("   FAIL  sin firmware NVIDIA en rootfs");
        eprintln!("         ejecuta: ./scripts/l6-pack-firmware.sh");
        exit(1);
    }

    if ga107_acr.is_file() {
        println!("   OK    ga107/acr/ucode_ahesasc.bin");
    } else if ga102_acr.is_file() {
        println!("   WARN  falta ga107/acr — ga102/acr presente (ACR soft-fail posible)");
    } else {
        eprintln!("   WARN  sin ACR Ampere — G3 ola2 soft-fail en bring-up");
        eprintln!("         ejecuta: ./scripts/l6-pack-firmware.sh");
    }
}

fn print_profile_summary(profile: &DriverProfile) {
    let feats = drivers::kernel_feature_args(profile);
    println!(
        "package-usb-live: kernel features=[{}]",
        feats.join(", ")
    );
    if !profile.lxdde_ports.is_empty() {
        println!(
            "package-usb-live: lxdde ports=[{}] mode={:?}",
            profile.lxdde_ports.join(", "),
            profile.lxdde_mode
        );
    }
    if profile.firmware_exclude.is_empty() {
        println!("package-usb-live: firmware exclude=(ninguno)");
    } else {
        println!(
            "package-usb-live: firmware exclude=[{}]",
            profile.firmware_exclude.join(", ")
        );
    }
}

/// Pone `modelo=<name>` en rootfs solo durante el empaquetado live; restaura al salir.
struct LiveLlmConf {
    path: PathBuf,
    saved: Option<String>,
}

impl LiveLlmConf {
    fn set_model(root: &Path, name: &str) -> Self {
        let path = root.join("rootfs/etc/llm.conf");
        let saved = std::fs::read_to_string(&path).ok();
        let mut content = saved.clone().unwrap_or_else(|| {
            "modelo=tiny\nmax=128\ntemp=0.7\ntop_p=0.9\n".into()
        });
        let line = format!("modelo={name}\n");
        if content.contains("modelo=") {
            let mut out = String::new();
            for ln in content.lines() {
                if ln.starts_with("modelo=") {
                    out.push_str(&line);
                } else {
                    out.push_str(ln);
                    out.push('\n');
                }
            }
            content = out;
        } else {
            content.push_str(&line);
        }
        std::fs::write(&path, &content).expect("llm.conf live");
        Self { path, saved }
    }
}

impl Drop for LiveLlmConf {
    fn drop(&mut self) {
        if let Some(ref saved) = self.saved {
            let _ = std::fs::write(&self.path, saved);
        }
    }
}

fn live_image_bytes(data: &Path, models: &Path, uefi: &Path) -> u64 {
    let data_len = std::fs::metadata(data).expect("data").len();
    let models_len = std::fs::metadata(models).expect("models").len();
    let uefi_len = std::fs::metadata(uefi).expect("uefi").len();
    let align = 1024 * 1024;
    let header = (uefi_len + align - 1) / align * align;
    let p2_size = (data_len + align - 1) / align * align;
    let p3_size = (models_len + align - 1) / align * align;
    header + p2_size + live_models::P4_INSTALL_BYTES + p3_size + align
}

fn write_rootfs_install_meta(root: &Path, total: u64) {
    let etc = root.join("rootfs/etc");
    std::fs::create_dir_all(&etc).expect("rootfs/etc");
    std::fs::write(etc.join("soso-live.bytes"), total.to_string()).expect("soso-live.bytes");
    std::fs::write(etc.join("grub-linux.txt"), GRUB_LINUX_TXT).expect("grub-linux.txt");
}

const GRUB_LINUX_TXT: &str = "\
soso-install ya deja el disco arrancable por sí solo: registra una entrada UEFI\n\
«soso» en la NVRAM de la placa (lo hace el shim en el siguiente arranque del\n\
USB). No hace falta Linux para nada.\n\
\n\
Esto es solo el plan B, por si tu firmware ignora las entradas nuevas y prefieres\n\
arrancar soso desde el GRUB de Linux:\n\
\n\
  sudo /media/$USER/SOSOINSTALL/install-soso.sh --grub-only /dev/nvme1n1\n\
\n\
Sustituye /dev/nvme1n1 por el disco donde instalaste soso (lsblk).\n\
Luego reinicia y elige \"soso\" en el menú GRUB.\n\
\n\
Desinstalar: sudo rm /etc/grub.d/41_soso && sudo update-grub\n\
";

fn write_flash(
    out_dir: &Path,
    _live: &Path,
    data_len: u64,
    models_len: u64,
    total: u64,
    usb_bytes: Option<u64>,
    selection: &LiveModelSelection,
) {
    let flash = out_dir.join("FLASH-LIVE.txt");
    let usb_line = usb_bytes
        .map(|b| format!("Medido al flashear: {}.\n", live_models::format_bytes(b)))
        .unwrap_or_default();
    let model_demo = format!(
        "Modelo demo empaquetado: `{}` (Q4_K_M). También `tiny` sintético.\n\
         Escalera automática al flashear: tinyllama → mistral-7b → qwen3.8-27b (32 GB+).",
        selection.llm_name
    );
    let body = format!(
        r#"soso — arranque LIVE desde USB (no modifica el disco interno)

Archivo: soso-live.img ({:.1} GiB)
  Partición 1 — ESP UEFI (kernel)
  Partición 2 — rootfs sosofs (~{:.0} MiB)
  Partición 4 — SOSOINSTALL (FAT, instalador Linux)
  Partición 3 — modelos sosomfs (~{:.1} GiB)
{usb_line}
1) Escribir SOLO en el pendrive (identifica con lsblk, ej. /dev/sdX):

   sudo dd if=soso-live.img of=/dev/sdX bs=4M status=progress conv=fsync

   O con instalador y modelo según tamaño del stick:

   sudo cargo xtask flash-usb-live /dev/sdX --yes

2) UEFI → arrancar una vez desde USB (F12 / Boot menu).

3) Consola serie / GOP: live: GPT … | fs: sosofs live | ssh soso@<ip>

   Trazas de arranque: al volver a Linux, monta la ESP del USB (partición 1):
   - BOOTMARK.TXT — la escribe el shim UEFI; si sigue vacía, el firmware nunca
     llegó a ejecutar nuestro loader (Secure Boot, orden de arranque…).
   - SOSOLOG.TXT (256 KiB) — log del kernel; vacío con BOOTMARK escrita = el
     kernel se colgó o no detectó el USB (mira los checkpoints «boot:» en
     pantalla).
   - SOSOWIFI.TXT (4 KiB) — ssid=/psk= editables en la ESP antes de arrancar
   - SOSOBOOT.TXT — buzón entre soso-install y el shim; dice si la entrada de
     arranque UEFI se llegó a registrar y con qué número.

4) Instalar en disco interno **desde soso live** (sin pasar por Linux):

   soso-install                 # enseña discos, uso, y pide en cuál instalar
   soso-install nvme1 --yes     # o elige por nombre / id

   Reinicia con el USB puesto: el shim registra la entrada UEFI «soso».
   Después quita el USB y elige «soso» en el menú de arranque de la placa.

5) Instalar desde Linux (sin arrancar soso):

   sudo ./install-soso.sh /dev/nvme1n1 --yes

6) Flashear USB con instalador: sudo cargo xtask flash-usb-live /dev/sdX --yes
   (mide el pendrive, elige el mejor modelo GGUF llama que quepa, estira p3 al
   sobrante del stick; p4 SOSOINSTALL va en la imagen, tras el rootfs)

7) Apagar, quitar USB, arrancar disco habitual → Linux intacto (si no instalaste).

Prueba GPU (mismo stick, varias placas)
---------------------------------------
El live lleva nouveau + firmware Ampere (ga107/ga102) y Blackwell (gb205).
El bring-up elige el juego según el PCI id / boot0; no hay que reflashear.

  Ampere  10de:249c (RTX 3050 Mobile):
    nvidia: GPU 10de:249c NV_PMC_BOOT_0=0x........   # ≠ ffffffff
    nouveau-lx: familia=ga107  fw nvidia/ga107/gsp/…  (o fallback ga102)

  Blackwell 10de:2f18 (RTX 5070 Ti Mobile):
    nouveau-lx: familia=Blackwell  fw nvidia/gb205/gsp/…

Criterio G1 mínimo: BAR0 vivo (boot0 distinto de 0xffffffff).
Si sigue muerto: «fuera del bus / sin D0» — la dGPU puede estar en D3cold;
no uses VFIO en live (solo arranque directo desde USB).

Trazas: ESP part1 → SOSOLOG.TXT; kshell → hwscan (gpu-nvidia / lx-nouveau).
SSH: ssh -i target/soso_test_key soso@<ip>

Demo LLM
--------
{model_demo}

  ask
  soso-llm run {llm} --prompt "hola" --max 32

Simular capacidad sin pendrive: SOSO_LIVE_CAPACITY=64G cargo xtask package-usb-live
Sin descargas HF (el mayor ya materializado que quepa): SOSO_LIVE_OFFLINE=1 cargo xtask flash-usb-live /dev/sdX --yes
Override de modelo: SOSO_MODELS_DIR=/ruta/al/modelo cargo xtask package-usb-live

QEMU: SOSO_QEMU_LIVE=1 cargo xtask run
      (QEMU no tiene GPU NVIDIA — solo valida montaje live, no GA107)

Generado: {gen}
"#,
        total as f64 / (1024.0 * 1024.0 * 1024.0),
        data_len as f64 / (1024.0 * 1024.0),
        models_len as f64 / (1024.0 * 1024.0),
        llm = selection.llm_name,
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

/// Huecos 8.3 pre-creados en la ESP (log, hwscan, install, WiFi, OTA).
pub(crate) fn create_esp_slots(live: &Path) {
    create_esp_file(live, b"SOSOLOG ", b"TXT", 256 * 1024);
    create_esp_file(live, b"SOSODRV ", b"TXT", 16 * 1024);
    create_esp_file(live, b"SOSOBOOT", b"TXT", 4096);
    create_esp_file(live, b"SOSOWIFI", b"TXT", 4096);
    create_esp_file(live, b"SOSOUPD ", b"TXT", 4096);
    create_esp_file(live, b"SOSOKRN ", b"BIN", 64 * 1024 * 1024);
}

fn esp_wifi_name11() -> [u8; 11] {
    let mut name11 = [0u8; 11];
    name11[..8].copy_from_slice(b"SOSOWIFI");
    name11[8..11].copy_from_slice(b"TXT");
    name11
}

/// Lee `SOSOWIFI.TXT` de la ESP si existe (para preservarlo al reflashear p1).
pub(crate) fn read_esp_wificonf(dev: &Path) -> Option<Vec<u8>> {
    let p1 = partition_first_sector(dev, 1)?;
    crate::fat32_write::read_root_file(dev, p1, &esp_wifi_name11()).ok()
}

/// Restaura credenciales WiFi en la ESP tras actualizar p1.
pub(crate) fn write_esp_wificonf(dev: &Path, data: &[u8]) -> Result<(), String> {
    let p1 = partition_first_sector(dev, 1)
        .ok_or_else(|| String::from("sin partición ESP (p1)"))?;
    crate::fat32_write::write_root_file(dev, p1, b"SOSOWIFI", b"TXT", data)
}

/// Comprueba que `dev` tiene la GPT mínima de un live soso (p1–p3).
pub(crate) fn validate_live_usb(dev: &Path) -> Result<(), String> {
    let out = Command::new("sgdisk")
        .args(["-v"])
        .arg(dev)
        .output()
        .map_err(|e| format!("sgdisk: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "no hay GPT válida en {} (¿pendrive sin flashear?)",
            dev.display()
        ));
    }
    for part in 1..=3 {
        partition_range(dev, part).ok_or_else(|| {
            format!(
                "falta partición {part} en {} — haz un flash completo primero",
                dev.display()
            )
        })?;
    }
    Ok(())
}

/// Copia una partición entera de `src` a `dst` (mismo número). Aborta si la
/// fuente no cabe en el destino.
pub(crate) fn dd_partition(
    src: &Path,
    src_part: u32,
    dst: &Path,
    dst_part: u32,
) -> Result<(), String> {
    let (src_first, src_last) = partition_range(src, src_part)
        .ok_or_else(|| format!("sin partición {src_part} en {}", src.display()))?;
    let (dst_first, dst_last) = partition_range(dst, dst_part)
        .ok_or_else(|| format!("sin partición {dst_part} en {}", dst.display()))?;
    let src_sectors = src_last.saturating_sub(src_first).saturating_add(1);
    let dst_sectors = dst_last.saturating_sub(dst_first).saturating_add(1);
    if src_sectors > dst_sectors {
        return Err(format!(
            "partición {dst_part} del destino ({dst_sectors} sectores) es más pequeña \
             que la fuente ({src_sectors}) — haz un flash completo"
        ));
    }
    let label = format!("dd p{src_part}→p{dst_part}");
    run_cmd(
        Command::new("dd")
            .arg(format!("if={}", src.display()))
            .arg(format!("of={}", dst.display()))
            .args(["bs=512", "conv=notrunc"])
            .arg(format!("skip={src_first}"))
            .arg(format!("seek={dst_first}"))
            .arg(format!("count={src_sectors}")),
        &label,
    );
    Ok(())
}

/// Escribe `data` en la partición `part` del dispositivo (desde el sector 0 de la partición).
pub(crate) fn dd_to_partition(
    src: &Path,
    dst: &Path,
    dst_part: u32,
) -> Result<(), String> {
    let data_len = std::fs::metadata(src)
        .map_err(|e| format!("stat {}: {e}", src.display()))?
        .len();
    let (dst_first, dst_last) = partition_range(dst, dst_part)
        .ok_or_else(|| format!("sin partición {dst_part} en {}", dst.display()))?;
    let dst_sectors = dst_last.saturating_sub(dst_first).saturating_add(1);
    let need_sectors = data_len.div_ceil(512);
    if need_sectors > dst_sectors {
        return Err(format!(
            "{} ({} B) no cabe en partición {dst_part} ({} sectores) — haz un flash completo",
            src.display(),
            data_len,
            dst_sectors
        ));
    }
    run_cmd(
        Command::new("dd")
            .arg(format!("if={}", src.display()))
            .arg(format!("of={}", dst.display()))
            .args(["bs=512", "conv=notrunc"])
            .arg(format!("seek={dst_first}"))
            .arg(format!("count={need_sectors}")),
        &format!("dd → p{dst_part}"),
    );
    Ok(())
}

/// Pre-crea un fichero contiguo en la raíz de la ESP, relleno de `\n`.
///
/// El kernel no sabe crear ficheros FAT: `drivers/espfat.rs` solo localiza y
/// sobrescribe sectores de un fichero que ya existe y no está fragmentado. Por
/// eso el hueco se reserva aquí, en el empaquetado.
fn create_esp_file(live: &Path, name: &[u8; 8], ext: &[u8; 3], size: usize) {
    let p1_start = partition_first_sector(live, 1).expect("part1 lba");
    let label = format!(
        "{}.{}",
        String::from_utf8_lossy(name).trim_end(),
        String::from_utf8_lossy(ext)
    );
    let mut name11 = [0u8; 11];
    name11[..8].copy_from_slice(name);
    name11[8..11].copy_from_slice(ext);
    if let Ok(Some((_, existing))) = crate::fat32_write::find_root_entry(live, p1_start, &name11)
    {
        if existing as usize >= size {
            println!("package-usb-live: {label} ya reservado ({existing} B) — reutilizo");
            return;
        }
    }
    let data = vec![b'\n'; size];
    if let Err(e) = crate::fat32_write::write_root_file(live, p1_start, name, ext, &data) {
        eprintln!("package-usb-live: aviso: no pude crear {label}: {e}");
        eprintln!("package-usb-live: regenera con espacio libre en la ESP");
        return;
    }
    println!(
        "package-usb-live: {label} ({} KiB) en ESP part1 LBA {p1_start}",
        size / 1024
    );
}

pub(crate) fn run_cmd(cmd: &mut Command, label: &str) {
    let st = cmd.status().unwrap_or_else(|e| panic!("{label}: {e}"));
    if !st.success() {
        eprintln!("xtask: {label} falló");
        exit(st.code().unwrap_or(1));
    }
}

pub(crate) fn partition_first_sector(img: &Path, part: u32) -> Option<u64> {
    partition_range(img, part).map(|(first, _)| first)
}

/// `(primer sector, último sector)` leídos de la GPT en disco, no de `/dev/sdXN`.
pub(crate) fn partition_range(img: &Path, part: u32) -> Option<(u64, u64)> {
    let out = Command::new("sgdisk")
        .args(["-i", &part.to_string()])
        .arg(img)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut first = None;
    let mut last = None;
    for line in text.lines() {
        if line.contains("First sector:") {
            first = parse_sgdisk_sector(line);
        } else if line.contains("Last sector:") {
            last = parse_sgdisk_sector(line);
        }
    }
    Some((first?, last?))
}

fn parse_sgdisk_sector(line: &str) -> Option<u64> {
    line.split(':')
        .nth(1)?
        .trim()
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

pub fn live_image_path() -> PathBuf {
    super::project_root().join("target/usb-live/soso-live.img")
}

pub fn out_dir() -> PathBuf {
    super::project_root().join("target/usb-live")
}

pub fn ensure_live_image() {
    let p = live_image_path();
    if !p.exists() {
        run();
    }
}

fn write_installer_bundle(out_dir: &Path, live_bytes: u64) {
    let bytes_file = out_dir.join("soso-live.bytes");
    std::fs::write(&bytes_file, live_bytes.to_string()).expect("soso-live.bytes");

    let script = out_dir.join("install-soso.sh");
    std::fs::write(&script, install_soso_sh(live_bytes)).expect("install-soso.sh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755));
    }

    let install_txt = out_dir.join("INSTALL.txt");
    let body = format!(
        r#"soso — instalador dual-boot desde Linux (UEFI)

Lo normal es NO necesitar nada de esto: arranca el USB live y usa `soso-install`
desde el propio soso, que particiona el disco destino y registra la entrada de
arranque UEFI sin tocar el disco de Linux. Este directorio es el camino
alternativo, para instalar desde Linux o para forzar una entrada de GRUB.

Archivos en este directorio:
  soso-live.img     — imagen GPT (ESP + sosofs + sosomfs)
  install-soso.sh   — instalador (no requiere cargo/rust)
  soso-live.bytes   — tamaño de la imagen en bytes ({live_bytes} B)

1) Identifica el disco vacío (NO el de Linux, NO el USB):

   lsblk

2) Instala soso desde Linux:

   sudo ./install-soso.sh /dev/nvme1n1 --yes

   Solo GRUB (si ya instalaste desde soso live y prefieres GRUB a la entrada
   UEFI que registra el instalador nativo):

   sudo ./install-soso.sh --grub-only /dev/nvme1n1

3) Reinicia → menú GRUB → "soso".

Alternativa con cargo (misma máquina de desarrollo):

   sudo cargo xtask install-disk /dev/nvme1n1 --yes

Desinstalar entrada GRUB:

   sudo rm /etc/grub.d/41_soso && sudo update-grub
"#
    );
    std::fs::write(&install_txt, body).expect("INSTALL.txt");
    println!("package-usb-live: {}", script.display());
}

/// FAT32 de p4 (`SOSOINSTALL`) con el instalador, para incrustarla en la imagen.
fn build_install_fat(out_dir: &Path) -> PathBuf {
    let fat = out_dir.join("soso-install.fat");
    {
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&fat)
            .unwrap_or_else(|e| panic!("soso-install.fat: {e}"));
        f.set_len(live_models::P4_INSTALL_BYTES)
            .unwrap_or_else(|e| panic!("soso-install.fat size: {e}"));
    }
    run_cmd(
        Command::new("mkfs.vfat")
            .args(["-F", "32", "-n", "SOSOINSTALL"])
            .arg(&fat),
        "mkfs.vfat SOSOINSTALL",
    );
    if !fill_install_fat(&fat, out_dir) {
        eprintln!(
            "package-usb-live: aviso: FAT creada sin copiar el instalador (¿loop/udisks?)"
        );
    }
    fat
}

fn fill_install_fat(fat: &Path, out_dir: &Path) -> bool {
    fill_fat_losetup(fat, out_dir) || fill_fat_udisks(fat, out_dir)
}

fn fill_fat_losetup(fat: &Path, out_dir: &Path) -> bool {
    let out = Command::new("losetup")
        .args(["-f", "--show"])
        .arg(fat)
        .output();
    let Ok(out) = out else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let loopdev = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if loopdev.is_empty() {
        return false;
    }
    let ok = mount_and_copy_install(&loopdev, out_dir);
    let _ = Command::new("losetup").args(["-d", &loopdev]).status();
    ok
}

fn fill_fat_udisks(fat: &Path, out_dir: &Path) -> bool {
    let out = Command::new("udisksctl")
        .args(["loop-setup", "-f"])
        .arg(fat)
        .args(["--no-user-interaction"])
        .output();
    let Ok(out) = out else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let msg = String::from_utf8_lossy(&out.stdout);
    let Some(loopdev) = msg
        .split_whitespace()
        .rev()
        .find(|s| s.contains("/dev/"))
        .map(|s| s.trim_end_matches('.').to_string())
    else {
        return false;
    };
    let ok = if Command::new("udisksctl")
        .args(["mount", "-b", &loopdev, "--no-user-interaction"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        let mnt = Command::new("findmnt")
            .args(["-n", "-o", "TARGET", &loopdev])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(mnt) = mnt {
            copy_installer_files(out_dir, Path::new(&mnt));
            let _ = Command::new("udisksctl")
                .args(["unmount", "-b", &loopdev, "--no-user-interaction"])
                .status();
            true
        } else {
            false
        }
    } else {
        false
    };
    let _ = Command::new("udisksctl")
        .args(["loop-delete", "-b", &loopdev, "--no-user-interaction"])
        .status();
    ok
}

fn mount_and_copy_install(loopdev: &str, out_dir: &Path) -> bool {
    let mnt = out_dir.join("soso-install-mnt");
    let _ = std::fs::create_dir_all(&mnt);
    let _ = Command::new("umount").arg(&mnt).status();
    if !Command::new("mount")
        .args(["-t", "vfat", "-o", "loop"])
        .arg(loopdev)
        .arg(&mnt)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        // `mount -o loop` sobre el propio fichero, por si losetup no bastó.
        if !Command::new("mount")
            .args(["-t", "vfat"])
            .arg(loopdev)
            .arg(&mnt)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return false;
        }
    }
    copy_installer_files(out_dir, &mnt);
    let _ = Command::new("umount").arg(&mnt).status();
    true
}

fn copy_installer_files(src_dir: &Path, dst: &Path) {
    for name in ["install-soso.sh", "soso-live.bytes", "INSTALL.txt"] {
        let src = src_dir.join(name);
        if src.exists() {
            let _ = std::fs::copy(&src, dst.join(name));
        }
    }
    let sh = dst.join("install-soso.sh");
    if sh.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755));
        }
    }
}

fn install_soso_sh(live_bytes: u64) -> String {
    format!(
        r#"#!/bin/bash
# Instala soso en un disco vacío y añade entrada GRUB (dual-boot UEFI).
# Generado por cargo xtask package-usb-live — no requiere Rust.
set -euo pipefail

LIVE_BYTES={live_bytes}
DIR="$(cd "$(dirname "${{BASH_SOURCE[0]}}")" && pwd)"
TARGET=""
YES=0
NO_GRUB=0

usage() {{
  echo "uso: sudo $0 <disco> [--yes] [--no-grub]" >&2
  echo "     sudo $0 --grub-only <disco>   # tras soso-install desde live" >&2
  echo "  p. ej. sudo $0 /dev/nvme1n1 --yes" >&2
  exit 2
}}

if [[ "${{1:-}}" == "--grub-only" ]]; then
  shift
  TARGET="${{1:-}}"
  [[ -n "$TARGET" ]] || {{ echo "falta disco destino" >&2; exit 2; }}
  [[ $EUID -eq 0 ]] || {{ echo "ejecuta con sudo" >&2; exit 1; }}
  esp="${{TARGET}}p1"
  [[ -b "$esp" ]] || esp="${{TARGET}}1"
  uuid=$(blkid -s UUID -o value "$esp" 2>/dev/null || true)
  [[ -n "$uuid" ]] || {{ echo "no UUID en $esp" >&2; exit 1; }}
  cat > /etc/grub.d/41_soso <<GRUB
#!/bin/sh
exec tail -n +3 "\$0"
menuentry "soso" {{
    insmod part_gpt
    insmod fat
    search --no-floppy --fs-uuid --set=root $uuid
    chainloader /EFI/BOOT/BOOTX64.EFI
}}
GRUB
  chmod 755 /etc/grub.d/41_soso
  if command -v update-grub >/dev/null; then update-grub
  elif command -v grub-mkconfig >/dev/null; then grub-mkconfig -o /boot/grub/grub.cfg
  fi
  echo "GRUB: entrada soso (ESP UUID $uuid en $esp)"
  exit 0
fi

for arg in "$@"; do
  case "$arg" in
    --yes|-y) YES=1 ;;
    --no-grub) NO_GRUB=1 ;;
    -*) echo "opción desconocida: $arg" >&2; usage ;;
    *)
      if [[ -n "$TARGET" ]]; then usage; fi
      TARGET="$arg"
      ;;
  esac
done

[[ -n "$TARGET" ]] || usage
[[ $EUID -eq 0 ]] || {{ echo "ejecuta con sudo" >&2; exit 1; }}

need() {{ command -v "$1" >/dev/null || {{ echo "falta $1" >&2; exit 1; }}; }}
need dd
need sgdisk
need blkid
need lsblk
need findmnt

is_partition() {{
  local d="$1"
  [[ "$d" =~ p[0-9]+$ ]] && [[ "$d" == *nvme* || "$d" == *mmcblk* ]] && return 0
  [[ "$d" =~ [0-9]$ ]] && [[ "$d" != *nvme* ]] && [[ "$d" != *n1 ]] && return 0
  return 1
}}

if is_partition "$TARGET"; then
  echo "pasa el disco entero (p. ej. /dev/nvme1n1), no una partición" >&2
  exit 1
fi

[[ -b "$TARGET" ]] || {{ echo "no existe o no es block device: $TARGET" >&2; exit 1; }}

linux_disk() {{
  for mp in / /boot /boot/efi; do
    src=$(findmnt -n -o SOURCE "$mp" 2>/dev/null || true)
    [[ -n "$src" ]] || continue
    pk=$(lsblk -no PKNAME "$src" 2>/dev/null || true)
    if [[ -n "$pk" ]]; then echo "/dev/$pk"; else echo "$src"; fi
    return 0
  done
}}

ROOT=$(linux_disk || true)
canonical() {{ readlink -f "$1" 2>/dev/null || echo "$1"; }}
if [[ -n "$ROOT" ]] && [[ "$(canonical "$ROOT")" == "$(canonical "$TARGET")" ]]; then
  echo "refusing: $TARGET parece el disco de Linux ($ROOT)" >&2
  exit 1
fi

usb_disk() {{
  if [[ -n "${{SOSO_USB:-}}" ]]; then echo "$SOSO_USB"; return; fi
  local src mp
  mp=$(findmnt -n -o SOURCE "$DIR" 2>/dev/null || true)
  [[ -n "$mp" ]] || return 0
  src=$(lsblk -no PKNAME "$mp" 2>/dev/null || true)
  if [[ -n "$src" ]]; then echo "/dev/$src"; else echo "$mp"; fi
}}

USB=$(usb_disk || true)
if [[ -n "$USB" ]] && [[ "$(canonical "$USB")" == "$(canonical "$TARGET")" ]]; then
  echo "refusing: destino $TARGET es el mismo USB de origen" >&2
  exit 1
fi

while read -r name mp; do
  [[ -n "$mp" ]] || continue
  base=$(basename "$TARGET")
  [[ "$name" != "$base" ]] || continue
  echo "partición montada: /dev/$name → $mp" >&2
  exit 1
done < <(lsblk -rn -o NAME,MOUNTPOINT "$TARGET" 2>/dev/null || true)

if [[ $YES -eq 0 ]]; then
  echo "ATENCIÓN — se borrará TODO en $TARGET"
  read -r -p "¿Continuar? [y/N] " ans
  case "$ans" in y|Y|yes|si|s) ;; *) echo cancelado; exit 1 ;; esac
fi

write_image() {{
  if [[ -f "$DIR/soso-live.img" ]]; then
    echo "origen: $DIR/soso-live.img → $TARGET"
    dd if="$DIR/soso-live.img" of="$TARGET" bs=4M status=progress conv=fsync
  elif [[ -n "$USB" ]] && [[ -f "$DIR/soso-live.bytes" ]]; then
    echo "origen: $USB (primeros $LIVE_BYTES B) → $TARGET"
    dd if="$USB" of="$TARGET" bs=4M count=$(( (LIVE_BYTES + 4194303) / 4194304 )) status=progress conv=fsync
  else
    echo "no encuentro soso-live.img ni USB de origen (define SOSO_USB=/dev/sdX)" >&2
    exit 1
  fi
  sync
}}

write_image

disk_sectors() {{ blockdev --getsz "$TARGET" 2>/dev/null || echo 0; }}
live_sectors=$(( LIVE_BYTES / 512 ))
img_sectors=$(disk_sectors)
if [[ "$img_sectors" -gt "$live_sectors" ]]; then
  echo "ampliando partición 3…"
  sgdisk -e "$TARGET" -d 3 -n 3:0:0 -t 3:8300
fi

esp="${{TARGET}}p1"
[[ -b "$esp" ]] || esp="${{TARGET}}1"
uuid=$(blkid -s UUID -o value "$esp" 2>/dev/null || true)

grub_snippet() {{
  cat <<GRUB
#!/bin/sh
exec tail -n +3 "\$0"
menuentry "soso" {{
    insmod part_gpt
    insmod fat
    search --no-floppy --fs-uuid --set=root ${{1:-UNKNOWN}}
    chainloader /EFI/BOOT/BOOTX64.EFI
}}
GRUB
}}

if [[ $NO_GRUB -eq 0 ]] && [[ -n "$uuid" ]]; then
  grub_snippet "$uuid" > /etc/grub.d/41_soso
  chmod 755 /etc/grub.d/41_soso
  if command -v update-grub >/dev/null; then update-grub
  elif command -v grub-mkconfig >/dev/null; then grub-mkconfig -o /boot/grub/grub.cfg
  else echo "aviso: regenera grub.cfg manualmente" >&2
  fi
  echo "GRUB: entrada soso (ESP UUID $uuid)"
elif [[ $NO_GRUB -eq 0 ]]; then
  echo "aviso: no pude leer UUID de $esp — usa --no-grub y edita GRUB a mano" >&2
fi

echo ""
echo "✅ soso instalado en $TARGET"
echo "   Reinicia y elige \"soso\" en GRUB."
"#
    )
}
