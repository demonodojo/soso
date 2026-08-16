#!/usr/bin/env bash
# Copia solo blobs GSP gb205 (+ ga102 gsp si hay symlink) a rootfs/.
# Descomprime .zst → .bin (G3; soso no tiene zstd en kernel).
# Uso: ./scripts/l6-pack-firmware.sh [--repack]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${SOSO_FIRMWARE_SRC:-/lib/firmware/nvidia}"
DST="${ROOT}/rootfs/lib/firmware/nvidia"
REPACK=0
[[ "${1:-}" == "--repack" ]] && REPACK=1

if [[ ! -d "$SRC" ]]; then
  echo "FAIL: no existe $SRC (instala linux-firmware)" >&2
  exit 1
fi

if [[ "$REPACK" -eq 1 && -d "$DST" ]]; then
  rm -rf "$DST"
fi

mkdir -p "$DST"

copy_one() {
  local rel="$1"
  local dir
  dir="$(dirname "$rel")"
  mkdir -p "$DST/$dir"
  if [[ -L "$SRC/$rel" ]]; then
    cp -rL "$SRC/$rel" "$DST/$rel" 2>/dev/null || cp -r "$SRC/$rel" "$DST/$rel"
    echo "OK: $rel (symlink)"
  elif [[ -f "$SRC/$rel" ]]; then
    cp "$SRC/$rel" "$DST/$rel"
    echo "OK: $rel"
  else
    echo "WARN: falta $rel en $SRC" >&2
    return 1
  fi
}

decompress_zst_in() {
  local dir="$1"
  if ! command -v zstd >/dev/null 2>&1; then
    echo "FAIL: zstd no instalado (apt install zstd)" >&2
    exit 1
  fi
  local n=0
  while IFS= read -r -d '' zst; do
    local out="${zst%.zst}"
    zstd -d -f -q "$zst" -o "$out"
    rm -f "$zst"
    n=$((n + 1))
  done < <(find "$dir" -name '*.zst' -print0 2>/dev/null)
  echo "Descomprimidos $n blobs (solo .bin en rootfs)"
}

resolve_symlink_target() {
  local link="$1"
  if [[ -L "$DST/$link" ]]; then
    local target
    target=$(readlink "$DST/$link")
    if [[ "$target" == ../* ]]; then
      local base
      base=$(dirname "$link")
      target="${base}/${target#../}"
    fi
    echo "$target"
  fi
}

verify_gb205() {
  local bl=0 elf=0 acr=0
  local bl_path="$DST/gb205/gsp/bootloader-570.144.bin"
  local fmc="$DST/gb205/gsp/fmc-570.144.bin"
  local gsp="$DST/gb205/gsp/gsp-570.144.bin"

  if [[ -f "$bl_path" ]] && [[ $(stat -c%s "$bl_path") -gt 4096 ]]; then
    echo "OK: gb205/gsp/bootloader-570.144.bin ($(stat -c%s "$bl_path") bytes)"
    bl=1
  fi
  for f in "$fmc" "$gsp"; do
    if [[ -f "$f" ]] && head -c 4 "$f" | grep -q $'^\x7fELF'; then
      echo "OK: ${f#$DST/} (ELF)"
      elf=$((elf + 1))
    fi
  done
  for f in ga102/acr/ucode_ahesasc.bin ga102/acr/ucode_asb.bin; do
    if [[ -f "$DST/$f" ]] && [[ $(stat -c%s "$DST/$f") -gt 4096 ]]; then
      echo "OK: $f ($(stat -c%s "$DST/$f") bytes)"
      acr=$((acr + 1))
    fi
  done
  if [[ "$bl" -eq 0 || "$elf" -lt 2 ]]; then
    echo "FAIL: gb205 GSP incompleto (bootloader=$bl ELF=$elf/2)" >&2
    exit 1
  fi
  if [[ "$acr" -lt 2 ]]; then
    echo "WARN: ACR ga102 incompleto ($acr/2) — G3 ola2 soft-fail" >&2
  fi
}

echo "=== L6 pack firmware (mínimo gb205) → rootfs/lib/firmware/nvidia ==="

GB205=(
  gb205/gsp/bootloader-570.144.bin.zst
  gb205/gsp/fmc-570.144.bin.zst
  gb205/gsp/gsp-570.144.bin.zst
)
for f in "${GB205[@]}"; do
  copy_one "$f" || true
done

# --- Set GSP ga102 (Ampere) — objetivo RTX 3060: GSP maduro en nouveau ---
# gsp-570 suele ser symlink → ga102/gsp/gsp-570.144.bin.zst
GA102_GSP=(
  ga102/gsp/bootloader-570.144.bin.zst
  ga102/gsp/booter_load-570.144.bin.zst
  ga102/gsp/booter_unload-570.144.bin.zst
  ga102/gsp/gsp-570.144.bin.zst
)
for f in "${GA102_GSP[@]}"; do
  copy_one "$f" || true
done

GA102_ACR=(
  ga102/acr/ucode_ahesasc.bin.zst
  ga102/acr/ucode_asb.bin.zst
)
for f in "${GA102_ACR[@]}"; do
  copy_one "$f" || true
done

# --- Set GSP ga107 (Ampere RTX 3050 Mobile) — rutas propias, a menudo symlink a ga102 ---
GA107_GSP=(
  ga107/gsp/bootloader-570.144.bin.zst
  ga107/gsp/booter_load-570.144.bin.zst
  ga107/gsp/booter_unload-570.144.bin.zst
  ga107/gsp/gsp-570.144.bin.zst
)
for f in "${GA107_GSP[@]}"; do
  copy_one "$f" || true
done

GA107_ACR=(
  ga107/acr/ucode_ahesasc.bin.zst
  ga107/acr/ucode_asb.bin.zst
)
for f in "${GA107_ACR[@]}"; do
  copy_one "$f" || true
done

decompress_zst_in "$DST"

# Enlace gb205/gsp/gsp-570.144.bin → ga102 descomprimido (si aplica)
if [[ ! -e "$DST/gb205/gsp/gsp-570.144.bin" && -f "$DST/ga102/gsp/gsp-570.144.bin" ]]; then
  ln -sf "../../ga102/gsp/gsp-570.144.bin" "$DST/gb205/gsp/gsp-570.144.bin"
  echo "OK: gb205/gsp/gsp-570.144.bin → ga102/gsp/gsp-570.144.bin"
fi

verify_gb205

# Estado del set GSP ga102 (RTX 3060). Informativo: si está completo, soso puede
# gestionar la 3060 (GSP Ampere maduro en nouveau); si no, solo avisa.
verify_ga102_gsp() {
  local n=0
  for f in bootloader booter_load booter_unload gsp; do
    [[ -s "$DST/ga102/gsp/${f}-570.144.bin" ]] && n=$((n+1))
  done
  if [[ $n -ge 3 ]]; then
    echo "OK: set GSP ga102 (RTX 3060) presente ($n/4 blobs)"
  else
    echo "WARN: set GSP ga102 incompleto ($n/4) — para la 3060 instala linux-firmware con nvidia/ga102/gsp/{bootloader,booter_load,booter_unload,gsp}-570.144.bin"
  fi
}
verify_ga102_gsp

verify_ga107_gsp() {
  local n=0
  for f in bootloader booter_load booter_unload gsp; do
    [[ -s "$DST/ga107/gsp/${f}-570.144.bin" ]] && n=$((n+1))
  done
  if [[ $n -ge 3 ]]; then
    echo "OK: set GSP ga107 (RTX 3050 Mobile) presente ($n/4 blobs)"
  else
    echo "WARN: set GSP ga107 incompleto ($n/4) — para la 3050 Mobile instala linux-firmware con nvidia/ga107/gsp/{bootloader,booter_load,booter_unload,gsp}-570.144.bin"
  fi
}
verify_ga107_gsp

count=$(find "$DST" -type f 2>/dev/null | wc -l)
du_human=$(du -sh "$DST" | cut -f1)
echo ""
echo "Empaquetados $count ficheros ($du_human) bajo rootfs/lib/firmware/nvidia"
echo "Regenera imagen: cargo xtask build"
