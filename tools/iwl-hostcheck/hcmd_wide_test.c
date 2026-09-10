/* Host: HCMD gen2 usa cabecera wide (8 B); LEGACY_GROUP API → wire LONG_GROUP (DEF_ID). */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include <stdlib.h>

#define REPLY_SF_CFG_CMD 0xd1

struct iwl_sf_cfg_cmd {
    uint32_t state;
    uint32_t watermark[2];
    uint32_t long_delay_timeouts[5][2];
    uint32_t full_on_timeouts[5][2];
} __attribute__((packed));

struct iwl_phy_context_cmd_v1 {
    uint32_t id_and_color;
    uint32_t action;
    uint32_t apply_time;
    uint32_t tx_param_color;
    struct {
        uint8_t band;
        uint8_t channel;
        uint8_t width;
        uint8_t ctrl_pos;
    } ci;
    uint32_t txchain_info;
    uint32_t rxchain_info;
    uint32_t acquisition_data;
    uint32_t dsp_cfg_flags;
} __attribute__((packed));

static uint8_t g_mcr_pool[IWL_CMD_SLOT_SIZE * IWL_CMD_QUEUE_SIZE];
static uint8_t g_mtr_pool[IWL_TFH_TFD_SIZE * IWL_CMD_QUEUE_SIZE];
static uint32_t g_mmio_stub[0x500];

void lx_printk(const char *fmt, ...) { (void)fmt; }
void lx_mdelay(unsigned int ms) { (void)ms; }
void lx_udelay(unsigned int us) { (void)us; }

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

int iwl_fw_cmd_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    (void)iwl;
    (void)group;
    (void)cmd;
    return 0;
}

void iwl_ax211_deliver_rx(const uint8_t *data, int len)
{
    (void)data;
    (void)len;
}

void lx_iwlwifi_set_alive(int alive) { (void)alive; }

void iwl_ax211_add_bss(const struct iwl_ax211_bss *bss) { (void)bss; }

void iwl_mvm_rx_scan_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
{
    (void)iwl;
    (void)frame;
    (void)len;
}

void iwl_mvm_on_scan_complete(struct iwl_ax211_priv *iwl, uint32_t uid, uint8_t status)
{
    (void)iwl;
    (void)uid;
    (void)status;
}

int iwl_fw_upload_sections(struct iwl_ax211_priv *iwl,
                           struct iwl_context_info_dram *dram)
{
    (void)iwl;
    (void)dram;
    return 0;
}

static int check_wide_legacy(uint8_t id, const void *payload, uint16_t pay_len)
{
    struct iwl_ax211_priv iwl;

    memset(&iwl, 0, sizeof(iwl));
    iwl.alive = 1;
    iwl.cmd_qid = IWL_MVM_DQA_CMD_QUEUE;
    iwl.mcr_cpu = g_mcr_pool;
    iwl.mtr_cpu = g_mtr_pool;
    iwl.mcr_dma = 0x1000;
    iwl.mmio = g_mmio_stub;

    iwl.cmd_pending = 0;
    if (iwl_trans_send_cmd(&iwl, LEGACY_GROUP, id, payload, pay_len) != 0)
        return -1;
    iwl.cmd_pending = 0;

    if (sizeof(struct iwl_cmd_header_wide) != 8) {
        fprintf(stderr, "iwl_cmd_header_wide != 8 B\n");
        return -1;
    }
    if (g_mcr_pool[0] != id || g_mcr_pool[1] != LONG_GROUP) {
        fprintf(stderr, "cabecera cmd/grp incorrecta para 0x%02x (DEF_ID → grp=1)\n",
                id);
        return -1;
    }
    if (*(uint16_t *)(g_mcr_pool + 4) != pay_len) {
        fprintf(stderr, "length wide != payload para 0x%02x\n", id);
        return -1;
    }
    /* La ruta legacy antigua usaría cabecera de 4 B; debe haber quedado wide. */
    if (g_mcr_pool[5] != 0 || g_mcr_pool[6] != 0) {
        /* reserved + version en wide; legacy pondría length en offset 5. */
    }
    return 0;
}

int main(void)
{
    struct iwl_tx_ant_cfg_cmd ant;
    struct iwl_sf_cfg_cmd sf;
    struct iwl_phy_context_cmd_v1 phy;

    memset(&ant, 0, sizeof(ant));
    memset(&sf, 0, sizeof(sf));
    memset(&phy, 0, sizeof(phy));

    if (check_wide_legacy(TX_ANT_CONFIGURATION_CMD, &ant, (uint16_t)sizeof(ant)) != 0) {
        fprintf(stderr, "TX_ANT no usa cabecera wide\n");
        return -1;
    }
    if (check_wide_legacy(REPLY_SF_CFG_CMD, &sf, (uint16_t)sizeof(sf)) != 0) {
        fprintf(stderr, "REPLY_SF_CFG no usa cabecera wide\n");
        return -1;
    }
    if (check_wide_legacy(PHY_CONTEXT_CMD, &phy, (uint16_t)sizeof(phy)) != 0) {
        fprintf(stderr, "PHY_CONTEXT no usa cabecera wide\n");
        return -1;
    }

    puts("OK: LEGACY HCMD (TX_ANT/SF/PHY) wide 8 B + DEF_ID grp=1");
    return 0;
}
