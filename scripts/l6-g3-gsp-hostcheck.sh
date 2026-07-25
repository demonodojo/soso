#!/usr/bin/env bash
# L6 G3b — verifica en el host los pasos que preceden al COT: gsp_rm.c (ELF del
# ucode GSP-RM + tabla radix3) y gsp_wpr.c (bootloader RISC-V + GspFwWprMeta),
# contra el firmware real y sin GPU ni QEMU.
#
# El bucle de prueba natural de este código es lento y caro (sudo + VFIO + ~90 s
# de arranque), así que los mismos fuentes se compilan aquí con la capa lx y el
# MMIO simulados (tools/gsp-hostcheck/main.c) y se les pasan los blobs de verdad.
# No sustituye a la prueba en hardware: valida parseo y aritmética, no arranque.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$root/lxdde/ports/nouveau"
out="$root/target/gsp-hostcheck"
fwdir="${1:-$root/rootfs/lib/firmware/nvidia/gb205/gsp}"
ucode="$fwdir/gsp-570.144.bin"
boot="$fwdir/bootloader-570.144.bin"

for f in "$ucode" "$boot"; do
    if [[ ! -f "$f" ]]; then
        echo "falta $f — ./scripts/l6-pack-firmware.sh" >&2
        exit 1
    fi
done

mkdir -p "$out"

# El banco incluye los módulos tal cual, sin sus cabeceras (las tipa él mismo).
strip_module() {
    grep -v '^#include' "$1" |
        grep -v "^#ifndef ${2}$" | grep -v "^#define ${2}$" | grep -v '^#endif' |
        grep -v '^void \*memcpy' | grep -v '^void \*memset'
}

{ strip_module "$src/gsp_rm.h" GSP_RM_H; strip_module "$src/gsp_rm.c" GSP_RM_H; } \
    > "$out/gsp_rm_body.inc"
{ strip_module "$src/gsp_wpr.h" GSP_WPR_H; strip_module "$src/gsp_wpr.c" GSP_WPR_H; } \
    > "$out/gsp_wpr_body.inc"

cc -O1 -Wall -Wextra -Wno-unused-parameter -Wno-unused-function \
   -I"$out" -o "$out/hostcheck" "$root/tools/gsp-hostcheck/main.c"

echo "=== L6 G3b — pasos 3 y 4 sobre el firmware real ==="
"$out/hostcheck" "$ucode" "$boot"
