#!/usr/bin/env bash
# T42 / C-113: prepara en `rootfs/` lo que hace falta para compilar el kernel
# DENTRO de soso con `cargo build -Zbuild-std` y sin red:
#
# - `rootfs/lib/rustlib/src/rust/library`: las fuentes de `library/` del rust de
#   soso (el mismo árbol del que sale el rustc del guest);
# - `rootfs/var/t42/kd/`: `kernel/` y las crates por ruta de las que depende,
#   con TODAS las dependencias de crates.io (las del kernel y las de `library/`)
#   en `vendor/`, y `MANIFEST.sha256` de lo copiado.
#
# Uso: preparar.sh [ruta-al-vendor-de-rust]  (por defecto ~/.cache/soso-rust-vendor)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
RUST="${1:-${SOSO_RUST_VENDOR:-$HOME/.cache/soso-rust-vendor}}"
SRC_DST="$ROOT/rootfs/lib/rustlib/src/rust"
KD="$ROOT/rootfs/var/t42/kd"

mkdir -p "$SRC_DST"
rsync -a --delete --exclude target "$RUST/library/" "$SRC_DST/library/"
cp "$RUST/library/Cargo.lock" "$SRC_DST/library/Cargo.lock"
# `library/std` depende de las crates de soso por ruta absoluta del host: se llevan
# junto al árbol y se reescribe la ruta (el workspace de `library/` hay que poder
# cargarlo entero aunque sólo se compile `core`).
mkdir -p "$SRC_DST/crates" "$SRC_DST/vendor"
for c in soso-rt soso-alloc soso-abi; do
  rsync -a --delete --exclude target "$ROOT/crates/$c/" "$SRC_DST/crates/$c/"
done
rsync -a --delete --exclude target "$ROOT/vendor/linked_list_allocator/" "$SRC_DST/vendor/linked_list_allocator/"
sed -i "s|path = \"$ROOT/crates/soso-rt\"|path = \"../../crates/soso-rt\"|" "$SRC_DST/library/std/Cargo.toml"

rm -rf "$KD"
mkdir -p "$KD/crates" "$KD/vendor-propio" "$KD/kernel"
cp -r "$ROOT/kernel/." "$KD/kernel/"
rm -rf "$KD/kernel/target"
for c in sosofs sosomfs sosomodel block-dev gptdisk soso-resize-core soso-log-core espfat-core \
         soso-update-core soso-abi soso-hw soso-wpa2 xhci-nostd; do
  cp -r "$ROOT/crates/$c" "$KD/crates/$c"
  rm -rf "$KD/crates/$c/target"
done
cp -r "$ROOT/vendor/sunset-0.5.0" "$KD/vendor-propio/sunset-0.5.0"
# Blobs SASS que el kernel incrusta con `include_bytes!("../../../lxdde/…")`.
for sm in sm_86 sm_120; do
  mkdir -p "$KD/lxdde/ports/nouveau/sass/$sm"
  cp "$ROOT/lxdde/ports/nouveau/sass/$sm/"{saxpy,matvec}.sass.bin "$KD/lxdde/ports/nouveau/sass/$sm/"
done
# Las rutas relativas del kernel (`../crates`, `../vendor/sunset-0.5.0`) se conservan.
mv "$KD/vendor-propio" "$KD/vendor"
mkdir -p "$KD/vendor-crates"

cd "$KD/kernel"
# Vende las dependencias del kernel y las de std (`-s`) en un solo directorio.
# Esto es provisión en el HOST y usa la red (los crates que `library/` pide para
# otros targets, p. ej. dlmalloc, no están en la caché); lo que se copia al guest
# queda fijado por los dos Cargo.lock y se compila sin red.
cargo vendor --versioned-dirs --locked -s "$SRC_DST/library/Cargo.toml" "$KD/vendor-crates" > /dev/null
cat >> .cargo/config.toml <<'EOF'

[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "../vendor-crates"

# El guest no tiene `rust-lld`: se enlaza con su `wild-soso` (C-114).
[target.x86_64-soso]
linker = "wild-soso"
EOF
# soso no ofrece bloqueos de fichero y rustc los exige para el directorio
# incremental ("file locks not supported on this platform").
sed -i 's/^\[build\]$/[build]\nincremental = false/' .cargo/config.toml
cd "$KD"
find . -type f ! -path './vendor-crates/*' ! -name MANIFEST.sha256 | LC_ALL=C sort | xargs sha256sum > MANIFEST.sha256
echo "preparado: $KD ($(du -sh vendor-crates | cut -f1) vendidos); rust-src: $(du -sh "$SRC_DST" | cut -f1)"
