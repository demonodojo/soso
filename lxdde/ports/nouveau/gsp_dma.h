/* Bloque de memoria coherente con su dirección física — el equivalente de
 * `nvkm_gsp_mem` de nvkm. Lo comparten los pasos 4, 5 y 6 de la cadena FSP/COT. */
#ifndef GSP_DMA_H
#define GSP_DMA_H

#include "lx_emul.h"

struct gsp_dma_buf {
    void *va;
    uint64_t phys;
    unsigned long size;
};

/* Redondear hacia arriba a una potencia de dos. Vivía privado en `gsp_wpr.c` y
 * `gsp_grctx.c` lo volvió a escribir como máscara cruda en dos sitios; está aquí
 * porque es el header que ya comparten los módulos que reparten memoria. */
static inline uint64_t align_up_u64(uint64_t v, uint64_t a)
{
    return (v + a - 1ull) & ~(a - 1ull);
}

/* Reserva y pone a cero. Publica va/phys/size a la vez o deja el bloque intacto:
 * un `va` con `size` a cero haría que el release liberase otro tamaño. */
int gsp_dma_alloc(struct gsp_dma_buf *b, unsigned long size, const char *what);

/* Reserva y copia `src`. */
int gsp_dma_alloc_copy(struct gsp_dma_buf *b, const void *src, unsigned long size,
                       const char *what);

void gsp_dma_free(struct gsp_dma_buf *b);

#endif
