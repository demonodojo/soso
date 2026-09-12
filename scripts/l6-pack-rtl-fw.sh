#!/usr/bin/env bash
# Copia rtl8168h-2.fw (descomprime .zst) a rootfs/lib/firmware/rtl_nic/.
# Uso: ./scripts/l6-pack-rtl-fw.sh [--repack]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${SOSO_RTL_FW_SRC:-/lib/firmware/rtl_nic}"
DST="${ROOT}/rootfs/lib/firmware/rtl_nic"
REPACK=0
[[ "${1:-}" == "--repack" ]] && REPACK=1

if [[ "$REPACK" -eq 1 && -d "$DST" ]]; then
  rm -rf "$DST"
fi
mkdir -p "$DST"

pack_one() {
  local name="$1"
  if [[ -f "$SRC/${name}.zst" ]]; then
    if ! command -v zstd >/dev/null 2>&1; then
      echo "FAIL: zstd no instalado (apt install zstd)" >&2
      exit 1
    fi
    zstd -d -f -q "$SRC/${name}.zst" -o "$DST/$name"
    echo "OK: $name (desde .zst)"
  elif [[ -f "$SRC/$name" ]]; then
    cp "$SRC/$name" "$DST/$name"
    echo "OK: $name"
  else
    echo "WARN: falta $name en $SRC" >&2
    return 1
  fi
}

pack_one rtl8168h-2.fw
echo "=== rtl PHY firmware → $DST ==="
