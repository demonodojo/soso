//! `cargo xtask package-usb-live` — imagen GPT única para arranque live (sin tocar NVMe interno).

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

use crate::drivers::{self, DriverProfile};

/// Perfil live: `SOSO_DRIVERS` si está definido; si no, `live-usb` (GPU GA107 incluida).
fn live_profile() -> DriverProfile {
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
    let root = super::project_root();
    let profile = live_profile();
    preflight_gpu_firmware(&root, &profile);
    print_profile_summary(&profile);

    super::build_user();
    let _llm_conf = LiveLlmConf::set_tinyllama(&root);
    let _ = super::build_image_with_profile(&profile);

    let uefi = root.join("target/soso-uefi.img");
    let mut data = super::mkfs_rootfs_with_profile(true, &profile);
    let models = super::mkfs_models_live(true);

    let mut total = live_image_bytes(&data, &models, &uefi);
    for _ in 0..2 {
        write_rootfs_install_meta(&root, total);
        data = super::mkfs_rootfs_with_profile(true, &profile);
        total = live_image_bytes(&data, &models, &uefi);
    }

    let data_len = std::fs::metadata(&data).expect("data").len();
    let models_len = std::fs::metadata(&models).expect("models").len();
    let total = live_image_bytes(&data, &models, &uefi);
    let align = 1024 * 1024;
    let p2_size = (data_len + align - 1) / align * align;

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

    // Log del kernel, informe hwscan y buzón para el instalador → shim UEFI.
    create_esp_file(&live, &out_dir, b"SOSOLOG ", b"TXT", 256 * 1024);
    create_esp_file(&live, &out_dir, b"SOSODRV ", b"TXT", 16 * 1024);
    create_esp_file(&live, &out_dir, b"SOSOBOOT", b"TXT", 4096);

    write_flash(&out_dir, &live, data_len, models_len, total);
    write_installer_bundle(&out_dir, total);
    println!("package-usb-live: {}", live.display());
    print_profile_summary(&profile);
    println!("\n✅ Live USB listo en {}", out_dir.display());
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

/// Pone `modelo=tinyllama` en rootfs solo durante el empaquetado live; restaura al salir.
struct LiveLlmConf {
    path: PathBuf,
    saved: Option<String>,
}

impl LiveLlmConf {
    fn set_tinyllama(root: &Path) -> Self {
        let path = root.join("rootfs/etc/llm.conf");
        let saved = std::fs::read_to_string(&path).ok();
        let mut content = saved.clone().unwrap_or_else(|| {
            "modelo=tiny\nmax=128\ntemp=0.7\ntop_p=0.9\n".into()
        });
        if content.contains("modelo=") {
            let mut out = String::new();
            for line in content.lines() {
                if line.starts_with("modelo=") {
                    out.push_str("modelo=tinyllama\n");
                } else {
                    out.push_str(line);
                    out.push('\n');
                }
            }
            content = out;
        } else {
            content.push_str("modelo=tinyllama\n");
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
    header + p2_size + p3_size + align
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

   Trazas de arranque: al volver a Linux, monta la ESP del USB (partición 1):
   - BOOTMARK.TXT — la escribe el shim UEFI; si sigue vacía, el firmware nunca
     llegó a ejecutar nuestro loader (Secure Boot, orden de arranque…).
   - SOSOLOG.TXT (256 KiB) — log del kernel; vacío con BOOTMARK escrita = el
     kernel se colgó o no detectó el USB (mira los checkpoints «boot:» en
     pantalla).
   - SOSOBOOT.TXT — buzón entre soso-install y el shim; dice si la entrada de
     arranque UEFI se llegó a registrar y con qué número.

4) Instalar en disco interno **desde soso live** (sin pasar por Linux):

   soso-install list          # mira qué hay en cada disco antes de borrar
   soso-install nvme1 --yes

   Reinicia con el USB puesto: el shim registra la entrada UEFI «soso».
   Después quita el USB y elige «soso» en el menú de arranque de la placa.

5) Instalar desde Linux (sin arrancar soso):

   sudo ./install-soso.sh /dev/nvme1n1 --yes

6) Flashear USB con instalador: sudo cargo xtask flash-usb-live /dev/sdX --yes
   (estira p3 al sobrante del stick; p4 SOSOINSTALL 32 MiB al final; sosomfs
   crece al arrancar)

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

Demo LLM (TinyLlama 1.1B Chat Q4_K_M en /models)
-------------------------------------------------
El live trae `tinyllama` por defecto (`ask` y /etc/llm.conf). También `tiny` sintético.

  ask
  soso-llm run tinyllama --prompt "hola" --max 32

Si falta el modelo al empaquetar:
  cargo xtask fetch-hf TinyLlama/TinyLlama-1.1B-Chat-v1.0

Override al generar: SOSO_MODELS_DIR=/ruta/al/modelo SOSO_MODELS_SIZE=2G cargo xtask package-usb-live

QEMU: SOSO_QEMU_LIVE=1 cargo xtask run
      (QEMU no tiene GPU NVIDIA — solo valida montaje live, no GA107)

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

/// Pre-crea un fichero contiguo en la raíz de la ESP, relleno de `\n`.
///
/// El kernel no sabe crear ficheros FAT: `drivers/espfat.rs` solo localiza y
/// sobrescribe sectores de un fichero que ya existe y no está fragmentado. Por
/// eso el hueco se reserva aquí, en el empaquetado.
fn create_esp_file(live: &Path, out_dir: &Path, name: &[u8; 8], ext: &[u8; 3], size: usize) {
    let p1_start = partition_first_sector(live, 1).expect("part1 lba");
    let label = format!(
        "{}.{}",
        String::from_utf8_lossy(name).trim_end(),
        String::from_utf8_lossy(ext)
    );
    let template = out_dir.join(format!("{}-template.bin", label.replace('.', "-")));
    std::fs::write(&template, vec![b'\n'; size]).expect("plantilla ESP");
    let data = std::fs::read(&template).expect("read template");
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
