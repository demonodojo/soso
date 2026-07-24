#!/usr/bin/env bash
# Copia blobs GSP gb205 (y deps) desde linux-firmware del host a rootfs/.
# Uso: ./scripts/l6-pack-firmware.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${SOSO_FIRMWARE_SRC:-/lib/firmware/nvidia}"
DST="${ROOT}/rootfs/lib/firmware/nvidia"

if [[ ! -d "$SRC" ]]; then
  echo "FAIL: no existe $SRC (instala linux-firmware)" >&2
  exit 1
fi

mkdir -p "$DST"

copy_tree() {
  local rel="$1"
  if [[ -d "$SRC/$rel" ]]; then
    mkdir -p "$DST/$rel"
    cp -rL "$SRC/$rel/." "$DST/$rel/" 2>/dev/null || cp -r "$SRC/$rel/." "$DST/$rel/"
    echo "OK: $rel"
  elif [[ -L "$SRC/$rel" ]]; then
    mkdir -p "$(dirname "$DST/$rel")"
    cp -rL "$SRC/$rel" "$DST/$rel" 2>/dev/null || cp -r "$SRC/$rel" "$DST/$rel"
    echo "OK: $rel (symlink)"
  else
    echo "WARN: falta $rel en $SRC"
  fi
}

echo "=== L6 pack firmware → rootfs/lib/firmware/nvidia ==="
copy_tree "gb205/gsp"
copy_tree "ga102/gsp"
copy_tree "tu102/gsp"

count=$(find "$DST" -type f 2>/dev/null | wc -l)
echo ""
echo "Empaquetados $count ficheros bajo rootfs/lib/firmware/nvidia"
echo "Regenera imagen: cargo xtask build  (o mkfs rootfs)"
