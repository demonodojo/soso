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

static uint8_t phy_band_from_chan(uint8_t channel)
{
    if (channel >= 36 && channel <= 196)
        return PHY_BAND_5;
    return PHY_BAND_24;
}

static uint32_t phy_lmac_id(struct iwl_ax211_priv *iwl, uint8_t band)
{
    /* Linux iwl_mvm_get_lmac_id (binding.c:168): LMAC 5G solo con CDB (40),
     * no con BINDING_CDB (39). cc-a0-77 declara 39 sí / 40 no. */
    if (!iwl_fw_has_capa(iwl, IWL_UCODE_TLV_CAPA_CDB_SUPPORT) ||
        band == PHY_BAND_24)
        return IWL_LMAC_24G_INDEX;
    return IWL_LMAC_5G_INDEX;
}

int iwl_mvm_binding_send(struct iwl_ax211_priv *iwl, uint32_t action)
{
    struct iwl_binding_cmd_v1 cmd_v1;
    struct iwl_binding_cmd cmd;
    const void *payload;
    uint16_t pay_len;
    unsigned i;

    memset(&cmd_v1, 0, sizeof(cmd_v1));
    cmd_v1.id_and_color = iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(0, 0));
    cmd_v1.action = iwl_cpu_to_le32(action);
    cmd_v1.phy = cmd_v1.id_and_color;
    for (i = 0; i < MAX_MACS_IN_BINDING; i++)
        cmd_v1.macs[i] = iwl_cpu_to_le32(FW_CTXT_INVALID);
    cmd_v1.macs[0] = iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(iwl->scan_mac_id, 0));

    if (iwl_fw_has_capa(iwl, IWL_UCODE_TLV_CAPA_BINDING_CDB_SUPPORT)) {
        memset(&cmd, 0, sizeof(cmd));
        memcpy(&cmd, &cmd_v1, sizeof(cmd_v1));
        cmd.lmac_id = iwl_cpu_to_le32(phy_lmac_id(iwl, iwl->phy_band));
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
    iwl->binding_added = (action == FW_CTXT_ACTION_REMOVE) ? 0 : 1;
    lx_printk("iwl_mvm: BINDING PHY0↔MAC%u action=%u ok\n",
              (unsigned)iwl->scan_mac_id, (unsigned)action);
    return 0;
}

int iwl_mvm_binding_update(struct iwl_ax211_priv *iwl)
{
    uint32_t action;

    if (!iwl->mac_ctxt_added || !iwl->phy_ctxt_added) {
        lx_printk("iwl_mvm: BINDING sin MAC/PHY\n");
        return -1;
    }
    action = iwl->binding_added ? FW_CTXT_ACTION_MODIFY : FW_CTXT_ACTION_ADD;
    return iwl_mvm_binding_send(iwl, action);
}

static int iwl_mvm_binding_add_scan(struct iwl_ax211_priv *iwl)
{
    if (iwl->binding_added)
        return 0;
    if (!iwl->mac_ctxt_added || !iwl->phy_ctxt_added)
        return 0;
    return iwl_mvm_binding_update(iwl);
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

static int iwl_mvm_phy_send_rlc(struct iwl_ax211_priv *iwl)
{
    struct iwl_rlc_config_cmd cmd;
    uint8_t rx = iwl_mvm_valid_rx_ant(iwl);

    memset(&cmd, 0, sizeof(cmd));
    cmd.phy_id = iwl_cpu_to_le32(0);
    cmd.rlc.rx_chain_info =
        iwl_cpu_to_le32(phy_rxchain_info(rx, 2, 2));
    return iwl_trans_send_cmd_wait(iwl, DATA_PATH_GROUP, RLC_CONFIG_CMD, &cmd,
                                   (uint16_t)sizeof(cmd),
                                   IWL_MVM_HCMD_TIMEOUT_MS);
}

static int iwl_mvm_phy_ctxt_apply(struct iwl_ax211_priv *iwl, uint8_t channel,
                                  uint32_t action)
{
    uint8_t band = phy_band_from_chan(channel);
    uint8_t tx = iwl_mvm_valid_tx_ant(iwl);
    uint8_t rx = iwl_mvm_valid_rx_ant(iwl);
    uint32_t rxchain = phy_rxchain_info(rx, 2, 2);
    int ver = iwl_fw_cmd_ver(iwl, LEGACY_GROUP, PHY_CONTEXT_CMD);
    int rlc_ver = iwl_fw_cmd_ver(iwl, DATA_PATH_GROUP, RLC_CONFIG_CMD);
    int use_rlc = rlc_ver >= 2;
    uint32_t lmac = phy_lmac_id(iwl, band);

    if (ver >= 3) {
        struct iwl_phy_context_cmd cmd;

        memset(&cmd, 0, sizeof(cmd));
        cmd.id_and_color = iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(0, 0));
        cmd.action = iwl_cpu_to_le32(action);
        cmd.ci.channel = iwl_cpu_to_le32(channel);
        cmd.ci.band = band;
        cmd.ci.width = IWL_PHY_CHANNEL_MODE20;
        cmd.lmac_id = iwl_cpu_to_le32(lmac);
        cmd.rxchain_info = use_rlc ? 0 : rxchain;
        if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, PHY_CONTEXT_CMD, &cmd,
                                    (uint16_t)sizeof(cmd),
                                    IWL_MVM_HCMD_TIMEOUT_MS) != 0)
            return -1;
    } else {
        struct iwl_phy_context_cmd_v1 cmd;

        memset(&cmd, 0, sizeof(cmd));
        cmd.id_and_color = iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(0, 0));
        cmd.action = iwl_cpu_to_le32(action);
        cmd.ci.band = band;
        cmd.ci.channel = channel;
        cmd.ci.width = IWL_PHY_CHANNEL_MODE20;
        cmd.txchain_info = iwl_cpu_to_le32(tx);
        cmd.rxchain_info = use_rlc ? 0 : rxchain;
        if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, PHY_CONTEXT_CMD, &cmd,
                                    (uint16_t)sizeof(cmd),
                                    IWL_MVM_HCMD_TIMEOUT_MS) != 0)
            return -1;
    }

    if (action != FW_CTXT_ACTION_REMOVE && use_rlc) {
        if (iwl_mvm_phy_send_rlc(iwl) != 0) {
            lx_printk("iwl_mvm: RLC_CONFIG falló\n");
            return -1;
        }
    }

    if (action == FW_CTXT_ACTION_REMOVE) {
        iwl->phy_ctxt_added = 0;
        iwl->phy_channel = 0;
        iwl->phy_band = 0;
    } else {
        iwl->phy_ctxt_added = 1;
        iwl->phy_channel = channel;
        iwl->phy_band = band;
    }
    (void)tx;
    return 0;
}

int iwl_mvm_phy_ctxt_changed(struct iwl_ax211_priv *iwl, uint8_t channel)
{
    uint8_t band;
    uint32_t action = FW_CTXT_ACTION_MODIFY;

    if (!channel)
        return 0;
    band = phy_band_from_chan(channel);
    if (!iwl->phy_ctxt_added) {
        action = FW_CTXT_ACTION_ADD;
    } else if (iwl_fw_has_capa(iwl, IWL_UCODE_TLV_CAPA_BINDING_CDB_SUPPORT) &&
               iwl->phy_band != band) {
        /* Linux phy-ctxt.c:300: CDB y cambio de banda → REMOVE + ADD. */
        if (iwl_mvm_phy_ctxt_apply(iwl, channel, FW_CTXT_ACTION_REMOVE) != 0) {
            lx_printk("iwl_mvm: PHY_CONTEXT REMOVE ch%u falló\n", channel);
            return -1;
        }
        action = FW_CTXT_ACTION_ADD;
    }
    if (iwl_mvm_phy_ctxt_apply(iwl, channel, action) != 0) {
        lx_printk("iwl_mvm: PHY_CONTEXT ch%u falló\n", (unsigned)channel);
        return -1;
    }
    lx_printk("iwl_mvm: PHY_CONTEXT ch%u band=%u action=%u ok (tx=0x%x rx=0x%x)\n",
              (unsigned)channel, (unsigned)band, (unsigned)action,
              (unsigned)iwl_mvm_valid_tx_ant(iwl),
              (unsigned)iwl_mvm_valid_rx_ant(iwl));
    return 0;
}

static int iwl_mvm_phy_ctxt_add_minimal(struct iwl_ax211_priv *iwl)
{
    if (iwl->phy_ctxt_added)
        return 0;
    return iwl_mvm_phy_ctxt_changed(iwl, 6);
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
