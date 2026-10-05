#!/usr/bin/env bash
# T42, C-115/C-116/C-117: enlaza las herramientas de LLVM (clang, llvm-ar, llvm-objcopy…)
# COMO PROGRAMAS DE SOSO a partir de un build normal de LLVM en el host.
#
# CMake enlaza para Linux (dinámico, glibc); el guest no tiene cargador. Se reaprovecha
# lo que CMake ya compiló: los objetos de la herramienta (con su `main` renombrado a
# `herramienta_main`) y las bibliotecas estáticas de su `link.txt`, y se enlazan con un
# `main` de Rust (el runtime de soso: TLS, argv, stack…) con el rustc cruzado y
# `wild-soso`, igual que `rustc` y `cargo`.
#
# Uso: enlazar.sh <build-llvm> <destino> <herramienta:directorio-cmake>...
#   p. ej. enlazar.sh build out llvm-ar:tools/llvm-ar clang:tools/clang/tools/driver
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
B="$(realpath "$1")"; OUT="$(realpath -m "$2")"; shift 2
V="${SOSO_RUST_VENDOR:-$HOME/.cache/soso-rust-vendor}"
RUSTC="$V/build-soso/x86_64-unknown-linux-gnu/stage1/bin/rustc"
# La `std` de soso MÁS RECIENTE (la que se despliega en el guest): la del stage1 puede ser
# anterior al arreglo del TCB (`%fs:0x10`) que glibc necesita.
SYSROOT="$OUT/sysroot"
mkdir -p "$SYSROOT/lib/rustlib"
ln -sfn "$V/build-soso/x86_64-unknown-linux-gnu/ci-rustc-sysroot/lib/rustlib/x86_64-unknown-soso" "$SYSROOT/lib/rustlib/x86_64-unknown-soso"
mkdir -p "$OUT"
AQUI=$(dirname "$0")
cp "$AQUI/lanzador.rs" "$AQUI/shims.rs" "$OUT/"
for spec in "$@"; do
  tool="${spec%%:*}"; dir="${spec#*:}"
  link="$B/$dir/CMakeFiles/$tool.dir/link.txt"
  [ -f "$link" ] || { echo "no hay $link" >&2; exit 1; }
  w="$OUT/$tool.obj"; rm -rf "$w"; mkdir -p "$w"
  objs=(); libs=()
  # El primer token es el compilador; hasta `-o` van opciones y objetos; después, bibliotecas.
  read -ra toks <<<"$(tr -d '"' < "$link")"
  seen_o=0
  for ((i=1; i<${#toks[@]}; i++)); do
    t="${toks[$i]}"
    case "$t" in
      -o) seen_o=1; i=$((i+1)); continue ;;
      *.o) objs+=("$B/$dir/$t") ;;
      *.a) case "$t" in /*) libs+=("$t") ;; *) libs+=("$B/$dir/$t") ;; esac ;;
    esac
  done
  # `main` → `herramienta_main` en el objeto que lo define.
  n=0
  for o in "${objs[@]}"; do
    c="$w/$(printf %03d $n).o"; cp "$o" "$c"
    # (sin `grep -q` en tubería: con pipefail, el SIGPIPE de `nm` daba falso negativo)
    simbolos="$(nm "$c" 2>/dev/null || true)"
    if grep -E ' T main$' <<<"$simbolos" >/dev/null; then objcopy --redefine-sym main=herramienta_main "$c"; fi
    n=$((n+1))
  done
  rm -f "$w/libobjs.a"; ar rcs "$w/libobjs.a" "$w"/*.o
  args=(-Zunstable-options --sysroot "$SYSROOT" --edition 2021 --target x86_64-unknown-soso -C linker=wild-soso -C panic=abort
        -C opt-level=2 -C link-arg=--allow-multiple-definition -L "$w" -l static=objs)
  for l in "${libs[@]}"; do args+=(-C "link-arg=$l"); done
  args+=(-l stdc++ -l m -l dl -l pthread -l rt)
  RUST_TARGET_PATH="$V" PATH="$ROOT/target/release:$HOME/.cargo/bin:$PATH" \
    "$RUSTC" "${args[@]}" "$OUT/lanzador.rs" -o "$OUT/$tool"
  echo "enlazado: $OUT/$tool ($(du -h "$OUT/$tool" | cut -f1))"
done
