#!/usr/bin/env bash
# Bootstrap cruzado del compilador Sal y su backend LLVM nativo.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
sal=${SOSO_SAL_SOURCE:-"$repo/../sal"}
vendor=${SOSO_RUST_VENDOR:-"$HOME/.cache/soso-rust-vendor"}
out="$repo/target/sal/native"
mkdir -p "$out/sysroot/lib/rustlib"
rustc="$vendor/build-soso/x86_64-unknown-linux-gnu/stage1/bin/rustc"
syslib="$vendor/build-soso/x86_64-unknown-linux-gnu/ci-rustc-sysroot/lib/rustlib/x86_64-unknown-soso"
test -x "$rustc"
test -d "$syslib/lib"
ln -sfn "$syslib" "$out/sysroot/lib/rustlib/x86_64-unknown-soso"
export PATH="$repo/target/release:$PATH"
export RUST_TARGET_PATH="$vendor"
llvm=${SAL_LLVM_CONFIG:-/usr/lib/llvm-19/bin/llvm-config}
test -x "$llvm"
test "$("$llvm" --version | cut -d. -f1)" = 19
clang $("$llvm" --cflags) -O2 -c "$repo/tools/sal-llc/main.c" -o "$out/llc.o"
ar rcs "$out/libsal_llc.a" "$out/llc.o"
cp "$repo/tests/self-improvement/native/llvm/"{lanzador.rs,shims.rs} "$out/"
args=(-Zunstable-options --sysroot "$out/sysroot" --edition 2021 --target x86_64-unknown-soso
      -C linker=wild-soso -C panic=abort -C opt-level=2 -C link-arg=--allow-multiple-definition
      -L "$out" -l static=sal_llc)
for lib in $("$llvm" --link-static --libfiles core irreader nativecodegen); do args+=(-C "link-arg=$lib"); done
for lib in $("$llvm" --link-static --system-libs); do args+=(-C "link-arg=$lib"); done
args+=(-l stdc++ -l m -l dl -l pthread -l rt)
"$rustc" "${args[@]}" "$out/lanzador.rs" -o "$out/sal-llc"
# Host procedural macros stay on Linux; ordinary dependencies use soso's std.
RUSTC="$rustc" CARGO_TARGET_DIR="$out/cargo" \
 CARGO_TARGET_X86_64_UNKNOWN_SOSO_RUSTFLAGS="-Zunstable-options --sysroot=$out/sysroot -C linker=wild-soso -C panic=abort" \
 cargo build --manifest-path "$sal/Cargo.toml" --target x86_64-unknown-soso --release --bin sal
cp "$out/cargo/x86_64-unknown-soso/release/sal" "$out/sal"
printf 'Compilador Sal y LLVM nativos: %s\n' "$out"
