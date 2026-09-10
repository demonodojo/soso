#define GFP_KERNEL 0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#include "iwl_internal.h"
#include "iwl_ax211.h"

void lx_printk(const char *fmt, ...) { (void)fmt; }

void lx_mdelay(unsigned int ms) { (void)ms; }

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

int cmd_wait_hostcheck(void);
int mvm_init_hostcheck(void);

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
    if (cmd_wait_hostcheck() != 0) {
        return 1;
    }
    if (mvm_init_hostcheck() != 0) {
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
    {
        const unsigned scan_nch = 21u;
        unsigned pay_v17 = iwl_scan_req_umac_v17_size(scan_nch);
        uint8_t scan_ver = 0;
        unsigned i;

        for (i = 0; i < iwl.cmd_ver_count; i++) {
            if (iwl.cmd_ver[i].group == LONG_GROUP &&
                iwl.cmd_ver[i].cmd == SCAN_REQ_UMAC) {
                scan_ver = iwl.cmd_ver[i].version;
                break;
            }
        }
        if (scan_ver < 14) {
            fprintf(stderr, "SCAN_REQ_UMAC ver=%u (esperaba >=14)\n", scan_ver);
            return 1;
        }
        if (pay_v17 > IWL_CMD_SLOT_SIZE) {
            fprintf(stderr, "SCAN_REQ_UMAC v17 (%u ch) no cabe (%u > %u)\n",
                    scan_nch, pay_v17, IWL_CMD_SLOT_SIZE);
            return 1;
        }
        if (iwl.phy_sku == 0 || iwl.n_scan_channels == 0) {
            fprintf(stderr, "PHY_SKU o N_SCAN ausentes en TLV\n");
            return 1;
        }
        {
            uint32_t doorbell = (1u & 0xffu) | ((uint32_t)IWL_MVM_DQA_CMD_QUEUE << 16);
            if (doorbell != 0x00000001u) {
                fprintf(stderr, "doorbell=0x%08x (esperaba 0x00000001)\n", doorbell);
                return 1;
            }
            uint16_t seq = (uint16_t)(QUEUE_TO_SEQ(IWL_MVM_DQA_CMD_QUEUE) |
                                      INDEX_TO_SEQ(1));
            if (seq != 0x0001) {
                fprintf(stderr, "seq=0x%04x (esperaba 0x0001)\n", seq);
                return 1;
            }
        }
        {
            uint8_t init_ver = 0;
            unsigned j;
            uint32_t init_flags = (uint32_t)(1u << IWL_INIT_NVM);

            for (j = 0; j < iwl.cmd_ver_count; j++) {
                if (iwl.cmd_ver[j].group == SYSTEM_GROUP &&
                    iwl.cmd_ver[j].cmd == INIT_EXTENDED_CFG_CMD) {
                    init_ver = iwl.cmd_ver[j].version;
                    break;
                }
            }
            if (init_flags != 2u) {
                fprintf(stderr, "IWL_INIT_NVM init_flags=0x%x (esperaba 2)\n",
                        init_flags);
                return 1;
            }
            printf("OK: INIT_EXTENDED_CFG init_flags=0x%x (BIT(IWL_INIT_NVM))%s\n",
                   init_flags,
                   init_ver ? "" : " (CMD_VERSION ausente → ver=0)");
            if (init_ver)
                printf("OK: INIT_EXTENDED_CFG TLV ver=%u\n", init_ver);
        }
        printf("OK: SCAN_REQ_UMAC v%u %u B, PHY_SKU=0x%08x n_scan=%u, doorbell ok\n",
               scan_ver, pay_v17, iwl.phy_sku, (unsigned)iwl.n_scan_channels);
    }
    if (sizeof(struct iwl_nvm_access_cmd) != 8) {
        fprintf(stderr, "iwl_nvm_access_cmd=%zu (esperaba 8)\n",
                sizeof(struct iwl_nvm_access_cmd));
        return 1;
    }
    if (sizeof(struct iwl_nvm_access_resp) != 8) {
        fprintf(stderr, "iwl_nvm_access_resp header=%zu (esperaba 8)\n",
                sizeof(struct iwl_nvm_access_resp));
        return 1;
    }
    puts("OK: NVM_ACCESS_CMD/resp API v2 (8 B cabecera)");
    {
        /* Init unificado: grp=12 solo NVM_ACCESS_COMPLETE (0) y NVM_GET_INFO (2). */
        if (NVM_ACCESS_CMD == NVM_GET_INFO || NVM_ACCESS_CMD == NVM_ACCESS_COMPLETE) {
            fprintf(stderr, "NVM_ACCESS_CMD=0x%02x colisiona con grp12\n",
                    NVM_ACCESS_CMD);
            return 1;
        }
        if (NVM_GET_INFO != 0x02u) {
            fprintf(stderr, "NVM_GET_INFO=0x%02x (esperaba 0x02)\n", NVM_GET_INFO);
            return 1;
        }
        puts("OK: init unificado usa NVM_ACCESS_COMPLETE/NVM_GET_INFO (grp12), no 0x88");
    }
    {
        unsigned pay17 = iwl_scan_req_umac_v17_size(21);
        if (pay17 <= sizeof(struct iwl_scan_probe_params_v4)) {
            fprintf(stderr, "scan v17 sin hueco probe (%u)\n", pay17);
            return 1;
        }
        puts("OK: scan v17 incluye probe_params");
    }
    if (sizeof(struct iwl_scan_config) != 12) {
        fprintf(stderr, "iwl_scan_config=%zu (esperaba 12 para SCAN_CFG v5)\n",
                sizeof(struct iwl_scan_config));
        return 1;
    }
    puts("OK: SCAN_CFG v5 payload iwl_scan_config 12 B");
    {
        const char *base = strrchr(argv[1], '/');
        int is_ax200 = 0;

        base = base ? base + 1 : argv[1];
        if (strstr(base, "cc-a0-77") != NULL)
            is_ax200 = 1;
        if (is_ax200) {
            int ver_tx;
            int ver_phy;

            if (iwl_fw_has_capa(&iwl, IWL_UCODE_TLV_CAPA_DQA_SUPPORT)) {
                fprintf(stderr, "cc-a0-77 no debe declarar CAPA_DQA_SUPPORT\n");
                return 1;
            }
            ver_tx = iwl_fw_cmd_ver(&iwl, LEGACY_GROUP, TX_ANT_CONFIGURATION_CMD);
            if (ver_tx != 1) {
                fprintf(stderr, "TX_ANT cmd_ver=%d (esperaba 1 vía DEF_ID grp=1)\n",
                        ver_tx);
                return 1;
            }
            ver_phy = iwl_fw_cmd_ver(&iwl, LEGACY_GROUP, PHY_CONTEXT_CMD);
            if (ver_phy != 4) {
                fprintf(stderr, "PHY_CONTEXT cmd_ver=%d (esperaba 4 vía DEF_ID grp=1)\n",
                        ver_phy);
                return 1;
            }
            printf("OK: DEF_ID cmd_ver TX_ANT=%d PHY_CONTEXT=%d, sin CAPA_DQA\n",
                   ver_tx, ver_phy);
        }
    }
    return 0;
}
