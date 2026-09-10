/* Host: sin CAPA_DQA el up mínimo no encola DATA_PATH/DQA_ENABLE (grp=5 id=0). */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

static int g_dqa_sent;

void lx_printk(const char *fmt, ...) { (void)fmt; }
void lx_mdelay(unsigned int ms) { (void)ms; }

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

int iwl_trans_send_cmd_async(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                             const void *payload, uint16_t pay_len)
{
    (void)iwl;
    (void)group;
    (void)id;
    (void)payload;
    (void)pay_len;
    return 0;
}

int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t pay_len, int wait_ms)
{
    (void)iwl;
    (void)payload;
    (void)pay_len;
    (void)wait_ms;
    if (group == DATA_PATH_GROUP && id == DQA_ENABLE_CMD)
        g_dqa_sent = 1;
    return 0;
}

int iwl_mvm_send_scan_cfg(struct iwl_ax211_priv *iwl)
{
    iwl->scan_cfg_sent = 1;
    return 0;
}

#include "iwl_fw_body.inc"

static int run_up_without_dqa(const struct iwl_ax211_priv *parsed)
{
    struct iwl_ax211_priv iwl;

    if (iwl_fw_has_capa(parsed, IWL_UCODE_TLV_CAPA_DQA_SUPPORT)) {
        printf("OK: ucode declara CAPA_DQA — test omitido\n");
        return 0;
    }

    memset(&iwl, 0, sizeof(iwl));
    memcpy(iwl.fw_capa, parsed->fw_capa, sizeof(iwl.fw_capa));
    iwl.radio_ready = 1;
    iwl.phy_sku = parsed->phy_sku;
    iwl.fw_valid_tx_ant = parsed->fw_valid_tx_ant ? parsed->fw_valid_tx_ant : 3;
    iwl.fw_valid_rx_ant = parsed->fw_valid_rx_ant ? parsed->fw_valid_rx_ant : 3;
    iwl.lar_enabled = 0;
    iwl.cmd_ver_count = parsed->cmd_ver_count;
    memcpy(iwl.cmd_ver, parsed->cmd_ver, sizeof(iwl.cmd_ver));

    g_dqa_sent = 0;
    if (iwl_mvm_up_minimal(&iwl) != 0) {
        fprintf(stderr, "iwl_mvm_up_minimal falló en hostcheck DQA\n");
        return -1;
    }
    if (g_dqa_sent) {
        fprintf(stderr, "DQA_ENABLE encolado sin CAPA_DQA_SUPPORT\n");
        return -1;
    }
    if (!iwl.mvm_up_done) {
        fprintf(stderr, "mvm_up_done=0 tras up_minimal\n");
        return -1;
    }
    puts("OK: up_minimal sin DQA_ENABLE (sin CAPA_DQA_SUPPORT)");
    return 0;
}

int main(int argc, char **argv)
{
    struct iwl_ax211_priv iwl;
    FILE *f;
    long sz;
    uint8_t *fw;

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
    free(fw);
    return run_up_without_dqa(&iwl) != 0 ? 1 : 0;
}
