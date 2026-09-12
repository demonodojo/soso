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

#define IWL_PHY_CHANNEL_MODE20     0

#define MAC_CONTEXT_CMD            0x28

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
        lx_printk("iwl_mvm: SHARED_MEM_CFG falló\n");
        return -1;
    }
    lx_printk("iwl_mvm: SHARED_MEM_CFG ok\n");
    return 0;
}

static int iwl_mvm_sf_init_off(struct iwl_ax211_priv *iwl)
{
    struct iwl_sf_cfg_cmd sf;

    sf_fill_defaults(&sf);
    /* Linux `iwl_mvm_sf_update` (`mvm/sf.c`): `CMD_ASYNC`. No se espera 0xd1:
     * el wait síncrono podía cerrar pending sin RX y TX_ANT salía mientras el
     * FW aún procesaba SF (AX200 run13). */
    if (iwl_trans_send_cmd_async(iwl, LEGACY_GROUP, REPLY_SF_CFG_CMD, &sf,
                                 (uint16_t)sizeof(sf)) != 0) {
        lx_printk("iwl_mvm: REPLY_SF_CFG enqueue falló\n");
        return -1;
    }
    lx_printk("iwl_mvm: Smart Fifo SF_INIT_OFF (async)\n");
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

static int iwl_mvm_power_update_device(struct iwl_ax211_priv *iwl)
{
    struct iwl_device_power_cmd cmd;

    memset(&cmd, 0, sizeof(cmd));
    /* Linux `iwl_mvm_power_update_device` con power_scheme CAM: PS off. */
    if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, POWER_TABLE_CMD, &cmd,
                                (uint16_t)sizeof(cmd),
                                IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
        lx_printk("iwl_mvm: POWER_TABLE dispositivo falló\n");
        return -1;
    }
    lx_printk("iwl_mvm: POWER_TABLE dispositivo ok\n");
    return 0;
}

static int iwl_mvm_binding_add_scan(struct iwl_ax211_priv *iwl)
{
    struct iwl_binding_cmd_v1 cmd_v1;
    struct iwl_binding_cmd cmd;
    const void *payload;
    uint16_t pay_len;
    unsigned i;

    if (iwl->binding_added)
        return 0;
    if (!iwl->mac_ctxt_added || !iwl->phy_ctxt_added)
        return 0;

    memset(&cmd_v1, 0, sizeof(cmd_v1));
    cmd_v1.id_and_color = iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(0, 0));
    cmd_v1.action = iwl_cpu_to_le32(FW_CTXT_ACTION_ADD);
    cmd_v1.phy = cmd_v1.id_and_color;
    for (i = 0; i < MAX_MACS_IN_BINDING; i++)
        cmd_v1.macs[i] = iwl_cpu_to_le32(FW_CTXT_INVALID);
    cmd_v1.macs[0] = iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(iwl->scan_mac_id, 0));

    if (iwl_fw_has_capa(iwl, IWL_UCODE_TLV_CAPA_BINDING_CDB_SUPPORT)) {
        memset(&cmd, 0, sizeof(cmd));
        memcpy(&cmd, &cmd_v1, sizeof(cmd_v1));
        cmd.lmac_id = iwl_cpu_to_le32(0);
        payload = &cmd;
        pay_len = (uint16_t)sizeof(cmd);
    } else {
        payload = &cmd_v1;
        pay_len = IWL_BINDING_CMD_SIZE_V1;
    }

    if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, BINDING_CONTEXT_CMD, payload,
                                pay_len, IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
        lx_printk("iwl_mvm: BINDING_CONTEXT falló\n");
        return -1;
    }
    iwl->binding_added = 1;
    lx_printk("iwl_mvm: BINDING PHY0↔MAC%u ok\n",
              (unsigned)iwl->scan_mac_id);
    return 0;
}

static int iwl_mvm_power_update_mac_scan(struct iwl_ax211_priv *iwl)
{
    struct iwl_mac_power_cmd cmd;

    if (!iwl->binding_added)
        return 0;

    memset(&cmd, 0, sizeof(cmd));
    cmd.id_and_color = iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(iwl->scan_mac_id, 0));
    cmd.keep_alive_seconds = iwl_cpu_to_le16(POWER_KEEP_ALIVE_PERIOD_SEC);
    if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, MAC_PM_POWER_TABLE, &cmd,
                                (uint16_t)sizeof(cmd),
                                IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
        lx_printk("iwl_mvm: MAC_PM_POWER_TABLE falló\n");
        return -1;
    }
    lx_printk("iwl_mvm: MAC_PM_POWER_TABLE MAC%u ok\n",
              (unsigned)iwl->scan_mac_id);
    return 0;
}

static int iwl_mvm_mac_ctxt_add_scan(struct iwl_ax211_priv *iwl)
{
    struct iwl_mac_ctx_cmd cmd;

    if (iwl->mac_ctxt_added)
        return 0;

    memset(&cmd, 0, sizeof(cmd));
    cmd.id_and_color = FW_CMD_ID_AND_COLOR(iwl->scan_mac_id, 0);
    cmd.action = FW_CTXT_ACTION_ADD;
    cmd.mac_type = IWL_FW_MAC_TYPE_BSS_STA;
    memcpy(cmd.node_addr, iwl->mac, 6);
    memset(cmd.bssid_addr, 0xff, 6);
    cmd.cck_rates = 0x0fu;
    cmd.ofdm_rates = 0xffu;
    cmd.filter_flags = IWL_MAC_FILTER_ACCEPT_GRP | IWL_MAC_FILTER_IN_BEACON;
    iwl_mvm_mac_qos_defaults(cmd.ac);
    cmd.u.sta.is_assoc = 0u;
    if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, MAC_CONTEXT_CMD, &cmd,
                                (uint16_t)sizeof(cmd),
                                IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
        lx_printk("iwl_mvm: MAC_CONTEXT scan timeout (grp=1) — sigue up\n");
        return 0;
    }
    iwl->mac_ctxt_added = 1;
    lx_printk("iwl_mvm: MAC_CONTEXT scan id=%u ok\n",
              (unsigned)iwl->scan_mac_id);
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

    if (iwl_get_shared_mem_conf(iwl) != 0) {
        lx_printk("iwl_mvm: SHARED_MEM_CFG obligatorio — abort up\n");
        return -1;
    }
    /* SF async: un enqueue fallido se loguea; no se aborta el up con un ACK
     * inventado. TX_ANT sí es síncrono (Linux `iwl_send_tx_ant_cfg`). */
    (void)iwl_mvm_sf_init_off(iwl);

    if (iwl_mvm_send_tx_ant_cfg(iwl) != 0) {
        lx_printk("iwl_mvm: TX ant no configurada — abort up\n");
        return -1;
    }

    if (iwl_configure_rxq(iwl) != 0) {
        lx_printk("iwl_mvm: configure_rxq falló — abort up\n");
        return -1;
    }

    if (iwl_fw_has_capa(iwl, IWL_UCODE_TLV_CAPA_DQA_SUPPORT)) {
        if (iwl_mvm_send_dqa(iwl) != 0) {
            lx_printk("iwl_mvm: DQA no habilitado — abort up\n");
            return -1;
        }
    } else {
        lx_printk("iwl_mvm: DQA omitido (sin CAPA_DQA_SUPPORT)\n");
    }

    if (iwl_mvm_phy_ctxt_add_minimal(iwl) != 0) {
        lx_printk("iwl_mvm: phy ctxt no creado — abort up\n");
        return -1;
    }

    if (iwl_mvm_power_update_device(iwl) != 0) {
        lx_printk("iwl_mvm: power dispositivo no aplicado — abort up\n");
        return -1;
    }

    (void)iwl_mvm_mac_ctxt_add_scan(iwl);

    if (iwl_mvm_binding_add_scan(iwl) != 0) {
        lx_printk("iwl_mvm: binding PHY↔MAC no creado — abort up\n");
        return -1;
    }

    if (iwl_mvm_power_update_mac_scan(iwl) != 0) {
        lx_printk("iwl_mvm: power MAC no aplicado — abort up\n");
        return -1;
    }

    if (iwl->lar_enabled && iwl_mvm_init_mcc(iwl) != 0) {
        /* Sin regdominio aplicado el scan sale solo pasivo (R6). Antes esto
         * abortaba el up: con la validación completa de MCC, una respuesta que
         * no cuadra habría dejado el WiFi sin nada en vez de sin scan activo. */
        lx_printk("iwl_mvm: MCC no aplicado — el scan irá solo pasivo\n");
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
