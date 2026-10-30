#!/usr/bin/env bash
# N7 — regresión host: política scanout (GOP forzado vs auto) sin QEMU ni VFIO.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

echo "== scanout: tests Rust (router + ABI) =="
cargo test -p soso-wm-proto -p soso-abi --quiet

echo "== scanout: Sal comp_scanout_policy =="
python3 tests/sal/escritorio/probar_host.py comp_scanout_policy.sal

echo "== scanout: kernel SOSO_SCANOUT=gop (QEMU / consola GOP) =="
SOSO_SCANOUT=gop cargo check --manifest-path kernel/Cargo.toml -q

echo "== scanout: kernel SOSO_SCANOUT=auto (default) =="
SOSO_SCANOUT=auto cargo check --manifest-path kernel/Cargo.toml -q

echo "l6-scanout-regression: OK"
