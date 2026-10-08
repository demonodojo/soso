#!/usr/bin/env bash
# T42, perfil `qemu-lxdde`: deja en un directorio de trabajo lo que el guest necesita para
# construir `liblxdde.a` con el port **e1000e** y el kernel con la feature `lxdde`:
#   <dst>/lx/   raíz de `lxbuild` (lxdde/shim, lxdde/ports/e1000e y SÓLO las cabeceras de
#               Linux que ese port incluye de verdad, 12 ficheros: no los 1,5 GB del árbol)
#   <dst>/lxb/  el programa `lxbuild` (copia de xtask/src/lx_build.rs) para compilar en el guest
#
# Las cabeceras se averiguan compilando el port UNA vez en el host con el mismo clang y leyendo
# los `.d`: es la clausura exacta. Ampliar a otro port (iwlwifi, nouveau, ath11k) es repetirlo
# con ese port; sus fuentes y cabeceras son muchos más ficheros.
#
# Uso: preparar.sh <dst> <clang-del-host> [puerto]     (por defecto e1000e)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
DST="$(realpath -m "$1")"; CLANG="$(realpath "$2")"; PORT="${3:-e1000e}"
HOST="$DST/.referencia-host"
rm -rf "$DST"; mkdir -p "$DST/lx/lxdde/ports" "$DST/lxb" "$HOST/lxdde/ports"
( cd "$ROOT/tests/self-improvement/native/lxdde/lxbuild" && cargo build -q )
LXBUILD="$ROOT/tests/self-improvement/native/lxdde/lxbuild/target/debug/lxbuild"
# Referencia en el host (con el árbol Linux completo por enlace simbólico).
cp -r "$ROOT/lxdde/shim" "$HOST/lxdde/"; cp -r "$ROOT/lxdde/ports/$PORT" "$HOST/lxdde/ports/"
ln -s "$ROOT/lxdde/linux" "$HOST/lxdde/linux"
( cd "$HOST" && SOSO_LX_ROOT="$HOST" LX_CC="$CLANG" "$LXBUILD" "$PORT" )
# Clausura de cabeceras.
cp -r "$ROOT/lxdde/shim" "$DST/lx/lxdde/"; cp -r "$ROOT/lxdde/ports/$PORT" "$DST/lx/lxdde/ports/"
rm -f "$DST/lx/lxdde/shim/src/generated_dummies.c"
cat "$HOST"/target/lxdde/*.d | tr ' \\' '\n\n' | grep "^$HOST/lxdde/linux/" | sort -u | sed "s|^$HOST/||" |
while read -r f; do mkdir -p "$DST/lx/$(dirname "$f")"; cp -L "$ROOT/$f" "$DST/lx/$f"; done
# Fuentes del propio Linux que lista el port (p. ej. lib/crc8.c) y el Makefile que `lxbuild` busca.
{ grep -hv '^#\|^lxdde/\|^$' "$ROOT/lxdde/ports/$PORT/source.list" || true; } | while read -r f; do
  mkdir -p "$DST/lx/lxdde/linux/$(dirname "$f")"; cp "$ROOT/lxdde/linux/$f" "$DST/lx/lxdde/linux/$f"; done
: > "$DST/lx/lxdde/linux/Makefile"
cp -r "$ROOT/tests/self-improvement/native/lxdde/lxbuild/Cargo.toml" "$ROOT/tests/self-improvement/native/lxdde/lxbuild/src" \
     "$ROOT/tests/self-improvement/native/lxdde/lxbuild/.cargo" "$DST/lxb/"
echo "preparado: $DST/lx ($(du -sh "$DST/lx" | cut -f1)), $DST/lxb; referencia del host en $HOST/target/lxdde"
