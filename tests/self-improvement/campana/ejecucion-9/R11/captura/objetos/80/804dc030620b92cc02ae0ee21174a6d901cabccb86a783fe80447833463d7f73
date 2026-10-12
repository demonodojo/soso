#!/usr/bin/env bash
# Aplica parches de soso a vendor/rust (PAL, build.rs, exit, Cargo.toml).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RUST="${SOSO_RUST_VENDOR:-${XDG_CACHE_HOME:-$HOME/.cache}/soso-rust-vendor}"
TREE="$ROOT/config/rust-soso/tree"

if [[ ! -d "$RUST/library/std" ]]; then
  echo "apply-patches: falta $RUST — ejecuta scripts/soso-rust-bootstrap.sh" >&2
  exit 1
fi

mkdir -p "$RUST/library/std/src/os/soso" "$RUST/library/std/src/sys/pal/soso"
rsync -a "$TREE/library/std/src/os/soso/" "$RUST/library/std/src/os/soso/"
rsync -a "$TREE/library/std/src/sys/pal/soso/" "$RUST/library/std/src/sys/pal/soso/"
if [[ -f "$ROOT/config/rust-soso/sys/pal/soso/dl.rs" ]]; then
  cp "$ROOT/config/rust-soso/sys/pal/soso/dl.rs" "$RUST/library/std/src/sys/pal/soso/dl.rs"
fi

grep -q 'target_os == "soso"' "$RUST/library/std/build.rs" || \
  sed -i 's/|| target_os == "vexos"/|| target_os == "vexos"\n        || target_os == "soso"/' \
    "$RUST/library/std/build.rs"

grep -q 'target_os = "soso"' "$RUST/library/std/src/sys/pal/mod.rs" || \
  perl -i -0pe 's/(\s+target_os = "zkvm" => \{\s+mod zkvm;\s+pub use self::zkvm::\*;\s+\})\s+_ =>/\1\n    target_os = "soso" => {\n        mod soso;\n        pub use self::soso::*;\n    }\n    _ =>/s' \
    "$RUST/library/std/src/sys/pal/mod.rs"

grep -q 'pub mod soso' "$RUST/library/std/src/os/mod.rs" || \
  sed -i '/#\[cfg(target_os = "hermit"\)\]/a\
#[cfg(target_os = "soso")]\
pub mod soso;' "$RUST/library/std/src/os/mod.rs"

grep -q 'target_os = "soso",' "$RUST/library/std/src/os/mod.rs" || \
  sed -i 's/target_os = "hermit",/target_os = "hermit",\n    target_os = "soso",/' \
    "$RUST/library/std/src/os/mod.rs"

grep -q 'target_os = "soso"' "$RUST/library/std/src/sys/exit.rs" || \
  sed -i 's/target_os = "hermit" => unsafe { hermit_abi::exit(code) },/target_os = "hermit" => unsafe { hermit_abi::exit(code) },\n        target_os = "soso" => soso_rt::exit(code),/' \
    "$RUST/library/std/src/sys/exit.rs"

grep -q "target_os = \"soso\"" "$RUST/library/std/Cargo.toml" || cat >>"$RUST/library/std/Cargo.toml" <<EOF

[target.'cfg(target_os = "soso")'.dependencies]
soso-rt = { path = "$ROOT/crates/soso-rt", features = ["dep-of-std"] }
EOF

grep -q 'target_os = "soso"' "$RUST/library/std/src/sys/env_consts.rs" || \
  perl -i -0pe 's/(#\[cfg\(target_os = "hermit"\)\]\npub mod os \{)/#[cfg(target_os = "soso")]\npub mod os {\n    pub const FAMILY: \&str = "unix";\n    pub const OS: \&str = "soso";\n    pub const ARCH: \&str = env!("STD_ENV_ARCH");\n}\n\n$1/s' \
    "$RUST/library/std/src/sys/env_consts.rs"

echo "apply-patches: OK → $RUST"
