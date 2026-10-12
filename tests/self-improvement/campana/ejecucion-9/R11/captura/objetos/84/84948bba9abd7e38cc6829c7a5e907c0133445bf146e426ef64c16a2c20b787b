#!/usr/bin/env bash
# Host check: parser VBIOS → FWSEC (PROM 0x300000, BIT 0x70, PMU 0x85) sin GPU.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$root/lxdde/ports/nouveau"
out="$root/target/fwsec-hostcheck"
rom="${1:-}"

mkdir -p "$out"

grep -v '^#include' "$src/gsp_fwsec.c" |
    grep -v '^void \*memcpy' | grep -v '^void \*memset' \
    > "$out/gsp_fwsec_body.inc"

cc -O1 -Wall -Wextra -Wno-unused-parameter -Wno-unused-function \
   -I"$out" -I"$src" \
   -o "$out/hostcheck" \
   "$root/tools/fwsec-hostcheck/main.c"

if [[ -z "$rom" ]]; then
    bdf="${SOSO_VBIOS_BDF:-0000:01:00.0}"
    rom_sys="/sys/bus/pci/devices/$bdf/rom"
    rom="$out/vbios.rom"
    if [[ -r "$rom_sys" ]]; then
        echo "volcando VBIOS desde $rom_sys"
        if [[ -w "$rom_sys" ]]; then
            echo 1 | sudo tee "$rom_sys" >/dev/null 2>&1 || true
        fi
        sudo dd if="$rom_sys" of="$rom" bs=1M count=1 status=none 2>/dev/null ||
            cp "$rom_sys" "$rom" 2>/dev/null || {
            echo "pasa un .rom: $0 /ruta/vbios.rom" >&2
            exit 1
        }
    else
        echo "pasa un .rom: $0 /ruta/vbios.rom" >&2
        exit 1
    fi
fi

if [[ ! -f "$rom" ]]; then
    echo "no existe $rom" >&2
    exit 1
fi

echo "=== fwsec hostcheck: $(basename "$rom") ==="
"$out/hostcheck" "$rom"
