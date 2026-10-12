/* Host: PNVM por SKU del ALIVE y descriptor fragmentado (capa 32). */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

void lx_printk(const char *fmt, ...) { (void)fmt; }
void lx_mdelay(unsigned int ms) { (void)ms; }
void lx_udelay(unsigned int us) { (void)us; }
void lx_iwlwifi_set_alive(int alive) { (void)alive; }

void iwl_ax211_deliver_rx(const uint8_t *data, int len)
{
    (void)data;
    (void)len;
}

void iwl_ax211_deliver_eapol(const uint8_t *data, int len)
{
    (void)data;
    (void)len;
}

void iwl_ax211_add_bss(const struct iwl_ax211_bss *bss) { (void)bss; }

void iwl_mvm_rx_scan_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
{
    (void)iwl;
    (void)frame;
    (void)len;
}

void iwl_mvm_rx_mlme_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
{
    (void)iwl;
    (void)frame;
    (void)len;
}

int iwl_mvm_rx_to_eth(const struct iwl_ax211_priv *iwl, const uint8_t *frame, int flen,
                      uint32_t rx_status, uint8_t *out, int out_cap)
{
    (void)iwl;
    (void)frame;
    (void)flen;
    (void)rx_status;
    (void)out;
    (void)out_cap;
    return -1;
}

void iwl_mvm_on_scan_complete(struct iwl_ax211_priv *iwl, uint32_t uid, uint8_t status)
{
    (void)iwl;
    (void)uid;
    (void)status;
}

void *lx_kmalloc(unsigned long size, unsigned gfp)
{
    (void)gfp;
    return malloc(size);
}

void lx_kfree(const void *p) { free((void *)p); }

void *lx_dma_alloc_coherent(void *dev, size_t size, uint64_t *dma, unsigned gfp)
{
    (void)dev;
    (void)gfp;
    void *p = aligned_alloc(4096, (size + 4095u) & ~(size_t)4095u);
    if (p && dma)
        *dma = (uint64_t)(uintptr_t)p;
    return p;
}

void lx_dma_free_coherent(void *dev, size_t size, void *cpu, uint64_t dma)
{
    (void)dev;
    (void)size;
    (void)dma;
    free(cpu);
}

#include "iwl_fw_body.inc"

static uint32_t chunk_sum(const struct iwl_pnvm_image *img)
{
    unsigned i;
    uint32_t total = 0;

    for (i = 0; i < img->n_chunks; i++)
        total += img->chunks[i].len;
    return total;
}

static int load_file(const char *path, uint8_t **out, unsigned long *len)
{
    FILE *f = fopen(path, "rb");
    long sz;

    if (!f)
        return -1;
    if (fseek(f, 0, SEEK_END) != 0) {
        fclose(f);
        return -1;
    }
    sz = ftell(f);
    if (sz <= 0) {
        fclose(f);
        return -1;
    }
    if (fseek(f, 0, SEEK_SET) != 0) {
        fclose(f);
        return -1;
    }
    *out = malloc((size_t)sz);
    if (!*out) {
        fclose(f);
        return -1;
    }
    if (fread(*out, 1, (size_t)sz, f) != (size_t)sz) {
        free(*out);
        fclose(f);
        return -1;
    }
    fclose(f);
    *len = (unsigned long)sz;
    return 0;
}

static void ax211_hw_ids(struct iwl_ax211_priv *iwl)
{
    /* mac_type 0x37 / rf_id 0x10d en iwlwifi-so-a0-gf-a0.pnvm (sección 0x510d1). */
    iwl->hw_rev = 0x370u;
    iwl->hw_rf_id = 0x10d000u;
}

static int test_sku_select(const char *pnvm_path)
{
    struct iwl_ax211_priv iwl;
    struct iwl_pnvm_image img;
    uint8_t *pnvm = NULL;
    unsigned long pnvm_len = 0;

    if (load_file(pnvm_path, &pnvm, &pnvm_len) != 0) {
        fprintf(stderr, "lectura PNVM falló: %s\n", pnvm_path);
        return -1;
    }

    memset(&iwl, 0, sizeof(iwl));
    ax211_hw_ids(&iwl);
    if (iwl_fw_parse_pnvm(&iwl, pnvm, pnvm_len) != 0) {
        fprintf(stderr, "parse pnvm file falló\n");
        free(pnvm);
        return -1;
    }

    iwl.sku_id[0] = 0x610d1u;
    iwl.sku_id[1] = 0;
    iwl.sku_id[2] = 0;
    if (iwl_fw_pnvm_select(&iwl, &img) != 0 || img.n_chunks != 2 ||
        chunk_sum(&img) != 13668u) {
        fprintf(stderr, "SKU 0x610d1: esperaba 2 chunks / 13668 B, hay %u / %u\n",
                (unsigned)img.n_chunks, (unsigned)chunk_sum(&img));
        free(pnvm);
        return -1;
    }
    printf("OK: SKU 0x610d1 → %u bytes (%u chunks)\n",
           (unsigned)chunk_sum(&img), (unsigned)img.n_chunks);

    iwl.sku_id[0] = 0x510d1u;
    if (iwl_fw_pnvm_select(&iwl, &img) != 0 || img.n_chunks != 2 ||
        chunk_sum(&img) != 13716u) {
        fprintf(stderr, "SKU 0x510d1: esperaba 2 chunks / 13716 B, hay %u / %u\n",
                (unsigned)img.n_chunks, (unsigned)chunk_sum(&img));
        free(pnvm);
        return -1;
    }
    printf("OK: SKU 0x510d1 → %u bytes (%u chunks)\n",
           (unsigned)chunk_sum(&img), (unsigned)img.n_chunks);

    free(pnvm);
    return 0;
}

static int test_publish_fragmented(const char *pnvm_path, const char *ucode_path)
{
    struct iwl_ax211_priv iwl;
    struct iwl_prph_scratch *scratch;
    uint8_t *pnvm = NULL;
    uint8_t *ucode = NULL;
    unsigned long pnvm_len = 0;
    unsigned long ucode_len = 0;
    const struct iwl_prph_scrath_mem_desc_addr_array *desc;
    uint32_t total;

    if (load_file(pnvm_path, &pnvm, &pnvm_len) != 0 ||
        load_file(ucode_path, &ucode, &ucode_len) != 0) {
        fprintf(stderr, "lectura firmware falló\n");
        free(pnvm);
        free(ucode);
        return -1;
    }

    memset(&iwl, 0, sizeof(iwl));
    ax211_hw_ids(&iwl);
    iwl.gen3 = 1;
    iwl.sku_id[0] = 0x510d1u;
    if (iwl_fw_parse_tlv(&iwl, ucode, ucode_len) != 0) {
        fprintf(stderr, "parse ucode falló\n");
        free(pnvm);
        free(ucode);
        return -1;
    }
    if (!iwl_fw_has_capa(&iwl, IWL_UCODE_TLV_CAPA_FRAGMENTED_PNVM_IMG)) {
        fprintf(stderr, "ucode -89 debe declarar capa 32 (fragmented PNVM)\n");
        free(pnvm);
        free(ucode);
        return -1;
    }
    if (iwl_fw_parse_pnvm(&iwl, pnvm, pnvm_len) != 0) {
        fprintf(stderr, "parse pnvm falló\n");
        free(pnvm);
        free(ucode);
        return -1;
    }

    scratch = calloc(1, sizeof(*scratch));
    if (!scratch) {
        free(pnvm);
        free(ucode);
        return -1;
    }
    iwl.scratch_cpu = scratch;

    if (scratch->ctrl_cfg.pnvm_cfg.pnvm_size != 0) {
        fprintf(stderr, "pnvm_cfg debe empezar vacío (gen3 boot)\n");
        free(scratch);
        free(pnvm);
        free(ucode);
        return -1;
    }
    if (iwl_trans_pnvm_publish(&iwl) != 0) {
        fprintf(stderr, "iwl_trans_pnvm_publish falló\n");
        free(scratch);
        free(pnvm);
        free(ucode);
        return -1;
    }

    total = scratch->ctrl_cfg.pnvm_cfg.pnvm_size;
    if (total != 13716u) {
        fprintf(stderr, "pnvm_size publicado %u, esperaba 13716\n", (unsigned)total);
        free(scratch);
        free(pnvm);
        free(ucode);
        return -1;
    }

    desc = (const struct iwl_prph_scrath_mem_desc_addr_array *)(uintptr_t)
        scratch->ctrl_cfg.pnvm_cfg.pnvm_base_addr;
    if (!desc || !desc->mem_descs[0] || !desc->mem_descs[1] || desc->mem_descs[2]) {
        fprintf(stderr, "descriptor fragmentado inválido (base=0x%llx d0=0x%llx d1=0x%llx)\n",
                (unsigned long long)scratch->ctrl_cfg.pnvm_cfg.pnvm_base_addr,
                (unsigned long long)desc ? desc->mem_descs[0] : 0,
                (unsigned long long)desc ? desc->mem_descs[1] : 0);
        free(scratch);
        free(pnvm);
        free(ucode);
        return -1;
    }
    if (iwl.pnvm_n_chunks != 2 || iwl.pnvm_chunks[0].len != 1656u ||
        iwl.pnvm_chunks[1].len != 12060u) {
        fprintf(stderr, "chunks DMA %u/%u/%u\n",
                (unsigned)iwl.pnvm_n_chunks,
                (unsigned)iwl.pnvm_chunks[0].len,
                (unsigned)iwl.pnvm_chunks[1].len);
        free(scratch);
        free(pnvm);
        free(ucode);
        return -1;
    }
    if (desc->mem_descs[0] != iwl.pnvm_chunks[0].dma ||
        desc->mem_descs[1] != iwl.pnvm_chunks[1].dma) {
        fprintf(stderr, "mem_descs no apuntan a los chunks DMA\n");
        free(scratch);
        free(pnvm);
        free(ucode);
        return -1;
    }

    printf("OK: publish fragmentado capa 32 — descriptor + 2 SEC_RT (%u B)\n",
           (unsigned)total);
    free(scratch);
    free(pnvm);
    free(ucode);
    return 0;
}

int main(int argc, char **argv)
{
    const char *pnvm_path;
    const char *ucode_path;

    if (argc != 3) {
        fprintf(stderr, "uso: %s pnvm ucode\n", argv[0]);
        return 1;
    }
    pnvm_path = argv[1];
    ucode_path = argv[2];
    if (test_sku_select(pnvm_path) != 0)
        return 1;
    if (test_publish_fragmented(pnvm_path, ucode_path) != 0)
        return 1;
    return 0;
}
