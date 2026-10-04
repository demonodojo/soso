#!/usr/bin/env bash
# T42, paso 4 (resto): las tres herramientas de imagen —`mkfs-soso`,
# `mkfs-sosomfs`, `mkmodel-soso`— con sus crates, vendidas, en `rootfs/var/t42/tools/`
# para compilarlas dentro de soso sin red (target std `x86_64-unknown-soso`).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
SRC="$ROOT/tests/self-improvement/native/herramientas"
DST="$ROOT/rootfs/var/t42/tools"
rm -rf "$DST"
mkdir -p "$DST/crates" "$DST/tools" "$DST/.cargo"
for c in block-dev sosofs sosomfs sosomodel soso-llm-core soso-audio; do cp -r "$ROOT/crates/$c" "$DST/crates/"; done
for t in mkfs-soso mkfs-sosomfs mkmodel-soso; do cp -r "$ROOT/tools/$t" "$DST/tools/"; done
rm -rf "$DST"/crates/*/target "$DST"/tools/*/target
cp "$SRC/Cargo.toml" "$DST/"
[ -f "$SRC/Cargo.lock" ] && cp "$SRC/Cargo.lock" "$DST/"
cp "$SRC/.cargo/config.toml" "$DST/.cargo/"
( cd "$DST" && cargo vendor --offline --versioned-dirs vendor > /dev/null )
cp "$DST/Cargo.lock" "$SRC/Cargo.lock"
( cd "$DST" && find . -type f ! -name MANIFEST.sha256 ! -path './vendor/*/target/*' | LC_ALL=C sort | xargs sha256sum > MANIFEST.sha256 )
echo "preparado: $DST ($(wc -l < "$DST/MANIFEST.sha256") ficheros, $(du -sh "$DST/vendor" | cut -f1) vendidos)"
