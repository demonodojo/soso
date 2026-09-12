/* MLME mínimo: contexto MAC MODIFY + ADD_STA v10 y claves CCMP. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);
extern char *strncpy(char *dst, const char *src, unsigned long n);

#define MAC_CONTEXT_CMD            0x28
#define ADD_STA_KEY                0x17

struct iwl_mvm_add_sta_key_cmd {
    uint8_t operation;
    uint8_t encryption_type;
    uint8_t key_size;
    uint8_t key_flags;
    uint8_t key[32];
    uint8_t key_offset;
    uint8_t sta_id;
    uint16_t key_idx;
    uint32_t tkip_rx_mic_key;
    uint32_t tkip_tx_mic_key;
    uint8_t mac_addr[6];
    uint16_t reserved;
} __attribute__((packed));

static int iwl_mvm_mac_context_assoc(struct iwl_ax211_priv *iwl, const uint8_t *bssid)
{
    struct iwl_mac_ctx_cmd cmd;
    uint32_t id_color = FW_CMD_ID_AND_COLOR(iwl->scan_mac_id, 0);

    memset(&cmd, 0, sizeof(cmd));
    cmd.id_and_color = id_color;
    cmd.action = FW_CTXT_ACTION_MODIFY;
    cmd.mac_type = IWL_FW_MAC_TYPE_BSS_STA;
    memcpy(cmd.node_addr, iwl->mac, 6);
    memcpy(cmd.bssid_addr, bssid, 6);
    cmd.cck_rates = 0x0fu;
    cmd.ofdm_rates = 0xffu;
    cmd.filter_flags = IWL_MAC_FILTER_ACCEPT_GRP;
    iwl_mvm_mac_qos_defaults(cmd.ac);
    cmd.u.sta.is_assoc = 1u;
    return iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, MAC_CONTEXT_CMD, &cmd,
                                   (uint16_t)sizeof(cmd), IWL_MVM_HCMD_TIMEOUT_MS);
}

static int iwl_mvm_add_sta_ap(struct iwl_ax211_priv *iwl, const uint8_t *bssid)
{
    struct iwl_mvm_add_sta_cmd sta;
    uint32_t flags = STA_FLG_FAT_EN_40MHZ | STA_FLG_MIMO_EN_SISO |
                     STA_FLG_CLASS_AUTH | STA_FLG_CLASS_ASSOC;
    uint32_t flags_msk = STA_FLG_FAT_EN_MSK | STA_FLG_MIMO_EN_MSK |
                         STA_FLG_CLASS_AUTH | STA_FLG_CLASS_ASSOC;
    unsigned pay_len = iwl_mvm_add_sta_cmd_size(iwl);

    memset(&sta, 0, sizeof(sta));
    sta.add_modify = 0;
    sta.sta_id = IWL_MVM_AP_STA_ID;
    sta.mac_id_n_color = iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(iwl->scan_mac_id, 0));
    memcpy(sta.addr, bssid, 6);
    sta.tid_disable_tx = iwl_cpu_to_le16(0xffff);
    sta.station_type = IWL_STA_LINK;
    sta.station_flags = iwl_cpu_to_le32(flags);
    sta.station_flags_msk = iwl_cpu_to_le32(flags_msk);
    return iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, ADD_STA, &sta,
                                   (uint16_t)pay_len, IWL_MVM_HCMD_TIMEOUT_MS);
}

int iwl_mvm_assoc_prepare(struct iwl_ax211_priv *iwl, const char *ssid,
                          const uint8_t *bssid)
{
    if (iwl_mvm_mac_context_assoc(iwl, bssid) != 0) {
        lx_printk("iwl_mvm: MAC_CONTEXT falló\n");
        return -1;
    }
    if (iwl_mvm_add_sta_ap(iwl, bssid) != 0) {
        lx_printk("iwl_mvm: ADD_STA falló\n");
        return -1;
    }
    strncpy(iwl->ssid, ssid, IWL_AX211_SSID_MAX);
    iwl->ssid[IWL_AX211_SSID_MAX] = '\0';
    memcpy(iwl->bssid, bssid, 6);
    iwl->ap_sta_id = IWL_MVM_AP_STA_ID;
    iwl->associated = 1;
    lx_printk("iwl_mvm: asociado a '%s'\n", ssid);
    return 0;
}

int iwl_mvm_install_key(struct iwl_ax211_priv *iwl, const uint8_t key[16], int key_idx)
{
    struct iwl_mvm_add_sta_key_cmd k;

    if (!iwl->associated || !key)
        return -1;
    memset(&k, 0, sizeof(k));
    k.operation = 1;
    k.encryption_type = 1;
    k.key_size = 16;
    k.key_flags = (uint8_t)(key_idx == 0 ? 1 : 2);
    memcpy(k.key, key, 16);
    k.sta_id = iwl->ap_sta_id ? iwl->ap_sta_id : IWL_MVM_AP_STA_ID;
    k.key_idx = (uint16_t)key_idx;
    memcpy(k.mac_addr, iwl->bssid, 6);
    return iwl_trans_send_cmd_wait(iwl, MAC_CONF_GROUP, ADD_STA_KEY, &k,
                                   (uint16_t)sizeof(k), IWL_MVM_HCMD_TIMEOUT_MS);
}
