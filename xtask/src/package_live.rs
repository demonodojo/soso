//! `cargo xtask package-usb-live` — imagen GPT única para arranque live (sin tocar NVMe interno).

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

pub fn run() {
    let root = super::project_root();
    super::build_user();
    let _ = super::build_image();

    let uefi = root.join("target/soso-uefi.img");
    let mut data = super::mkfs_rootfs(true);
    let models = super::mkfs_models(true);

    let mut total = live_image_bytes(&data, &models, &uefi);
    for _ in 0..2 {
        write_rootfs_install_meta(&root, total);
        data = super::mkfs_rootfs(true);
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

    create_sosolog_on_esp(&live, &out_dir);
    create_sosodrv_on_esp(&live, &out_dir);

    write_flash(&out_dir, &live, data_len, models_len, total);
    write_installer_bundle(&out_dir, total);
    println!("package-usb-live: {}", live.display());
    println!("\n✅ Live USB listo en {}", out_dir.display());
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
Tras instalar desde soso live (soso-install), añade la entrada GRUB desde Linux:\n\
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

4) Instalar en disco interno **desde soso live**:

   soso-install list
   soso-install nvme1 --yes

   Luego reinicia en Linux y ejecuta (USB conectado):

   sudo /media/$USER/SOSOINSTALL/install-soso.sh --grub-only /dev/nvme1n1

5) Instalar desde Linux (sin arrancar soso):

   sudo ./install-soso.sh /dev/nvme1n1 --yes

6) Flashear USB con instalador: sudo cargo xtask flash-usb-live /dev/sdX --yes

7) Apagar, quitar USB, arrancar disco habitual → Linux intacto (si no instalaste).

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

fn create_sosodrv_on_esp(live: &Path, out_dir: &Path) {
    let p1_start = partition_first_sector(live, 1).expect("part1 lba");
    let template = out_dir.join("sosodrv-template.bin");
    std::fs::write(&template, vec![b'\n'; 16 * 1024]).expect("sosodrv template");
    let name = *b"SOSODRV ";
    let ext = *b"TXT";
    if let Err(e) = crate::fat32_write::write_root_file(
        live,
        p1_start,
        &name,
        &ext,
        &std::fs::read(&template).expect("read template"),
    ) {
        eprintln!("package-usb-live: aviso: no pude crear SOSODRV.TXT: {e}");
        return;
    }
    println!(
        "package-usb-live: SOSODRV.TXT (16 KiB) en ESP part1 LBA {p1_start}"
    );
}

fn create_sosolog_on_esp(live: &Path, out_dir: &Path) {
    let p1_start = partition_first_sector(live, 1).expect("part1 lba");
    let template = out_dir.join("sosolog-template.bin");
    std::fs::write(&template, vec![b'\n'; 256 * 1024]).expect("sosolog template");
    let name = *b"SOSOLOG ";
    let ext = *b"TXT";
    if let Err(e) = crate::fat32_write::write_root_file(live, p1_start, &name, &ext, &std::fs::read(&template).expect("read template")) {
        eprintln!("package-usb-live: aviso: no pude crear SOSOLOG.TXT: {e}");
        eprintln!("package-usb-live: instala mtools o regenera con espacio libre en la ESP");
        return;
    }
    println!(
        "package-usb-live: SOSOLOG.TXT (256 KiB) en ESP part1 LBA {p1_start}"
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

Archivos en este directorio:
  soso-live.img     — imagen GPT (ESP + sosofs + sosomfs)
  install-soso.sh   — instalador (no requiere cargo/rust)
  soso-live.bytes   — tamaño de la imagen en bytes ({live_bytes} B)

1) Identifica el disco vacío (NO el de Linux, NO el USB):

   lsblk

2) Instala soso (desde Linux o tras `soso-install` en live):

   sudo ./install-soso.sh /dev/nvme1n1 --yes

   Solo GRUB (tras instalar desde soso live):

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
