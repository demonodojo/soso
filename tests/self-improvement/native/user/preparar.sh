#!/usr/bin/env bash
# T42, paso 5b: prepara en `rootfs/var/t42/ud/` el workspace `user/` (sin las
# `soso-agent-probe`, que compila C ajeno; soso-hf, soso-update y soso-web entran con el
# proveedor criptográfico en Rust puro, sin `ring`)
# para compilarlo DENTRO de soso con `cargo build -Zbuild-std` sin red.
# Reutiliza `rootfs/lib/rustlib/src` (lo deja `../kernel/preparar.sh`).
#
# Uso: preparar.sh [miembros...]  (por defecto: libsoso init sosh coreutils soso-ed soso-hf soso-update soso-web soso-agent-probe)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
RUST="${SOSO_RUST_VENDOR:-$HOME/.cache/soso-rust-vendor}"
SRC_DST="$ROOT/rootfs/lib/rustlib/src/rust"
UD="$ROOT/rootfs/var/t42/ud"
MIEMBROS=("$@")
[ ${#MIEMBROS[@]} -gt 0 ] || MIEMBROS=(libsoso init sosh coreutils soso-ed soso-hf soso-update soso-web soso-agent-probe)
[ -d "$SRC_DST/library" ] || { echo "falta $SRC_DST: corre ../kernel/preparar.sh antes" >&2; exit 1; }

rm -rf "$UD"
mkdir -p "$UD/user" "$UD/crates" "$UD/vendor-crates"
for m in "${MIEMBROS[@]}"; do
  cp -r "$ROOT/user/$m" "$UD/user/$m"
  rm -rf "$UD/user/$m/target"
done
cp "$ROOT/user/link.ld" "$ROOT/user/x86_64-soso-user.json" "$UD/user/"
# Workspace recortado: mismos `[workspace.dependencies]` y perfiles.
python3 - "$ROOT/user/Cargo.toml" "$UD/user/Cargo.toml" "${MIEMBROS[@]}" <<'PY'
import re, sys
src, dst, *miembros = sys.argv[1:]
t = open(src).read()
lista = ", ".join(f'"{m}"' for m in miembros)
t = re.sub(r'members = \[[^\]]*\]', f'members = [{lista}]', t, count=1)
open(dst, "w").write(t)
PY
# Crates por ruta que alcanzan los miembros (cierre transitivo).
# `soso-http` con `ring` necesita C y ensamblador (C-115/C-116): en el guest se pide
# su proveedor en Rust puro (RustCrypto).
for m in "${MIEMBROS[@]}"; do
  sed -i 's|soso-http = { path = "../../crates/soso-http" }|soso-http = { path = "../../crates/soso-http", default-features = false, features = ["crypto-rust"] }|' "$UD/user/$m/Cargo.toml"
done
copiar() {
  local c="$1"
  [ -d "$UD/crates/$c" ] && return 0
  cp -r "$ROOT/crates/$c" "$UD/crates/$c"; rm -rf "$UD/crates/$c/target"
  for d in $(grep -ho 'path = "\.\./[a-z0-9-]*"' "$UD/crates/$c/Cargo.toml" | sed 's|path = "\.\./||;s|"||'); do copiar "$d"; done
}
for m in "${MIEMBROS[@]}"; do
  for d in $(grep -ho 'path = "\.\./\.\./crates/[a-z0-9-]*"' "$UD/user/$m/Cargo.toml" | sed 's|.*crates/||;s|"||'); do copiar "$d"; done
done
for c in soso-abi; do copiar "$c"; done
# `soso-alloc` lleva `linked_list_allocator` por `../../vendor/`.
mkdir -p "$UD/vendor"
cp -r "$ROOT/vendor/linked_list_allocator" "$UD/vendor/linked_list_allocator"
rm -rf "$UD/vendor/linked_list_allocator/target"
# `[workspace.dependencies]` apunta a ../crates/soso-abi: ya cuadra con `$UD/crates`.
mkdir -p "$UD/user/.cargo"
cat > "$UD/user/.cargo/config.toml" <<'CFG'
# Igual que user/.cargo/config.toml, más el vendor y el enlazador del guest.
[build]
incremental = false
target = "x86_64-soso-user.json"

[unstable]
json-target-spec = true
build-std = ["core", "alloc", "compiler_builtins"]
build-std-features = ["compiler-builtins-mem"]

[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "../vendor-crates"

[target.x86_64-soso-user]
linker = "wild-soso"
rustflags = [
    "-C", "link-arg=-Tlink.ld",
    "-C", "relocation-model=static",
]
CFG
cd "$UD/user"
cp "$ROOT/user/Cargo.lock" Cargo.lock   # fija las versiones del repo
# Provisión en el HOST (usa red una vez); lo copiado queda fijado por Cargo.lock.
cargo vendor --versioned-dirs -s "$SRC_DST/library/Cargo.toml" "$UD/vendor-crates" > /dev/null
# La crate `cc` no conoce `target_os = "soso"` («cc cannot create named tempfile»). Se acepta
# en su rama de ficheros temporales y se rehace el `.cargo-checksum.json` del vendor.
python3 - "$UD/vendor-crates" <<'PY'
import glob, hashlib, json, sys
for d in glob.glob(sys.argv[1] + "/cc-*"):
    f = d + "/src/tempfile.rs"
    t = open(f).read()
    t2 = t.replace('#[cfg(not(any(unix, target_family = "wasm", windows)))]',
                   '#[cfg(not(any(unix, target_os = "soso", target_family = "wasm", windows)))]')
    if t2 != t:
        open(f, "w").write(t2)
        c = json.load(open(d + "/.cargo-checksum.json"))
        c["files"]["src/tempfile.rs"] = hashlib.sha256(t2.encode()).hexdigest()
        json.dump(c, open(d + "/.cargo-checksum.json", "w"))
PY
# Compiladores del guest (C-140): `cc` los toma de [env].
cat >> "$UD/user/.cargo/config.toml" <<'CFG'

[env]
CC = { value = "/var/t42/llvm/clang", force = true }
AR = { value = "/var/t42/llvm/llvm-ar", force = true }
CFLAGS = { value = "-resource-dir /var/t42/llvm/lib/clang/23", force = true }
CRATE_CC_NO_DEFAULTS = { value = "1", force = true }
CFG
cd "$UD"
find . -type f ! -path './vendor-crates/*' ! -name MANIFEST.sha256 | LC_ALL=C sort | xargs sha256sum > MANIFEST.sha256
echo "preparado: $UD ($(du -sh vendor-crates | cut -f1) vendidos, $(ls crates | wc -l) crates por ruta)"
