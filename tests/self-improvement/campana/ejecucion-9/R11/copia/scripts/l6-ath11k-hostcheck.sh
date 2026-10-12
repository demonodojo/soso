#!/usr/bin/env bash
# Host check: transporte MHI de ath11k (WCN6855) contra un modelo del dispositivo.
#
# Compila el port con `ath11k_read32`/`ath11k_write32` sustituidos por una
# máquina de estados que imita el arranque del chip (PBL → BHIe → AMSS → M0).
# Cubre la lógica del bring-up y sus caminos de error; los offsets y los tiempos
# del silicio real sólo los confirma la placa.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/target/ath11k-hostcheck"
src="$root/lxdde/ports/ath11k"

mkdir -p "$out"

# El `lx_emul.h` del shim arrastra un <stdint.h> freestanding que choca con el
# de la libc del host. Para el hostcheck basta con declarar lo que el port usa.
cat > "$out/lx_emul.h" <<'HDR'
/* Recorte de lx_emul.h para el hostcheck (generado por l6-ath11k-hostcheck.sh). */
#ifndef LX_EMUL_H
#define LX_EMUL_H
#include <stddef.h>
#include <stdint.h>
#define GFP_KERNEL 0x40u
#define GFP_ATOMIC 0x20u
struct lx_pci_dev;
int lx_printk(const char *fmt, ...);
void lx_udelay(unsigned long us);
void lx_mdelay(unsigned int ms);
uint64_t lx_ktime_get_ns(void);
void *lx_dma_alloc_coherent(struct lx_pci_dev *dev, size_t size, uint64_t *dma_handle, unsigned gfp);
void *lx_dma_alloc_wb(struct lx_pci_dev *dev, size_t size, uint64_t *dma_handle, unsigned gfp);
void lx_dma_flush_range(const void *ptr, size_t len);
void lx_dma_free_coherent(struct lx_pci_dev *dev, size_t size, void *cpu_addr, uint64_t dma_handle);
#endif
HDR

SRCS=("$root/tools/ath11k-hostcheck/main.c" "$src/ath11k_mhi.c" "$src/ath11k_qmi.c")
CFLAGS=(-O1 -Wall -Wextra -Wno-unused-parameter -Wno-unused-function -I"$out" -I"$src")

cc "${CFLAGS[@]}" -o "$out/hostcheck" "${SRCS[@]}"
"$out/hostcheck" "$@"

# Igual que los bancos de iwlwifi y GSP: una pasada con sanitizadores.
if [[ "${SOSO_ATH11K_NO_SAN:-0}" != 1 ]]; then
    cc "${CFLAGS[@]}" -g -fsanitize=address,undefined -fno-sanitize=alignment \
       -fno-omit-frame-pointer -fno-sanitize-recover=undefined \
       -o "$out/hostcheck-san" "${SRCS[@]}"
    if ! ASAN_OPTIONS="detect_leaks=0" UBSAN_OPTIONS="print_stacktrace=1" \
         "$out/hostcheck-san" "$@" > "$out/hostcheck-san.log" 2>&1; then
        echo "FALLO sanitizadores en el banco ath11k:" >&2
        cat "$out/hostcheck-san.log" >&2
        exit 1
    fi
    echo "OK: banco ath11k sin hallazgos de ASan/UBSan"
fi
