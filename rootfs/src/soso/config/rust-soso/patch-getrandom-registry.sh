#!/usr/bin/env bash
# C-068/C-069: el bootstrap de cargo regenera el lock y usa ~/.cargo/registry.
# Sobrescribimos crates parcheadas en config/rust-soso/vendor/.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
REG="${CARGO_HOME:-$HOME/.cargo}/registry/src"
shopt -s nullglob
for spec in "0.2.17:getrandom-0.2" "0.3.4:getrandom-0.3" "0.4.3:getrandom-0.4"; do
  ver="${spec%%:*}"
  vend="${spec#*:}"
  src="$ROOT/config/rust-soso/vendor/$vend"
  [[ -d "$src" ]] || { echo "patch-getrandom-registry: falta $src" >&2; exit 1; }
  for index in "$REG"/*; do
    [[ -d "$index" ]] || continue
    dst="$index/getrandom-$ver"
    mkdir -p "$dst"
    cp -a "$src/." "$dst/"
  done
done

for crate_spec in "gix-config-value:gix-config-value-0.18.1" "gix-sec:gix-sec-0.14.1" "filetime:filetime-0.2.29"; do
  vend="${crate_spec%%:*}"
  crate_dir="${crate_spec#*:}"
  src="$ROOT/config/rust-soso/vendor/$vend"
  [[ -d "$src" ]] || { echo "patch-getrandom-registry: falta $src" >&2; exit 1; }
  for index in "$REG"/*; do
    [[ -d "$index" ]] || continue
    dst="$index/$crate_dir"
    mkdir -p "$dst"
    cp -a "$src/." "$dst/"
  done
done

VENDOR_LIBC="$ROOT/config/rust-soso/vendor/libc"
for dst in "$REG"/*/libc-*; do
  [[ -d "$dst" ]] || continue
  python3 "$ROOT/config/rust-soso/patch-libc-soso.py" "$dst" "$VENDOR_LIBC"
done

for dst in "$REG"/*/curl-sys-0.4.90*; do
  [[ -d "$dst" ]] || continue
  python3 "$ROOT/config/rust-soso/patch-curl-sys-soso.py" "$dst"
done

for dst in "$REG"/*/socket2-0.6.4; do
  [[ -d "$dst" ]] || continue
  python3 "$ROOT/config/rust-soso/patch-socket2-soso.py" "$dst"
done

for dst in "$REG"/*/curl-0.4.50; do
  [[ -d "$dst" ]] || continue
  python3 "$ROOT/config/rust-soso/patch-curl-soso.py" "$dst"
done

for dst in "$REG"/*/libssh2-sys-0.3.2; do
  [[ -d "$dst" ]] || continue
  python3 "$ROOT/config/rust-soso/patch-libssh2-sys-soso.py" "$dst"
done

for dst in "$REG"/*/libgit2-sys-0.18.7+1.9.6; do
  [[ -d "$dst" ]] || continue
  python3 "$ROOT/config/rust-soso/patch-libgit2-sys-soso.py" "$dst"
done

for dst in "$REG"/*/git2-0.21.0; do
  [[ -d "$dst" ]] || continue
  python3 "$ROOT/config/rust-soso/patch-git2-soso.py" "$dst"
done

for dst in "$REG"/*/is_executable-1.0.6; do
  [[ -d "$dst" ]] || continue
  python3 "$ROOT/config/rust-soso/patch-is_executable-soso.py" "$dst"
done

for dst in "$REG"/*/url-*; do
  [[ -d "$dst" ]] || continue
  python3 "$ROOT/config/rust-soso/patch-url-soso.py" "$dst"
done

for dst in "$REG"/*/tar-0.4.*; do
  [[ -d "$dst" ]] || continue
  python3 "$ROOT/config/rust-soso/patch-tar-soso.py" "$dst"
done

for dst in "$REG"/*/home-0.5.*; do
  [[ -d "$dst" ]] || continue
  python3 "$ROOT/config/rust-soso/patch-home-soso.py" "$dst"
done

# Cargo.lock pide 0.2.186; a veces sólo está 0.2.189 en el registry.
for index in "$REG"/*; do
  [[ -d "$index" ]] || continue
  want="$index/libc-0.2.186"
  [[ -d "$want" ]] && continue
  for seed in "$index"/libc-0.2.*; do
    [[ -d "$seed" ]] || continue
    mkdir -p "$want"
    cp -a "$seed/." "$want/"
    python3 "$ROOT/config/rust-soso/patch-libc-soso.py" "$want" "$VENDOR_LIBC"
    break
  done
done
