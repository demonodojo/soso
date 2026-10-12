/* Reservas coherentes para las estructuras que lee el GSP. Ver gsp_dma.h. */
#include "gsp_dma.h"

/* Definidos en el shim (lxdde/shim/src/shims.c). */
void *memcpy(void *dst, const void *src, unsigned long n);
void *memset(void *dst, int c, unsigned long n);

static int dma_alloc_common(struct gsp_dma_buf *b, unsigned long size,
                            const char *what, int cached)
{
    uint64_t phys = 0;
    void *va;

    if (!b || size == 0) {
        return -1;
    }
    va = cached ? lx_dma_alloc_wb(NULL, size, &phys, GFP_KERNEL)
                : lx_dma_alloc_coherent(NULL, size, &phys, GFP_KERNEL);
    if (!va || !phys) {
        if (va) {
            lx_dma_free_coherent(NULL, size, va, phys);
        }
        lx_printk("nouveau-lx: sin memoria coherente para %s (%lu bytes)\n",
                  what ? what : "?", size);
        return -1;
    }
    memset(va, 0, size);
    b->va = va;
    b->phys = phys;
    b->size = size;
    return 0;
}

int gsp_dma_alloc(struct gsp_dma_buf *b, unsigned long size, const char *what)
{
    return dma_alloc_common(b, size, what, 0);
}

int gsp_dma_alloc_wb(struct gsp_dma_buf *b, unsigned long size, const char *what)
{
    return dma_alloc_common(b, size, what, 1);
}

int gsp_dma_alloc_copy(struct gsp_dma_buf *b, const void *src, unsigned long size,
                       const char *what)
{
    if (!src || gsp_dma_alloc(b, size, what) != 0) {
        return -1;
    }
    memcpy(b->va, src, size);
    return 0;
}

void gsp_dma_free(struct gsp_dma_buf *b)
{
    if (!b) {
        return;
    }
    if (b->va) {
        lx_dma_free_coherent(NULL, b->size, b->va, b->phys);
    }
    b->va = NULL;
    b->phys = 0;
    b->size = 0;
}
