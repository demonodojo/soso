/* Fase mínima de `iwl_mvm_up` tras init NVM (shared mem, SF, phy, TX ant, MCC). */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memset(void *dst, int c, unsigned long n);
extern void *memcpy(void *dst, const void *src, unsigned long n);

#define SHARED_MEM_CFG_CMD         0x00
#define REPLY_SF_CFG_CMD           0xd1

#define SF_LONG_DELAY_ON           0
#define SF_FULL_ON                 1
#define SF_UNINIT                  2
#define SF_INIT_OFF                3
#define SF_TRANSIENT_STATES_NUMBER 2
#define SF_NUM_SCENARIO            5
#define SF_NUM_TIMEOUT_TYPES       2

#define SF_W_MARK_SCAN             4096
#define SF_W_MARK_MIMO2            8192
#define SF_LONG_DELAY_AGING_TIMER  1000000

#define FW_CTXT_ID_POS             0
#define FW_CTXT_COLOR_POS          8
#define FW_CMD_ID_AND_COLOR(id, color) \
    (((uint32_t)(id) << FW_CTXT_ID_POS) | ((uint32_t)(color) << FW_CTXT_COLOR_POS))
#define FW_CTXT_ACTION_ADD         1

#define PHY_BAND_24                1
#define IWL_PHY_CHANNEL_MODE20     0

#define PHY_RX_CHAIN_VALID_POS     1
#define PHY_RX_CHAIN_CNT_POS       10
#define PHY_RX_CHAIN_MIMO_CNT_POS  12

struct iwl_sf_cfg_cmd {
    uint32_t state;
    uint32_t watermark[SF_TRANSIENT_STATES_NUMBER];
    uint32_t long_delay_timeouts[SF_NUM_SCENARIO][SF_NUM_TIMEOUT_TYPES];
    uint32_t full_on_timeouts[SF_NUM_SCENARIO][SF_NUM_TIMEOUT_TYPES];
} __attribute__((packed));

struct iwl_fw_channel_info {
    uint32_t channel;
    uint8_t band;
    uint8_t width;
    uint8_t ctrl_pos;
    uint8_t reserved;
} __attribute__((packed));

struct iwl_phy_context_cmd {
    uint32_t id_and_color;
    uint32_t action;
    struct iwl_fw_channel_info ci;
    uint32_t lmac_id;
    uint32_t rxchain_info;
    uint32_t dsp_cfg_flags;
    uint32_t reserved;
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

static uint32_t phy_rxchain_info(uint8_t valid_rx, uint8_t idle, uint8_t active)
{
    uint32_t v = (uint32_t)valid_rx << PHY_RX_CHAIN_VALID_POS;

    v |= (uint32_t)idle << PHY_RX_CHAIN_CNT_POS;
    v |= (uint32_t)active << PHY_RX_CHAIN_MIMO_CNT_POS;
    return iwl_cpu_to_le32(v);
}

static void sf_fill_defaults(struct iwl_sf_cfg_cmd *sf)
{
    static const uint32_t full_on_def[SF_NUM_SCENARIO][SF_NUM_TIMEOUT_TYPES] = {
        { 400, 160 }, { 400, 160 }, { 400, 160 }, { 400, 160 }, { 400, 160 },
    };
    unsigned i, j;

    memset(sf, 0, sizeof(*sf));
    sf->state = iwl_cpu_to_le32(SF_INIT_OFF);
    sf->watermark[SF_LONG_DELAY_ON] = iwl_cpu_to_le32(SF_W_MARK_SCAN);
    sf->watermark[SF_FULL_ON] = iwl_cpu_to_le32(SF_W_MARK_MIMO2);
    for (i = 0; i < SF_NUM_SCENARIO; i++) {
        for (j = 0; j < SF_NUM_TIMEOUT_TYPES; j++) {
            sf->long_delay_timeouts[i][j] =
                iwl_cpu_to_le32(SF_LONG_DELAY_AGING_TIMER);
            sf->full_on_timeouts[i][j] =
                iwl_cpu_to_le32(full_on_def[i][j]);
        }
    }
}

static int iwl_get_shared_mem_conf(struct iwl_ax211_priv *iwl)
{
    /* Linux `fw/smem.c`: pedir offsets; payload vacío. */
    if (iwl_trans_send_cmd_wait(iwl, SYSTEM_GROUP, SHARED_MEM_CFG_CMD,
                                NULL, 0, IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
        lx_printk("iwl_mvm: SHARED_MEM_CFG falló — sigue\n");
        return -1;
    }
    lx_printk("iwl_mvm: SHARED_MEM_CFG ok\n");
    return 0;
}

static int iwl_mvm_sf_init_off(struct iwl_ax211_priv *iwl)
{
    struct iwl_sf_cfg_cmd sf;

    sf_fill_defaults(&sf);
    if (iwl_trans_send_cmd(iwl, LEGACY_GROUP, REPLY_SF_CFG_CMD, &sf,
                           (uint16_t)sizeof(sf)) != 0) {
        lx_printk("iwl_mvm: REPLY_SF_CFG falló\n");
        return -1;
    }
    lx_printk("iwl_mvm: Smart Fifo SF_INIT_OFF\n");
    return 0;
}

static int iwl_configure_rxq(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    /* Gen2 con una cola RX: Linux `iwl_configure_rxq` retorna 0 sin HCMD. */
    return 0;
}

static int iwl_mvm_send_dqa(struct iwl_ax211_priv *iwl)
{
    struct iwl_dqa_enable_cmd cmd;

    memset(&cmd, 0, sizeof(cmd));
    cmd.cmd_queue = iwl_cpu_to_le32(IWL_MVM_DQA_CMD_QUEUE);
    if (iwl_trans_send_cmd_wait(iwl, DATA_PATH_GROUP, DQA_ENABLE_CMD,
                                &cmd, (uint16_t)sizeof(cmd),
                                IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
        lx_printk("iwl_mvm: DQA_ENABLE falló\n");
        return -1;
    }
    lx_printk("iwl_mvm: DQA_ENABLE ok\n");
    return 0;
}

static int iwl_mvm_phy_ctxt_add_minimal(struct iwl_ax211_priv *iwl)
{
    uint8_t tx = iwl_mvm_valid_tx_ant(iwl);
    uint8_t rx = iwl_mvm_valid_rx_ant(iwl);
    uint32_t rxchain = phy_rxchain_info(rx, 2, 2);
    int ver = iwl_fw_cmd_ver(iwl, LEGACY_GROUP, PHY_CONTEXT_CMD);

    if (iwl->phy_ctxt_added)
        return 0;

    if (ver >= 3) {
        struct iwl_phy_context_cmd cmd;

        memset(&cmd, 0, sizeof(cmd));
        cmd.id_and_color = iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(0, 0));
        cmd.action = iwl_cpu_to_le32(FW_CTXT_ACTION_ADD);
        cmd.ci.channel = iwl_cpu_to_le32(6);
        cmd.ci.band = PHY_BAND_24;
        cmd.ci.width = IWL_PHY_CHANNEL_MODE20;
        cmd.lmac_id = iwl_cpu_to_le32(0);
        cmd.rxchain_info = rxchain;
        if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, PHY_CONTEXT_CMD, &cmd,
                                    (uint16_t)sizeof(cmd),
                                    IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
            lx_printk("iwl_mvm: PHY_CONTEXT v%u falló\n", ver);
            return -1;
        }
    } else {
        struct iwl_phy_context_cmd_v1 cmd;

        memset(&cmd, 0, sizeof(cmd));
        cmd.id_and_color = iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(0, 0));
        cmd.action = iwl_cpu_to_le32(FW_CTXT_ACTION_ADD);
        cmd.ci.band = PHY_BAND_24;
        cmd.ci.channel = 6;
        cmd.ci.width = IWL_PHY_CHANNEL_MODE20;
        cmd.txchain_info = iwl_cpu_to_le32(tx);
        cmd.rxchain_info = rxchain;
        if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, PHY_CONTEXT_CMD, &cmd,
                                    (uint16_t)sizeof(cmd),
                                    IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
            lx_printk("iwl_mvm: PHY_CONTEXT v1 falló\n");
            return -1;
        }
    }

    iwl->phy_ctxt_added = 1;
    lx_printk("iwl_mvm: PHY_CONTEXT ch6 2.4GHz ok (tx=0x%x rx=0x%x)\n",
              (unsigned)tx, (unsigned)rx);
    return 0;
}

uint8_t iwl_mvm_scan_rx_ant(struct iwl_ax211_priv *iwl)
{
    if (iwl->scan_rx_ant)
        return iwl->scan_rx_ant;
    return iwl_mvm_valid_rx_ant(iwl);
}

int iwl_mvm_up_minimal(struct iwl_ax211_priv *iwl)
{
    if (!iwl->radio_ready) {
        lx_printk("iwl_mvm: up sin INIT_COMPLETE\n");
        return -1;
    }
    if (iwl->mvm_up_done)
        return 0;

    iwl_get_shared_mem_conf(iwl);
    if (iwl_mvm_sf_init_off(iwl) != 0)
        return -1;

    if (iwl_mvm_send_tx_ant_cfg(iwl) != 0) {
        lx_printk("iwl_mvm: TX ant no configurada — abort up\n");
        return -1;
    }

    if (iwl_configure_rxq(iwl) != 0) {
        lx_printk("iwl_mvm: configure_rxq falló — abort up\n");
        return -1;
    }

    if (iwl_mvm_send_dqa(iwl) != 0) {
        lx_printk("iwl_mvm: DQA no habilitado — abort up\n");
        return -1;
    }

    if (iwl_mvm_phy_ctxt_add_minimal(iwl) != 0) {
        lx_printk("iwl_mvm: phy ctxt no creado — abort up\n");
        return -1;
    }

    if (iwl->lar_enabled && iwl_mvm_init_mcc(iwl) != 0) {
        lx_printk("iwl_mvm: MCC no aplicado — abort up\n");
        return -1;
    }

    if (iwl_mvm_send_scan_cfg(iwl) != 0) {
        lx_printk("iwl_mvm: SCAN_CFG falló — abort up\n");
        return -1;
    }

    iwl->mvm_up_done = 1;
    lx_printk("iwl_mvm: up mínimo listo (phy_sku=0x%08x tx=0x%x rx=0x%x)\n",
              (unsigned)iwl->phy_sku,
              (unsigned)iwl_mvm_valid_tx_ant(iwl),
              (unsigned)iwl_mvm_valid_rx_ant(iwl));
    return 0;
}
