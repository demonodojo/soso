#!/usr/bin/env bash
# Prepara rust-lang/rust con PAL soso (Hito 3a).
#
# Por defecto clona en ~/.cache/soso-rust-vendor (fuera del workspace soso,
# necesario para que x.py/cargo resuelvan el workspace de rust).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VENDOR="${SOSO_RUST_VENDOR:-${XDG_CACHE_HOME:-$HOME/.cache}/soso-rust-vendor}"

if [[ ! -d "$VENDOR/.git" ]]; then
  echo "soso-rust-bootstrap: clonando rust-lang/rust → $VENDOR"
  GIT_TERMINAL_PROMPT=0 git clone --depth 1 https://github.com/rust-lang/rust "$VENDOR"
fi

cp "$ROOT/targets/x86_64-unknown-soso.json" "$VENDOR/x86_64-unknown-soso.json"

cat >"$VENDOR/config.toml" <<EOF
# Generado por scripts/soso-rust-bootstrap.sh
[build]
host = ["x86_64-unknown-linux-gnu"]
target = ["x86_64-unknown-soso"]
build-dir = "build-soso"
install = false
extended = false

[rust]
codegen-backends = ["llvm"]
channel = "dev"
download-rustc = true

[target.x86_64-unknown-soso]
linker = "$ROOT/tools/wild-soso/target/release/wild-soso"

[llvm]
download-ci-llvm = false
EOF

SOSO_RUST_VENDOR="${SOSO_RUST_VENDOR:-${XDG_CACHE_HOME:-$HOME/.cache}/soso-rust-vendor}"
bash "$ROOT/config/rust-soso/apply-patches.sh"

( cd "$ROOT" && cargo build -q -p sosoas -p wild-soso -p soso-rt 2>/dev/null ) || \
  echo "soso-rust-bootstrap: compila sosoas, wild-soso y soso-rt en el host"

cat <<EOF

Vendor: $VENDOR
Parches aplicados. Compilar libstd:

  cd "$VENDOR"
  export PATH="$ROOT/tools/sosoas/target/release:$ROOT/tools/wild-soso/target/release:\$PATH"
  ./x.py build library/std --target x86_64-unknown-soso

O desde la raíz del repo:

  cargo xtask rust-build-std

Sysroot guest: /usr/lib/rustlib/x86_64-unknown-soso/
EOF
