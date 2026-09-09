/* Acceso MMIO real del port ath11k.
 *
 * Vive en su propio fichero porque es la costura que el hostcheck sustituye
 * por un modelo del dispositivo (`tools/ath11k-hostcheck`).
 *
 * Los registros MHI están en la parte baja del BAR0 y se leen directos. Linux
 * sólo necesita la ventana deslizante de `ath11k_pcic_read32` para offsets por
 * encima de `ATH11K_PCI_WINDOW_START` (0x80000), que son los del SoC; el
 * transporte MHI se queda muy por debajo.
 */
#include "lx_emul.h"
#include "ath11k_internal.h"

uint32_t ath11k_read32(struct ath11k_base *ab, uint32_t off)
{
    if (!ab->mmio)
        return 0xffffffffu;
    if (ab->mmio_len && off + 4 > ab->mmio_len)
        return 0xffffffffu;
    return ab->mmio[off / 4];
}

void ath11k_write32(struct ath11k_base *ab, uint32_t off, uint32_t val)
{
    if (!ab->mmio)
        return;
    if (ab->mmio_len && off + 4 > ab->mmio_len)
        return;
    ab->mmio[off / 4] = val;
}
