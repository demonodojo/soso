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
    void *p = aligned_alloc(4096, (size + 4095u) & ~(size_t)4095u);
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

/* Comparación independiente con los TLV originales: no basta contar mapas.
 * La dirección SRAM que encabeza SEC_RT nunca debe llegar al buffer DMA. */
static int verify_payloads(const uint8_t *fw, size_t size,
                           const struct iwl_context_info_dram *dram)
{
    const uint64_t *maps[] = { dram->lmac_img, dram->umac_img, dram->virtual_img };
    size_t pos = sizeof(struct iwl_tlv_ucode_header);
    unsigned phase = 0, counts[3] = {0, 0, 0};
    while (size - pos >= 8) {
        uint32_t type = le32(fw + pos), len = le32(fw + pos + 4);
        pos += 8;
        if (len > size - pos)
            return -1;
        if (type == IWL_UCODE_TLV_SEC_RT) {
            if (len < 4)
                return -1;
            uint32_t offset = le32(fw + pos);
            if (offset == IWL_CPU1_CPU2_SEPARATOR) {
                phase = 1;
            } else if (offset == IWL_PAGING_SEPARATOR) {
                phase = 2;
            } else {
                if (counts[phase] >= IWL_MAX_DRAM_ENTRY)
                    return -1;
                const void *dma = (const void *)(uintptr_t)maps[phase][counts[phase]++];
                if (!dma || memcmp(dma, fw + pos + 4, len - 4) != 0) {
                    fprintf(stderr, "payload DMA distinto en fase %u sección %u\n",
                            phase, counts[phase] - 1);
                    return -1;
                }
            }
        }
        size_t padded = ((size_t)len + 3u) & ~(size_t)3u;
        if (padded > size - pos)
            return -1;
        pos += padded;
    }
    for (unsigned phase = 0; phase < 3; phase++)
        if (counts[phase] != (unsigned)count_nonzero((uint64_t *)maps[phase], IWL_MAX_DRAM_ENTRY))
            return -1;
    return pos == size ? 0 : -1;
}

static int synthetic_sections(void)
{
    uint8_t fw[sizeof(struct iwl_tlv_ucode_header) + 5 * 16] = {0};
    const uint32_t magic = IWL_TLV_UCODE_MAGIC;
    memcpy(fw + 4, &magic, 4);
    /* Datos de cuatro bytes: TLV de tamaño 8 que no es un separador.
     * INIT/WOWLAN intercalados no deben mezclarse con la imagen regular. */
    const uint32_t types[] = {19, 20, 19, 21, 19};
    const uint32_t offsets[] = {0x800000, 0x900000, IWL_CPU1_CPU2_SEPARATOR, 0x900004, 0xa00000};
    for (unsigned i = 0; i < 5; i++) {
        uint32_t words[] = {types[i], 8, offsets[i], 0x87654320u + i};
        memcpy(fw + sizeof(struct iwl_tlv_ucode_header) + i * 16, words, 16);
    }
    struct iwl_ax211_priv iwl = {0};
    struct iwl_context_info_dram dram = {0};
    if (iwl_fw_parse_tlv(&iwl, fw, sizeof(fw)) || iwl.fw.rt_n != 3 ||
        iwl_fw_upload_sections(&iwl, &dram) || verify_payloads(fw, sizeof(fw), &dram))
        return -1;
    if (iwl_fw_parse_tlv(&iwl, fw, sizeof(fw) - 1) == 0)
        return -1;
    return 0;
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
    if (synthetic_sections() != 0) {
        fprintf(stderr, "regresión SEC_RT/payload/truncado\n");
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
    if (verify_payloads(fw, (size_t)sz, &dram) != 0)
        return 1;
    puts("OK: payloads DMA idénticos a SEC_RT sin prefijo SRAM");
    if (sizeof(struct iwl_context_info) < 256u) {
        fprintf(stderr, "iwl_context_info demasiado pequeño (%zu)\n",
                sizeof(struct iwl_context_info));
        return 1;
    }
    return 0;
}
