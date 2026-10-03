#!/usr/bin/env bash
# Prepara el fixture de T41 paso 5: compilar con cargo, dentro de soso, un crate
# real del repo (`crates/soso-abi`). Copia las fuentes a `rootfs/var/t41/real/`
# y escribe `MANIFEST.sha256` con el hash de cada fichero copiado.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../../.." && pwd)"
SRC="$ROOT/tests/self-improvement/native/cargo/real"
DST="$ROOT/rootfs/var/t41/real"
rm -rf "$DST"
mkdir -p "$DST"
cp -r "$SRC/Cargo.toml" "$SRC/stub-core" "$DST/"
mkdir -p "$DST/soso-abi"
cp -r "$ROOT/crates/soso-abi/Cargo.toml" "$ROOT/crates/soso-abi/src" "$DST/soso-abi/"
( cd "$DST" && find . -type f ! -name MANIFEST.sha256 | LC_ALL=C sort | xargs sha256sum > MANIFEST.sha256 )
echo "preparado: $DST ($(wc -l < "$DST/MANIFEST.sha256") ficheros)"
