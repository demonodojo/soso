#!/usr/bin/env bash
# Inventario de símbolos undefined al añadir fuentes nvkm a source.list.
# Uso: ./scripts/l6-g3-nvkm-inventory.sh [archivo.list]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIST="${1:-$ROOT/lxdde/ports/nouveau/nvkm_ola1.list}"
OUT="$ROOT/target/g3-nvkm-undefined.txt"

if [[ ! -f "$LIST" ]]; then
  echo "FAIL: no existe $LIST" >&2
  exit 1
fi

TMP="$ROOT/lxdde/ports/nouveau/source.list.bak"
cp "$ROOT/lxdde/ports/nouveau/source.list" "$TMP"
cleanup() {
  mv "$TMP" "$ROOT/lxdde/ports/nouveau/source.list"
}
trap cleanup EXIT

{
  cat "$ROOT/lxdde/ports/nouveau/source.list"
  echo "# --- nvkm ola1 trial ---"
  grep -v '^#' "$LIST" | grep -v '^$' || true
} >"$ROOT/lxdde/ports/nouveau/source.list"

cargo xtask lx-build nouveau >/dev/null 2>&1 || true
nm "$ROOT/target/lxdde/liblxdde.a" 2>/dev/null | awk '/ U / {print $3}' | sort -u >"$OUT" || true

echo "=== G3 nvkm — símbolos undefined (top 40) ==="
head -40 "$OUT"
echo ""
echo "Total: $(wc -l <"$OUT") — lista completa: $OUT"
