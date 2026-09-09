#!/usr/bin/env bash
# Copia el firmware de ath11k (WCN6855 hw2.1 — el WiFi de la Steam Deck OLED)
# a rootfs/lib/firmware/. Descomprime .zst porque soso no lleva zstd en kernel,
# igual que el empaquetado del GSP.
#
# Uso: ./scripts/l6-pack-ath11k-fw.sh [--repack]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${SOSO_ATH11K_FW_SRC:-/lib/firmware/ath11k/WCN6855/hw2.1}"
DST="${ROOT}/rootfs/lib/firmware/ath11k/WCN6855/hw2.1"
REPACK=0
[[ "${1:-}" == "--repack" ]] && REPACK=1

if [[ ! -d "$SRC" ]]; then
  echo "FAIL: no existe $SRC (instala linux-firmware)" >&2
  exit 1
fi

[[ "$REPACK" -eq 1 && -d "$DST" ]] && rm -rf "$DST"
mkdir -p "$DST"

copiar() {
  local rel="$1" src dst
  src="$SRC/$rel"
  dst="$DST/${rel%.zst}"
  mkdir -p "$(dirname "$dst")"
  if [[ "$rel" == *.zst ]]; then
    zstd -qdf --stdout "$src" > "$dst"
  else
    cp -L "$src" "$dst"
  fi
  printf '  %-28s %10s B\n' "${rel%.zst}" "$(stat -c%s "$dst")"
}

echo "ath11k: $SRC → $DST"
encontrados=0
# amss.bin es el firmware que se descarga por BHIe; m3.bin lo pide el firmware
# por QMI; board-2.bin lleva las calibraciones por board_id; regdb.bin las
# reglas regulatorias.
for f in amss.bin m3.bin board-2.bin regdb.bin; do
  for cand in "$f" "$f.zst"; do
    if [[ -e "$SRC/$cand" ]]; then
      copiar "$cand"
      encontrados=$((encontrados + 1))
      break
    fi
  done
done

# Variante del módulo: linux-firmware publica subdirectorios por modelo de
# tarjeta (el de la Deck OLED es un QCNFA765). Se copian si están, para poder
# contrastarlos con los genéricos cuando la placa diga cuál acepta.
if [[ -d "$SRC/nfa765" ]]; then
  echo "  (variante nfa765 presente)"
  for f in amss.bin m3.bin board-2.bin; do
    for cand in "nfa765/$f" "nfa765/$f.zst"; do
      [[ -e "$SRC/$cand" ]] && copiar "$cand" && break
    done
  done
fi

if [[ "$encontrados" -eq 0 ]]; then
  echo "FAIL: no se copió ningún blob desde $SRC" >&2
  exit 1
fi
echo "OK: $encontrados blobs principales en $DST"
