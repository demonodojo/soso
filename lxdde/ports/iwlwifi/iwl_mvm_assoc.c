/* MLME mínimo: contexto MAC, ADD_STA y claves CCMP. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);
extern char *strncpy(char *dst, const char *src, unsigned long n);

#define MAC_CONTEXT_CMD            0x28
#define ADD_STA_KEY                0x17
#define IWL_MAC_CTXT_ACTION_ADD    1u
#define IWL_MAC_TYPE_BSS_STA       1u
#define MAC_FILTER_IN_BEACON       (1u << 3)

struct iwl_mvm_add_sta_cmd {
    uint8_t add_modify;
    uint8_t awake_acs;
    uint8_t tid_disable_tx;
    uint8_t reserved;
    uint32_t mac_id_n_color;
    uint8_t addr[6];
    uint16_t reserved2;
    uint32_t station_flags;
    uint32_t station_type;
} __attribute__((packed));

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

static int iwl_mvm_mac_context_sta(struct iwl_ax211_priv *iwl, const uint8_t *bssid)
{
    uint8_t cmd[148];

    memset(cmd, 0, sizeof(cmd));
    *(uint32_t *)(cmd + 4) = IWL_MAC_CTXT_ACTION_ADD;
    *(uint32_t *)(cmd + 8) = IWL_MAC_TYPE_BSS_STA;
    memcpy(cmd + 16, iwl->mac, 6);
    memcpy(cmd + 24, bssid, 6);
    *(uint32_t *)(cmd + 32) = 0x0fu;
    *(uint32_t *)(cmd + 36) = 0xffu;
    *(uint32_t *)(cmd + 56) = MAC_FILTER_IN_BEACON;
    *(uint32_t *)(cmd + 60) = 1u;
    return iwl_trans_send_cmd_wait(iwl, MAC_CONF_GROUP, MAC_CONTEXT_CMD, cmd,
                                   (uint16_t)sizeof(cmd), 500);
}

static int iwl_mvm_add_sta_ap(struct iwl_ax211_priv *iwl, const uint8_t *bssid)
{
    struct iwl_mvm_add_sta_cmd sta;

    memset(&sta, 0, sizeof(sta));
    sta.add_modify = 1;
    sta.mac_id_n_color = 0;
    memcpy(sta.addr, bssid, 6);
    sta.station_flags = 1;
    sta.station_type = 1;
    return iwl_trans_send_cmd_wait(iwl, MAC_CONF_GROUP, ADD_STA, &sta,
                                   (uint16_t)sizeof(sta), 500);
}

int iwl_mvm_assoc_prepare(struct iwl_ax211_priv *iwl, const char *ssid,
                          const uint8_t *bssid)
{
    if (iwl_mvm_mac_context_sta(iwl, bssid) != 0) {
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
    k.sta_id = 0;
    k.key_idx = (uint16_t)key_idx;
    memcpy(k.mac_addr, iwl->bssid, 6);
    return iwl_trans_send_cmd_wait(iwl, MAC_CONF_GROUP, ADD_STA_KEY, &k,
                                   (uint16_t)sizeof(k), 500);
}
