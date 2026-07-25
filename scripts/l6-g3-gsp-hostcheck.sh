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
fmc="$fwdir/fmc-570.144.bin"

for f in "$ucode" "$boot" "$fmc"; do
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

# Orden = orden de dependencias entre módulos.
for m in gsp_dma gsp_rm gsp_wpr gsp_libos fmc_lx fsp_lx gsp_rpc gsp_cmdq; do
    guard="$(echo "$m" | tr '[:lower:]' '[:upper:]')_H"
    { strip_module "$src/$m.h" "$guard"; strip_module "$src/$m.c" "$guard"; } \
        > "$out/${m}_body.inc"
done

cc -O1 -Wall -Wextra -Wno-unused-parameter -Wno-unused-function \
   -I"$out" -I"$src" -o "$out/hostcheck" "$root/tools/gsp-hostcheck/main.c"

echo "=== L6 — pasos 3 a 6 de la cadena FSP/COT + recepción de RPC ==="
"$out/hostcheck" "$ucode" "$boot" "$fmc"
