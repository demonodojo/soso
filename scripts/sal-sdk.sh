#!/usr/bin/env bash
# SDK freestanding CPU de Sal. Los artefactos solo se escriben en target/.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
sal=${SOSO_SAL_SOURCE:-"$repo/../sal"}
out="$repo/target/sal/sdk"
mkdir -p "$out"
cc=${SAL_HOST_CC:-clang}
lld=$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld
(cd "$repo/user" && cargo build --release -p sal-runtime)
flags=(--target=x86_64-unknown-none-elf -DSAL_SOSO -ffreestanding -fno-builtin
       -fno-stack-protector -mno-red-zone -ffunction-sections -fdata-sections -O2)
for part in sal_runtime kernels soso/format soso/memory; do
    "$cc" "${flags[@]}" -c "$sal/runtime/$part.c" -o "$out/${part##*/}.o"
done
"$lld" -flavor gnu -r "$out/sal_runtime.o" "$out/format.o" "$out/memory.o" -o "$out/runtime.o"
cp "$repo/user/target/x86_64-soso-user/release/libsal_runtime.a" "$out/libsal-soso.a"
cp "$repo/user/link.ld" "$out/link.ld"
cp "$sal/runtime/sal_runtime.h" "$out/sal_runtime.h"
(
    cd "$out"
    echo 'sal-soso-sdk=1'
    git -C "$sal" rev-parse HEAD
    sha256sum runtime.o kernels.o libsal-soso.a link.ld sal_runtime.h
) > "$out/manifest.txt"
printf 'SDK de Sal: %s\n' "$out"
