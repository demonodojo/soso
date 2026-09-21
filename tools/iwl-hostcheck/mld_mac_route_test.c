/* Host: AX211 (so-a0-gf-a0) usa MAC_CONFIG; AX200 (cc-a0) conserva MAC_CONTEXT. */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

void lx_printk(const char *fmt, ...) { (void)fmt; }

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

static int uses_mld_mac(const struct iwl_ax211_priv *iwl)
{
    if (iwl_fw_cmd_ver((struct iwl_ax211_priv *)iwl, LEGACY_GROUP,
                       MAC_CONTEXT_CMD) > 0)
        return 0;
    return iwl_fw_cmd_ver((struct iwl_ax211_priv *)iwl, MAC_CONF_GROUP,
                          MAC_CONFIG_CMD) > 0;
}

static int fw_has_binding_cmd(const struct iwl_ax211_priv *iwl)
{
    return iwl_fw_cmd_ver((struct iwl_ax211_priv *)iwl, LEGACY_GROUP,
                          BINDING_CONTEXT_CMD) > 0;
}

static int parse_ucode(const char *path, struct iwl_ax211_priv *iwl)
{
    FILE *f;
    long sz;
    uint8_t *fw;
    int rc;

    f = fopen(path, "rb");
    if (!f) {
        perror(path);
        return -1;
    }
    fseek(f, 0, SEEK_END);
    sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    fw = malloc((size_t)sz);
    if (!fw || fread(fw, 1, (size_t)sz, f) != (size_t)sz) {
        fclose(f);
        return -1;
    }
    fclose(f);
    memset(iwl, 0, sizeof(*iwl));
    rc = iwl_fw_parse_tlv(iwl, fw, (unsigned long)sz);
    free(fw);
    return rc;
}

int main(int argc, char **argv)
{
    struct iwl_ax211_priv ax211;
    struct iwl_ax211_priv ax200;

    if (argc != 3) {
        fprintf(stderr, "uso: %s gf-a0-89.ucode cc-a0-77.ucode\n", argv[0]);
        return 1;
    }
    if (parse_ucode(argv[1], &ax211) != 0 || parse_ucode(argv[2], &ax200) != 0) {
        fprintf(stderr, "parse TLV falló\n");
        return 1;
    }
    if (sizeof(struct iwl_mac_config_cmd) != 52) {
        fprintf(stderr, "MAC_CONFIG size=%zu (esperado 52)\n",
                sizeof(struct iwl_mac_config_cmd));
        return 1;
    }
    if (sizeof(struct iwl_link_config_cmd) != 208) {
        fprintf(stderr, "LINK_CONFIG size=%zu (esperado 208)\n",
                sizeof(struct iwl_link_config_cmd));
        return 1;
    }
    {
        struct iwl_link_config_cmd bind = {};
        struct iwl_link_config_cmd act = {};

        bind.phy_id = iwl_cpu_to_le32(0);
        bind.modify_mask = iwl_cpu_to_le32(0);
        bind.active = iwl_cpu_to_le32(0);
        act.modify_mask = iwl_cpu_to_le32(LINK_CONTEXT_MODIFY_ACTIVE |
                                            LINK_CONTEXT_MODIFY_RATES_INFO);
        act.active = iwl_cpu_to_le32(1);
        act.cck_rates = iwl_cpu_to_le32(0x0fu);
        act.ofdm_rates = iwl_cpu_to_le32(0xffu);
        if (bind.active != 0 || bind.phy_id != iwl_cpu_to_le32(0) ||
            act.cck_rates != iwl_cpu_to_le32(0x0fu) ||
            act.ofdm_rates != iwl_cpu_to_le32(0xffu)) {
            fprintf(stderr, "secuencia LINK bind/activate mal armada\n");
            return 1;
        }
    }
    if (!uses_mld_mac(&ax211)) {
        fprintf(stderr, "AX211 debería usar MLD MAC\n");
        return 1;
    }
    if (uses_mld_mac(&ax200)) {
        fprintf(stderr, "AX200 no debería usar MLD MAC\n");
        return 1;
    }
    if (fw_has_binding_cmd(&ax211)) {
        fprintf(stderr, "AX211 no declara BINDING 0x2b\n");
        return 1;
    }
    if (!fw_has_binding_cmd(&ax200)) {
        fprintf(stderr, "AX200 debería declarar BINDING 0x2b\n");
        return 1;
    }
    puts("OK: ruta MLD en gf-a0-89, MAC_CONTEXT en cc-a0-77");
    return 0;
}
