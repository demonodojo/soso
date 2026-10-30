#!/usr/bin/env bash
# N8 — smoke host: GB205 en árbol, tile scanout, regresión scanout.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$root/lxdde/ports/nouveau"

grep -q 'GB205_DEVICE_ID' "$src/gsp_chip.h"
grep -q 'gsp_vram_blit_pitch' "$src/gsp_bringup.c"
grep -q 'nvdisp_scanout_tile_bytes' "$src/nvdisp_scanout.c"

cd "$root"
./scripts/l6-scanout-regression.sh
cargo test -p soso-wm-proto damaged_tile --quiet

echo "l6-nvdisp-blackwell: OK"
