#!/usr/bin/env bash
# T42, C-117/C-120: las etapas del cargador (`bootloader` 0.11.15) compiladas DENTRO de soso.
#
# `bootloader` las compila en su `build.rs` con `cargo install` + `-Zbuild-std` para targets
# de 16/32/64 bits y UEFI, enlaza con `rust-lld` y aplana con `llvm-objcopy`. Aquí se trae el
# código de las cinco crates (boot sector, etapas 2/3/4 y `bootloader-x86_64-uefi`) con sus
# `Cargo.lock`, sus JSON de target y sus scripts de enlace, y se vende lo necesario; el guest
# las compila con su rustc, enlaza con el `lld` hecho para soso y aplana con `llvm-objcopy`.
#
# Cambio de plataforma: los JSON de las etapas 3 y 4 piden `"rustc-abi": "x86-softfloat"`, que
# el rustc del guest (7-sep-2026) ya no acepta (se quitó el 5-jul-2026): se escribe `softfloat`.
#
# Uso: preparar.sh <dst> [dir-rust-src-con-library]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
DST="$(realpath -m "$1")"
SRC_LIB="${2:-$ROOT/rootfs/lib/rustlib/src/rust/library}"
REG="$(ls -d "$HOME"/.cargo/registry/src/*/ | head -1)"
V=0.11.15
rm -rf "$DST"; mkdir -p "$DST/targets"
declare -A CRATES=( [boot-sector]=bootloader-x86_64-bios-boot-sector [stage-2]=bootloader-x86_64-bios-stage-2
                    [stage-3]=bootloader-x86_64-bios-stage-3 [stage-4]=bootloader-x86_64-bios-stage-4
                    [uefi]=bootloader-x86_64-uefi )
for d in "${!CRATES[@]}"; do
  cp -r "$REG/${CRATES[$d]}-$V" "$DST/$d"; rm -f "$DST/$d/.cargo-ok"
done
# `x86_64` 0.15.2 (el de los Cargo.lock de las etapas 4 y UEFI) no compila con el rustc del guest:
# el trait `Step` de `core` ganó `forward_overflowing`/`backward_overflowing`. 0.15.5 sí (es el
# que usa el kernel).
for d in stage-4 uefi; do
  ( cd "$DST/$d" && cargo +nightly-2026-07-01 update -p x86_64 --precise 0.15.5 > /dev/null )
done
for j in i386-code16-boot-sector i386-code16-stage-2 i686-stage-3 x86_64-stage-4; do
  sed 's/"x86-softfloat"/"softfloat"/' "$REG/bootloader-$V/$j.json" > "$DST/targets/$j.json"
done
# Configuración de cada crate: vendor, build-std y el lld del guest.
cfg() { # dir target-json-o-triple triple-name
  mkdir -p "$DST/$1/.cargo"
  cat > "$DST/$1/.cargo/config.toml" <<CFG
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "../vendor"

[build]
incremental = false
# `core` de los targets de 32 bits emite miles de avisos y su salida JSON (decenas de MB por
# una tubería) hacía fallar a cargo con EFAULT: se silencian.
rustflags = ["-Awarnings"]
target = "$2"

[unstable]
json-target-spec = true
build-std = ["core"]
build-std-features = ["compiler-builtins-mem"]

[target.$3]
linker = "$4"
CFG
}
G=/var/t42/cg
cfg boot-sector "$G/targets/i386-code16-boot-sector.json" i386-code16-boot-sector /var/t42/llvm/bin/ld.lld-soso x
cfg stage-2     "$G/targets/i386-code16-stage-2.json"     i386-code16-stage-2     /var/t42/llvm/bin/ld.lld-soso x
cfg stage-3     "$G/targets/i686-stage-3.json"            i686-stage-3            /var/t42/llvm/bin/ld.lld-soso x
cfg stage-4     "$G/targets/x86_64-stage-4.json"          x86_64-stage-4          /var/t42/llvm/bin/ld.lld-soso x
cfg uefi        "x86_64-unknown-uefi"                     x86_64-unknown-uefi     /var/t42/llvm/bin/lld-link-soso x
# El envoltorio del lld (se compila en el guest: ver C-142): `cg/lldwrap`.
cp -r "$ROOT/tests/self-improvement/native/cargador/lldwrap" "$DST/lldwrap"
# Las etapas 3 y 4 piden `debug = 2`: sólo agranda la salida de cargo (`objcopy -O binary` la descarta).
for d in stage-3 stage-4; do printf '\n[profile.%s]\ndebug = 0\n' "$d" >> "$DST/$d/.cargo/config.toml"; done
# El LLVM del guest convierte un bucle de `uefi` en una llamada a `wcslen` (reconocimiento de
# idiomas de LLVM ≥ 21) y el cargador UEFI no enlaza con libc. Se define aquí; la lectura es
# `volatile` para que el propio bucle no se reconozca otra vez como `wcslen` y se llame a sí mismo.
cat >> "$DST/uefi/src/main.rs" <<'RS'

/// `wcslen` para el enlace sin libc (ver `tests/self-improvement/native/cargador/preparar.sh`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcslen(s: *const u16) -> usize {
    let mut n = 0;
    while unsafe { core::ptr::read_volatile(s.add(n)) } != 0 {
        n += 1;
    }
    n
}
RS
# Vendor (en el host; usa red una vez): las dependencias de las cinco crates y las de `core`.
( cd "$DST/boot-sector" && cargo +nightly-2026-07-01 vendor --versioned-dirs \
    -s "$DST/stage-2/Cargo.toml" -s "$DST/stage-3/Cargo.toml" -s "$DST/stage-4/Cargo.toml" \
    -s "$DST/uefi/Cargo.toml" -s "$SRC_LIB/Cargo.toml" "$DST/vendor" > /dev/null )
echo "preparado: $DST ($(du -sh "$DST" | cut -f1); $(ls "$DST/vendor" | wc -l) crates vendidas)"
