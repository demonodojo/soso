#!/usr/bin/env bash
# C-144: ejerce directamente el backend versionado que usa soso.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../../.." && pwd)"
tmp_root="$(mktemp -d /tmp/soso-jobserver-regression.XXXXXX)"
trap 'rm -rf -- "$tmp_root"' EXIT

# Sólo std: sin registry, ni Python, ni preparación/modificación de fuentes.
rustc --edition=2021 --test --crate-name jobserver \
  --cfg 'feature="soso-tests"' \
  "$ROOT/config/rust-soso/vendor/jobserver/src/lib.rs" \
  -o "$tmp_root/jobserver-tests"
timeout 30 "$tmp_root/jobserver-tests" --nocapture
