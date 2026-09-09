/* MVM: scan, asociación, TX/RX 802.3. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);
extern int memcmp(const void *a, const void *b, unsigned long n);
extern char *strncpy(char *dst, const char *src, unsigned long n);
extern int strncmp(const char *a, const char *b, unsigned long n);

struct add_sta {
    uint8_t mac[6];
    uint8_t reserved[2];
    uint32_t flags;
} __attribute__((packed));

static const struct {
    uint8_t ch;
    uint8_t band;
} scan_channels[] = {
    {1, 0}, {2, 0}, {3, 0}, {4, 0}, {5, 0}, {6, 0}, {7, 0},
    {8, 0}, {9, 0}, {10, 0}, {11, 0}, {12, 0}, {13, 0},
    {36, 1}, {40, 1}, {44, 1}, {48, 1}, {149, 1}, {153, 1}, {157, 1}, {161, 1},
};

static uint32_t iwl_read32(struct iwl_ax211_priv *iwl, uint32_t off)
{
    return iwl->mmio[off / 4];
}

static int iwl_hw_rf_kill(struct iwl_ax211_priv *iwl)
{
    uint32_t gp = iwl_read32(iwl, CSR_GP_CNTRL);
    if (!(gp & CSR_GP_CNTRL_REG_FLAG_HW_RF_KILL_SW)) {
        lx_printk("iwl_mvm: RF-kill activo GP_CNTRL=0x%08x\n", gp);
        return -1;
    }
    return 0;
}

static int iwl_send_scan_cfg(struct iwl_ax211_priv *iwl)
{
    struct iwl_scan_config cfg;

    if (iwl->scan_cfg_sent)
        return 0;
    memset(&cfg, 0, sizeof(cfg));
    cfg.bcast_sta_id = 0xff;
    cfg.tx_chains = 1;
    cfg.rx_chains = 1;
    if (iwl_trans_send_cmd(iwl, LONG_GROUP, SCAN_CFG_CMD, &cfg, sizeof(cfg)) != 0) {
        lx_printk("iwl_mvm: SCAN_CFG_CMD falló\n");
        return -1;
    }
    iwl->scan_cfg_sent = 1;
    return 0;
}

static uint16_t iwl_build_scan_req(struct iwl_ax211_priv *iwl, uint8_t *buf, unsigned cap)
{
    unsigned nch = sizeof(scan_channels) / sizeof(scan_channels[0]);
    unsigned pay = IWL_SCAN_REQ_UMAC_SIZE_V6 + nch * sizeof(struct iwl_scan_channel_cfg_umac) +
                   sizeof(struct iwl_scan_req_umac_tail_v1);
    uint8_t *data;
    struct iwl_scan_req_umac_tail_v1 *tail;
    unsigned i;

    if (pay > cap)
        return 0;

    memset(buf, 0, pay);

    /* SCAN_REQUEST_CMD_UMAC_API v6 — 44 B hasta channel.count */
    *(uint16_t *)(buf + 12) =
        (uint16_t)(IWL_UMAC_SCAN_GEN_FLAGS_PASS_ALL | IWL_UMAC_SCAN_GEN_FLAGS_ITER_COMPLETE);
    buf[17] = 30; /* active_dwell */
    buf[18] = 30; /* passive_dwell */
    buf[19] = 10; /* fragmented_dwell */
    buf[41] = (uint8_t)nch; /* channel.count */

    data = buf + IWL_SCAN_REQ_UMAC_SIZE_V6;
    for (i = 0; i < nch; i++) {
        struct iwl_scan_channel_cfg_umac *ch =
            (struct iwl_scan_channel_cfg_umac *)(data + i * sizeof(*ch));
        ch->v2.channel_num = scan_channels[i].ch;
        ch->v2.band = scan_channels[i].band;
        ch->v2.iter_count = 1;
        ch->v2.iter_interval = 0;
    }

    tail = (struct iwl_scan_req_umac_tail_v1 *)(data + nch * sizeof(struct iwl_scan_channel_cfg_umac));
    tail->schedule[0].interval = 0;
    tail->schedule[0].iter_count = 1;
    tail->schedule[1].iter_count = 0xff;
    tail->delay = 0;

    (void)iwl;
    return (uint16_t)pay;
}

static int frame_is_beacon_or_probe_resp(const uint8_t *frame, int len)
{
    if (len < 2)
        return 0;
    uint16_t fc = (uint16_t)frame[0] | ((uint16_t)frame[1] << 8);
    uint8_t type = (uint8_t)((fc >> 2) & 3u);
    uint8_t subtype = (uint8_t)((fc >> 4) & 0xfu);
    return type == 0 && (subtype == 8 || subtype == 5);
}

static int parse_mgmt_ssid(const uint8_t *frame, int len, uint8_t *ssid, int ssid_max)
{
    int hdrlen;
    int fixed;
    int pos;
    int end;

    if (!frame_is_beacon_or_probe_resp(frame, len))
        return -1;

    hdrlen = 24;
    if (len < hdrlen + 12)
        return -1;

    fixed = (frame[0] & 0xf0) == 0x50 ? 14 : 12; /* probe resp vs beacon */
    pos = hdrlen + fixed;
    end = len;
    while (pos + 2 <= end) {
        uint8_t id = frame[pos];
        uint8_t elen = frame[pos + 1];
        if (pos + 2 + elen > end)
            break;
        if (id == 0 && elen > 0 && elen <= ssid_max) {
            memcpy(ssid, &frame[pos + 2], elen);
            return elen;
        }
        pos += 2 + elen;
    }
    return -1;
}

void iwl_mvm_rx_scan_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
{
    uint8_t ssid[32];
    int slen;
    struct iwl_ax211_bss bss;
    const uint8_t *bssid;

    if (!iwl->scan_active || len <= 0)
        return;
    slen = parse_mgmt_ssid(frame, len, ssid, 32);
    if (slen <= 0)
        return;
    if (len < 24)
        return;
    bssid = frame + 16;
    for (int i = 0; i < iwl->scan_count; i++) {
        if (!memcmp(iwl->scan[i].bssid, bssid, 6) &&
            !strncmp(iwl->scan[i].ssid, (const char *)ssid, IWL_AX211_SSID_MAX))
            return;
    }
    memset(&bss, 0, sizeof(bss));
    memcpy(bss.bssid, bssid, 6);
    memcpy(bss.ssid, ssid, (size_t)slen);
    bss.ssid[slen] = '\0';
    bss.rssi = iwl->last_rx_rssi ? iwl->last_rx_rssi : -70;
    bss.channel = iwl->last_rx_channel ? iwl->last_rx_channel : 6;
    bss.open = 1;
    iwl_ax211_add_bss(&bss);
}

int iwl_mvm_scan(struct iwl_ax211_priv *iwl)
{
    uint8_t req[IWL_CMD_SLOT_SIZE];
    uint16_t pay;
    int last_count = 0;
    int notifs = 0;

    if (iwl_hw_rf_kill(iwl) != 0)
        return -1;

    iwl->scan_count = 0;
    iwl->scan_complete = 0;
    iwl->scan_active = 1;

    if (iwl_send_scan_cfg(iwl) != 0) {
        iwl->scan_active = 0;
        return -1;
    }

    pay = iwl_build_scan_req(iwl, req, sizeof(req));
    if (pay == 0) {
        lx_printk("iwl_mvm: SCAN_REQ demasiado grande\n");
        iwl->scan_active = 0;
        return -1;
    }

    lx_printk("iwl_mvm: SCAN_REQ_UMAC %u B, %u canales\n", pay,
              (unsigned)(sizeof(scan_channels) / sizeof(scan_channels[0])));

    if (iwl_trans_send_cmd(iwl, LONG_GROUP, SCAN_REQ_UMAC, req, pay) != 0) {
        lx_printk("iwl_mvm: SCAN_REQ_UMAC rechazado\n");
        iwl->scan_active = 0;
        return -1;
    }

    for (int i = 0; i < 400; i++) {
        iwl_trans_poll(iwl);
        if (iwl->scan_count != last_count) {
            lx_printk("iwl_mvm: scan_count=%d\n", iwl->scan_count);
            last_count = iwl->scan_count;
        }
        if (iwl->scan_complete)
            notifs++;
        if (iwl->scan_complete && iwl->scan_count > 0)
            break;
        if (iwl->scan_complete && i > 20)
            break;
        lx_mdelay(20);
    }

    iwl->scan_active = 0;
    lx_printk("iwl_mvm: scan fin count=%d complete=%d notif=%d GP=0x%08x\n",
              iwl->scan_count, iwl->scan_complete, notifs,
              iwl_read32(iwl, CSR_GP_CNTRL));

    if (iwl->scan_count > 0)
        return 0;
    if (iwl->scan_complete)
        return -1;
    return -2;
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
