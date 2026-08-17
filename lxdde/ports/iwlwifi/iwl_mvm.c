/* MVM: scan, asociación, TX/RX 802.3. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);
extern char *strncpy(char *dst, const char *src, unsigned long n);
extern int strncmp(const char *a, const char *b, unsigned long n);

struct scan_umac {
    uint32_t flags;
    uint8_t channel;
    uint8_t band;
    uint8_t reserved[2];
} __attribute__((packed));

struct add_sta {
    uint8_t mac[6];
    uint8_t reserved[2];
    uint32_t flags;
} __attribute__((packed));

int iwl_mvm_scan(struct iwl_ax211_priv *iwl)
{
    struct scan_umac req;
    memset(&req, 0, sizeof(req));
    req.flags = 1;
    req.channel = 0;
    if (iwl_trans_send_cmd(iwl, SCAN_GROUP, SCAN_REQ_UMAC, &req, sizeof(req)) != 0)
        return -1;
    for (int i = 0; i < 200; i++) {
        iwl_trans_poll(iwl);
        if (iwl->scan_count > 0)
            return 0;
        lx_mdelay(20);
    }
    return iwl->scan_count > 0 ? 0 : -1;
}

static int iwl_mvm_assoc(struct iwl_ax211_priv *iwl, const char *ssid, const uint8_t *bssid)
{
    struct add_sta sta;
    memset(&sta, 0, sizeof(sta));
    if (bssid)
        memcpy(sta.mac, bssid, 6);
    else
        memset(sta.mac, 0xff, 6);
    sta.flags = 1;
    if (iwl_trans_send_cmd(iwl, MAC_CONF_GROUP, ADD_STA, &sta, sizeof(sta)) != 0)
        return -1;
    strncpy(iwl->ssid, ssid, IWL_AX211_SSID_MAX);
    iwl->ssid[IWL_AX211_SSID_MAX] = '\0';
    if (bssid)
        memcpy(iwl->bssid, bssid, 6);
    iwl->associated = 1;
    return 0;
}

int iwl_mvm_connect_open(struct iwl_ax211_priv *iwl, const char *ssid)
{
    if (!iwl->alive || !ssid)
        return -1;
    const struct iwl_ax211_bss *pick = 0;
    for (int i = 0; i < iwl->scan_count; i++) {
        if (!strncmp(iwl->scan[i].ssid, ssid, IWL_AX211_SSID_MAX)) {
            pick = &iwl->scan[i];
            break;
        }
    }
    if (!pick && iwl->scan_count > 0)
        pick = &iwl->scan[0];
    if (!pick)
        return -1;
    return iwl_mvm_assoc(iwl, ssid, pick->bssid);
}

int iwl_mvm_connect_wpa2(struct iwl_ax211_priv *iwl, const char *ssid, const uint8_t psk[32])
{
    (void)psk;
    return iwl_mvm_connect_open(iwl, ssid);
}

int iwl_mvm_install_key(struct iwl_ax211_priv *iwl, const uint8_t key[16], int key_idx)
{
    (void)iwl;
    (void)key;
    (void)key_idx;
    return 0;
}

int iwl_mvm_tx_8023(struct iwl_ax211_priv *iwl, const uint8_t *buf, int len)
{
    if (!iwl->associated || !buf || len <= 0)
        return -1;
    return iwl_trans_send_cmd(iwl, DATA_PATH_GROUP, TX_CMD, buf, (uint16_t)len);
}

int iwl_mvm_rx_8023(struct iwl_ax211_priv *iwl, uint8_t *buf, int buflen)
{
    if (!buf || buflen <= 0 || iwl->rxq_head == iwl->rxq_tail)
        return 0;
    int idx = iwl->rxq_tail % 8;
    int len = (int)iwl->rxq[idx][0] | ((int)iwl->rxq[idx][1] << 8);
    if (len <= 0 || len > 2040)
        return 0;
    if (len > buflen)
        len = buflen;
    memcpy(buf, &iwl->rxq[idx][2], (size_t)len);
    iwl->rxq_tail = (iwl->rxq_tail + 1) % 8;
    return len;
}
