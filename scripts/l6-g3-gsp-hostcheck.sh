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
for m in gsp_dma gsp_rm gsp_wpr gsp_libos fmc_lx fsp_lx gsp_cpu_seq gsp_rpc gsp_cmdq gsp_rm_obj \
         gsp_vram gsp_vmm gsp_top gsp_chip gsp_pramin gsp_chan gsp_ce gsp_bar1 gsp_grctx gsp_buf gsp_compute gsp_fini; do
    guard="$(echo "$m" | tr '[:lower:]' '[:upper:]')_H"
    { strip_module "$src/$m.h" "$guard"; strip_module "$src/$m.c" "$guard"; } \
        > "$out/${m}_body.inc"
done

# Un fichero por arquitectura (R5): el banco comprueba los dos juegos y que
# no son intercambiables.
GSP_SRCS=(
    "$root/tools/gsp-hostcheck/main.c"
    "$src/sass_sm86.c" "$src/sass_sm120.c"
)
GSP_CFLAGS=(-O1 -Wall -Wextra -Wno-unused-parameter -Wno-unused-function
    -I"$out" -I"$src")

cc "${GSP_CFLAGS[@]}" -o "$out/hostcheck" "${GSP_SRCS[@]}"

# Segunda pasada con sanitizadores (R1/R10): el banco maneja punteros a
# firmware real y aritmética de páginas; ASan/UBSan cazan ahí lo que el
# resultado «OK» no ve. `alignment` fuera: las structs de protocolo son packed.
# SOSO_GSP_NO_SAN=1 lo salta (bootstrap sin ASan).
if [[ "${SOSO_GSP_NO_SAN:-0}" != 1 ]]; then
    cc "${GSP_CFLAGS[@]}" -g -fsanitize=address,undefined -fno-sanitize=alignment \
       -fno-omit-frame-pointer -fno-sanitize-recover=undefined \
       -o "$out/hostcheck-san" "${GSP_SRCS[@]}"
fi

echo "=== Ampere boot: FWSEC → booter (sin ACR previo) ==="
if awk '/^static int run_ampere_boot\(\)/,/^static int run_gsp_rm_chain|^static int run_fmc|^}/' \
    "$src/gsp_bringup.c" | grep -q 'run_acr_sec2'; then
    echo "FALLO: run_ampere_boot aún llama run_acr_sec2 antes del booter" >&2
    exit 1
fi
echo "OK: run_ampere_boot no invoca ACR antes del booter"

# R5.1: las tres medidas del CE tienen que seguir ahí y en orden. El bring-up
# no entra en el banco (necesita MMIO real), así que esto se comprueba sobre el
# fuente: una de las tres se perdió una vez y el diagnóstico se quedó ciego.
echo "=== R5.1: sondas de CE antes/después de GR0 y del promote ==="
orden=$(grep -n "run_ce_probe(CE_PROBE_\|gsp_compute_stage_sass_bringup(" "$src/gsp_bringup.c" |
    grep -oE "CE_PROBE_[A-Z0-9_]+|gsp_compute_stage_sass_bringup" | tr '\n' ' ')
esperado="CE_PROBE_ANTES_GR0 gsp_compute_stage_sass_bringup CE_PROBE_TRAS_GOLDEN CE_PROBE_TRAS_GR0 CE_PROBE_TRAS_PROMO "
if [[ "$orden" != "$esperado" ]]; then
    echo "FALLO: sondas de CE en «$orden»; se esperaba «$esperado»" >&2
    echo "       (la medida va antes de la precarga de SASS, que es la mitigación)" >&2
    exit 1
fi
if ! grep -q "gsp_chan_dump(&g_chan, \"sonda CE" "$src/gsp_bringup.c" ||
   ! grep -q "gsp_ce_drain_events(&g_ce)" "$src/gsp_bringup.c"; then
    echo "FALLO: el primer fallo de la sonda no conserva canal ni fault/RC" >&2
    exit 1
fi
echo "OK: tres medidas de CE en orden, con volcado de estado en el primer fallo"

echo "=== R5.1b: selftest CE fallido reintenta COPY0 ==="
if ! awk '/^static int run_chan_ce_stage\(/,/^static int gsp_gr_golden_oneinit/' \
        "$src/gsp_bringup.c" |
     grep -q 'reintentando COPY0'; then
    echo "FALLO: run_chan_ce_stage no reintenta COPY0 si el primer motor falla" >&2
    exit 1
fi
if ! awk '/^static int ce_bind_and_verify\(/,/^static int run_chan_ce_stage/' \
        "$src/gsp_bringup.c" | grep -q 'gsp_ce_selftest'; then
    echo "FALLO: el reintento COPY0 no pasa por gsp_ce_selftest" >&2
    exit 1
fi
echo "OK: CE selftest fallido (o init) reintenta COPY0"

echo "=== R5.2: PROMOTE_CTX antes de RM_ALLOC compute ==="
compute_ln=$(awk '/static int run_compute_stage/,/^}/ {
    if ($0 ~ /gsp_compute_init/) { print NR; exit }
}' "$src/gsp_bringup.c")
promo_ln=$(awk '/static int run_compute_stage/,/^}/ {
    if ($0 ~ /gsp_grctx_promote/) { print NR; exit }
}' "$src/gsp_bringup.c")
if [[ -z "$promo_ln" || -z "$compute_ln" || "$promo_ln" -ge "$compute_ln" ]]; then
    echo "FALLO: gsp_grctx_promote (L$promo_ln) debe ir antes de gsp_compute_init (L$compute_ln)" >&2
    exit 1
fi
echo "OK: promote L$promo_ln antes de compute_init L$compute_ln (r535_gr_chan_new)"

# R5.3: el encoder Ampere (QMDV01_07, 256 B) está transcrito en nvrm_r570.h.
# lxdde/reference/ es gitignored y no existe en CI; el check obligatorio va
# contra fuentes versionadas. Si el árbol de referencia está presente, se
# conserva el sentinel sobre cla0c0qmd.h por si aparecen cabeceras QMD nuevas.
echo "=== R5.3: encoder QMD Ampere en el árbol versionado ==="
nvrm="$src/nvrm_r570.h"
compute="$src/gsp_compute.c"
if ! grep -q 'GSP_QMD_VERSION_AMPERE' "$nvrm" ||
   ! grep -q 'QMDV02_PROGRAM_OFFSET' "$nvrm" ||
   ! grep -q 'GspQmdV02' "$nvrm"; then
    echo "FALLO: nvrm_r570.h no tiene el layout QMD v01_07 de Ampere (QMDV02_*)" >&2
    exit 1
fi
if ! grep -q 'GSP_FAM_AMPERE.*GSP_QMD_VERSION_AMPERE' "$compute"; then
    echo "FALLO: gsp_compute.c no declara qmd_version=GSP_QMD_VERSION_AMPERE para Ampere" >&2
    exit 1
fi
if [[ -d "$root/lxdde/reference" ]]; then
    qmd_hdrs=$(find "$root/lxdde/reference" -iname '*qmd*.h' 2>/dev/null | wc -l | tr -d ' ')
    cla0c0="$root/lxdde/reference/open-gpu-kernel-modules-570.144/src/common/sdk/nvidia/inc/class/cla0c0qmd.h"
    if [[ -f "$cla0c0" ]]; then
        qmd_vers=$(grep -ho 'QMDV[0-9][0-9]_[0-9][0-9]' "$cla0c0" |
            sort -u | tr '\n' ' ')
        if [[ "$qmd_hdrs" != 1 || "$qmd_vers" != "QMDV00_06 QMDV01_06 QMDV01_07 " ]]; then
            echo "FALLO: han cambiado las cabeceras de QMD en lxdde/reference ($qmd_hdrs fichero(s): $qmd_vers)." >&2
            echo "       Si ya está el QMD de Ampere, implementa su encoder y actualiza" >&2
            echo "       gsp_family_caps (qmd_version deja de ser 0)." >&2
            exit 1
        fi
        echo "OK: encoder Ampere en nvrm_r570.h; referencia cla0c0qmd.h ($qmd_vers)"
    else
        echo "OK: encoder Ampere en nvrm_r570.h; lxdde/reference sin cla0c0qmd.h"
    fi
else
    echo "OK: encoder Ampere en nvrm_r570.h (lxdde/reference ausente — CI)"
fi

echo "=== L6 — pasos 3 a 6 de la cadena FSP/COT + recepción de RPC ==="
SOSO_ROOT="$root" "$out/hostcheck" "$ucode" "$boot" "$fmc"

if [[ -x "$out/hostcheck-san" ]]; then
    echo "=== L6 — misma cadena con ASan+UBSan ==="
    if ! SOSO_ROOT="$root" ASAN_OPTIONS="detect_leaks=0" \
         UBSAN_OPTIONS="print_stacktrace=1" \
         "$out/hostcheck-san" "$ucode" "$boot" "$fmc" > "$out/hostcheck-san.log" 2>&1; then
        echo "FALLO sanitizadores en el banco GSP:" >&2
        cat "$out/hostcheck-san.log" >&2
        exit 1
    fi
    echo "OK: banco GSP sin hallazgos de ASan/UBSan"
fi
