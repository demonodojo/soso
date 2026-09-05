#define GFP_KERNEL 0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#include "iwl_internal.h"
#include "iwl_ax211.h"

void lx_printk(const char *fmt, ...) { (void)fmt; }

void *lx_kmalloc(unsigned long size, unsigned gfp) {
    (void)gfp;
    return malloc(size);
}

void lx_kfree(const void *p) { free((void *)p); }

void *lx_dma_alloc_coherent(void *dev, size_t size, uint64_t *dma, unsigned gfp) {
    (void)dev;
    (void)gfp;
    void *p = aligned_alloc(4096, size);
    if (p && dma) {
        *dma = (uint64_t)(uintptr_t)p;
    }
    return p;
}

void lx_dma_free_coherent(void *dev, size_t size, void *cpu, uint64_t dma) {
    (void)dev;
    (void)size;
    (void)dma;
    free(cpu);
}

#include "iwl_fw_body.inc"

static int count_nonzero(uint64_t *map, int max) {
    int n = 0;
    for (int i = 0; i < max; i++) {
        if (map[i]) {
            n++;
        }
    }
    return n;
}

int main(int argc, char **argv)
{
    struct iwl_ax211_priv iwl;
    FILE *f;
    long sz;
    uint8_t *fw;
    int lmac, umac, paging;
    struct iwl_context_info_dram dram;

    if (argc != 2) {
        fprintf(stderr, "uso: %s firmware.ucode\n", argv[0]);
        return 1;
    }
    f = fopen(argv[1], "rb");
    if (!f) {
        perror(argv[1]);
        return 1;
    }
    fseek(f, 0, SEEK_END);
    sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (sz <= 0) {
        fprintf(stderr, "firmware vacío\n");
        return 1;
    }
    fw = malloc((size_t)sz);
    if (!fw || fread(fw, 1, (size_t)sz, f) != (size_t)sz) {
        fprintf(stderr, "lectura falló\n");
        return 1;
    }
    fclose(f);

    memset(&iwl, 0, sizeof(iwl));
    if (iwl_fw_parse_tlv(&iwl, fw, (unsigned long)sz) != 0) {
        fprintf(stderr, "parse TLV falló\n");
        return 1;
    }
    memset(&dram, 0, sizeof(dram));
    if (iwl_fw_upload_sections(&iwl, &dram) != 0) {
        fprintf(stderr, "upload sections falló\n");
        return 1;
    }
    lmac = count_nonzero(dram.lmac_img, IWL_MAX_DRAM_ENTRY);
    umac = count_nonzero(dram.umac_img, IWL_MAX_DRAM_ENTRY);
    paging = count_nonzero(dram.virtual_img, IWL_MAX_DRAM_ENTRY);
    printf("lmac=%d umac=%d paging=%d\n", lmac, umac, paging);
    if (!lmac || !umac) {
        return 1;
    }
    return 0;
}
