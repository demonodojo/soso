#!/usr/bin/env bash
# L6 G3b — verifica gsp_rm.c (ELF del ucode GSP-RM + tabla radix3) en el host,
# contra el firmware real y sin GPU ni QEMU.
#
# El bucle de prueba natural de este código es lento y caro (sudo + VFIO + ~90 s
# de arranque), así que el mismo fuente se compila aquí con la capa lx simulada
# (tools/gsp-rm-hostcheck/main.c) y se le pasa el blob de verdad. No sustituye a
# la prueba en hardware: valida el parseo y la aritmética, no el arranque.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$root/lxdde/ports/nouveau"
out="$root/target/gsp-rm-hostcheck"
fw="${1:-$root/rootfs/lib/firmware/nvidia/gb205/gsp/gsp-570.144.bin}"

if [[ ! -f "$fw" ]]; then
    echo "falta el ucode $fw — ./scripts/l6-pack-firmware.sh" >&2
    exit 1
fi

mkdir -p "$out"

# El banco incluye el módulo tal cual, sin sus cabeceras (las tipa él mismo).
{
    grep -v '^#include' "$src/gsp_rm.h" |
        grep -v '^#ifndef GSP_RM_H' | grep -v '^#define GSP_RM_H$' | grep -v '^#endif'
    grep -v '^#include' "$src/gsp_rm.c" |
        grep -v '^void \*memcpy' | grep -v '^void \*memset'
} > "$out/gsp_rm_body.inc"

cc -O1 -Wall -Wextra -Wno-unused-parameter \
   -I"$out" -o "$out/hostcheck" "$root/tools/gsp-rm-hostcheck/main.c"

echo "=== L6 G3b — radix3 sobre $(basename "$fw") ==="
"$out/hostcheck" "$fw"
