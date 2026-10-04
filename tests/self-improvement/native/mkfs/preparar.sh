#!/usr/bin/env bash
# T42, paso 4: compilar `mkfs-soso` dentro de soso con fuentes vendidas.
# Copia `block-dev`, `sosofs` y `mkfs-soso` del repo a `rootfs/var/t42/mkfs/`,
# vende sus dependencias con el `Cargo.lock` fijado aquí (offline: salen de la
# caché de cargo del host) y escribe `MANIFEST.sha256` de lo copiado y vendido.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
SRC="$ROOT/tests/self-improvement/native/mkfs"
DST="$ROOT/rootfs/var/t42/mkfs"
rm -rf "$DST"
mkdir -p "$DST/crates" "$DST/tools" "$DST/.cargo"
cp -r "$ROOT/crates/block-dev" "$ROOT/crates/sosofs" "$DST/crates/"
cp -r "$ROOT/tools/mkfs-soso" "$DST/tools/"
rm -rf "$DST"/crates/*/target "$DST"/tools/*/target
cp "$SRC/Cargo.toml" "$SRC/Cargo.lock" "$DST/"
cp "$SRC/.cargo/config.toml" "$DST/.cargo/"
( cd "$DST" && cargo vendor --offline --versioned-dirs --locked vendor > /dev/null )
( cd "$DST" && find . -type f ! -name MANIFEST.sha256 ! -path './vendor/*/target/*' | LC_ALL=C sort | xargs sha256sum > MANIFEST.sha256 )
echo "preparado: $DST ($(wc -l < "$DST/MANIFEST.sha256") ficheros, $(du -sh "$DST/vendor" | cut -f1) vendidos)"
