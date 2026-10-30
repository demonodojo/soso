#!/usr/bin/env bash
# L6 NVDisplay — smoke en host (sin QEMU): hostcheck nvdisp + tests Sal/Rust del router.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

echo "== l6-nvdisp-smoke: gsp-hostcheck (nvdisp) =="
./scripts/l6-g3-gsp-hostcheck.sh
if ! grep -q 'nvdisp_scanout.c' "$root/lxdde/ports/nouveau/source.list"; then
    echo "source.list sin nvdisp_scanout" >&2
    exit 1
fi

echo "== l6-nvdisp-smoke: Sal comp_scanout_policy =="
python3 tests/sal/escritorio/probar_host.py comp_scanout_policy.sal

echo "== l6-nvdisp-smoke: regresión N7 scanout =="
./scripts/l6-scanout-regression.sh

echo "l6-nvdisp-smoke: OK"
