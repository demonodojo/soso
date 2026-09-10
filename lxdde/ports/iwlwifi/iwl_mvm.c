/* MVM: scan, asociación, TX/RX 802.3. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);
extern int memcmp(const void *a, const void *b, unsigned long n);
extern char *strncpy(char *dst, const char *src, unsigned long n);
extern int strncmp(const char *a, const char *b, unsigned long n);

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

int iwl_mvm_send_scan_cfg(struct iwl_ax211_priv *iwl)
{
    uint8_t ver = (uint8_t)iwl_fw_cmd_ver(iwl, LONG_GROUP, SCAN_CFG_CMD);
    uint16_t pay_len;

    if (iwl->scan_cfg_sent)
        return 0;

    if (ver >= 5) {
        /* Linux scan.c:1238 — API reducida, struct iwl_scan_config (12 B). */
        struct iwl_scan_config cfg;

        memset(&cfg, 0, sizeof(cfg));
        cfg.bcast_sta_id = 0xff;
        cfg.tx_chains = iwl_cpu_to_le32(iwl_mvm_valid_tx_ant(iwl));
        cfg.rx_chains = iwl_cpu_to_le32(iwl_mvm_scan_rx_ant(iwl));
        pay_len = (uint16_t)sizeof(cfg);
        if (iwl_trans_send_cmd_wait(iwl, LONG_GROUP, SCAN_CFG_CMD, &cfg, pay_len,
                                    IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
            lx_printk("iwl_mvm: SCAN_CFG_CMD v%u falló\n", ver);
            return -1;
        }
    } else if (ver >= 2) {
        struct iwl_scan_config_v2 cfg;

        memset(&cfg, 0, sizeof(cfg));
        cfg.bcast_sta_id = 0xff;
        cfg.tx_chains = iwl_cpu_to_le32(iwl_mvm_valid_tx_ant(iwl));
        cfg.rx_chains = iwl_cpu_to_le32(iwl_mvm_scan_rx_ant(iwl));
        pay_len = (uint16_t)sizeof(cfg);
        if (iwl_trans_send_cmd_wait(iwl, LONG_GROUP, SCAN_CFG_CMD, &cfg, pay_len,
                                    IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
            lx_printk("iwl_mvm: SCAN_CFG_CMD v%u falló\n", ver);
            return -1;
        }
    } else {
        struct iwl_scan_config cfg;

        memset(&cfg, 0, sizeof(cfg));
        cfg.bcast_sta_id = 0xff;
        cfg.tx_chains = iwl_cpu_to_le32(iwl_mvm_valid_tx_ant(iwl));
        cfg.rx_chains = iwl_cpu_to_le32(iwl_mvm_scan_rx_ant(iwl));
        pay_len = (uint16_t)sizeof(cfg);
        if (iwl_trans_send_cmd_wait(iwl, LONG_GROUP, SCAN_CFG_CMD, &cfg, pay_len,
                                    IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
            lx_printk("iwl_mvm: SCAN_CFG_CMD falló\n");
            return -1;
        }
    }

    iwl->scan_cfg_sent = 1;
    lx_printk("iwl_mvm: SCAN_CFG_CMD v%u ok tx=0x%x rx=0x%x\n", ver,
              (unsigned)iwl_mvm_valid_tx_ant(iwl),
              (unsigned)iwl_mvm_scan_rx_ant(iwl));
    return 0;
}

/* Índices NVM → número/banda (iwl-nvm-parse.c, primeros 51). */
static const uint8_t nvm_chan_num[] = {
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
    36, 40, 44, 48, 52, 56, 60, 64,
    100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144,
    149, 153, 157, 161, 165, 169, 173, 177, 181,
    183, 184, 185, 187, 188, 189, 192, 196,
};

static unsigned iwl_mvm_collect_scan_channels(struct iwl_ax211_priv *iwl,
                                              uint8_t *ch, uint8_t *band,
                                              uint8_t *passive, unsigned max)
{
    unsigned n = 0;
    unsigned i;
    unsigned nvm_n = iwl->nvm_n_channels;
    unsigned table_n = sizeof(nvm_chan_num) / sizeof(nvm_chan_num[0]);

    if (nvm_n > 0) {
        if (nvm_n > table_n)
            nvm_n = table_n;
        for (i = 0; i < nvm_n && n < max; i++) {
            uint32_t flags = iwl->nvm_chan_flags[i];
            uint8_t num;

            if (!(flags & NVM_CHANNEL_VALID))
                continue;
            num = nvm_chan_num[i];
            ch[n] = num;
            band[n] = (num >= 36) ? 1 : 0;
            passive[n] = (!(flags & NVM_CHANNEL_ACTIVE) || (flags & NVM_CHANNEL_RADAR))
                             ? 1
                             : 0;
            n++;
        }
    }

    if (n == 0) {
        unsigned fallback = sizeof(scan_channels) / sizeof(scan_channels[0]);

        if (fallback > max)
            fallback = max;
        for (i = 0; i < fallback; i++) {
            ch[i] = scan_channels[i].ch;
            band[i] = scan_channels[i].band;
            passive[i] = 0;
        }
        n = fallback;
    }
    return n;
}

int iwl_mvm_scan_umac_supported(uint8_t ver)
{
    return ver == 6 || ver == 14 || ver == 15 || ver == 16 || ver == 17;
}

static uint16_t iwl_build_scan_req_v17(struct iwl_ax211_priv *iwl, uint8_t *buf, unsigned cap)
{
    struct iwl_scan_req_umac_v17 *req;
    unsigned nch;
    unsigned i;
    uint8_t ch[SCAN_MAX_NUM_CHANS_V3];
    uint8_t band[SCAN_MAX_NUM_CHANS_V3];
    uint8_t passive[SCAN_MAX_NUM_CHANS_V3];

    if (cap < sizeof(*req))
        return 0;

    nch = iwl_mvm_collect_scan_channels(iwl, ch, band, passive, SCAN_MAX_NUM_CHANS_V3);
    if (nch == 0)
        return 0;

    memset(buf, 0, sizeof(*req));
    req = (struct iwl_scan_req_umac_v17 *)buf;
    iwl->scan_uid++;
    if (iwl->scan_uid == 0)
        iwl->scan_uid = 1;
    req->uid = iwl_cpu_to_le32(iwl->scan_uid);
    req->ooc_priority = iwl_cpu_to_le32(1);
    req->scan_params.general_params.flags =
        (uint16_t)(IWL_UMAC_SCAN_GEN_FLAGS_V2_PASS_ALL |
                   IWL_UMAC_SCAN_GEN_FLAGS_V2_NTFY_ITER_COMPLETE);
    req->scan_params.general_params.active_dwell[0] = 30;
    req->scan_params.general_params.passive_dwell[0] = 110;
    req->scan_params.channel_params.count = (uint8_t)nch;
    for (i = 0; i < nch; i++) {
        req->scan_params.channel_params.channel_config[i].v2.channel_num = ch[i];
        req->scan_params.channel_params.channel_config[i].v2.band = band[i];
        req->scan_params.channel_params.channel_config[i].v2.iter_count = 1;
        if (passive[i])
            req->scan_params.channel_params.channel_config[i].flags = 1;
    }
    req->scan_params.periodic_params.schedule[0].iter_count = 1;
    req->scan_params.periodic_params.schedule[1].iter_count = 0xff;
    iwl_mvm_fill_probe_req(iwl, &req->scan_params.probe_params);
    return (uint16_t)sizeof(*req);
}

uint16_t iwl_mvm_build_scan_req(struct iwl_ax211_priv *iwl, uint8_t *buf, unsigned cap)
{
    uint8_t scan_ver = (uint8_t)iwl_fw_cmd_ver(iwl, LONG_GROUP, SCAN_REQ_UMAC);

    if (!iwl_mvm_scan_umac_supported(scan_ver))
        return 0;
    if (scan_ver >= 14)
        return iwl_build_scan_req_v17(iwl, buf, cap);

    unsigned nch = sizeof(scan_channels) / sizeof(scan_channels[0]);
    unsigned pay = IWL_SCAN_REQ_UMAC_SIZE_V6 + nch * sizeof(struct iwl_scan_channel_cfg_umac) +
                   sizeof(struct iwl_scan_req_umac_tail_v1);
    uint8_t *data;
    struct iwl_scan_req_umac_tail_v1 *tail;
    unsigned i;

    if (pay > cap)
        return 0;

    memset(buf, 0, pay);
    *(uint16_t *)(buf + 12) =
        (uint16_t)(IWL_UMAC_SCAN_GEN_FLAGS_PASS_ALL | IWL_UMAC_SCAN_GEN_FLAGS_ITER_COMPLETE);
    buf[17] = 30;
    buf[18] = 30;
    buf[19] = 10;
    buf[41] = (uint8_t)nch;

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
    int pos;
    int end;

    if (!frame_is_beacon_or_probe_resp(frame, len))
        return -1;

    if (len < 24 + 12)
        return -1;

    pos = 24 + 12;
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

    memset(ssid, 0, sizeof(ssid));
    struct iwl_ax211_bss bss;
    const uint8_t *bssid;

    if (!iwl->scan_active || len <= 0)
        return;
    slen = parse_mgmt_ssid(frame, len, ssid, 32);
    if (slen <= 0)
        return;
    if (slen < 32)
        ssid[slen] = '\0';
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
    bss.channel = iwl->last_rx_channel;
    bss.open = 1;
    iwl_ax211_add_bss(&bss);
}

void iwl_mvm_on_scan_complete(struct iwl_ax211_priv *iwl, uint32_t uid, uint8_t status)
{
    if (!iwl->scan_active)
        return;
    if (uid != iwl->scan_uid)
        return;
    if (status == IWL_SCAN_OFFLOAD_COMPLETED)
        iwl->scan_end = IWL_SCAN_END_NORMAL;
    else if (status == IWL_SCAN_OFFLOAD_ABORTED)
        iwl->scan_end = IWL_SCAN_END_ABORTED;
    else
        iwl->scan_end = IWL_SCAN_END_ABORTED;
    iwl->scan_complete = 1;
    iwl->scan_active = 0;
}

int iwl_mvm_scan(struct iwl_ax211_priv *iwl)
{
    uint8_t req[IWL_CMD_SLOT_SIZE];
    uint16_t pay;
    int last_count = 0;

    if (!iwl->radio_ready) {
        lx_printk("iwl_mvm: scan sin INIT_COMPLETE\n");
        return -1;
    }
    if (!iwl->mvm_up_done && iwl_mvm_up_minimal(iwl) != 0) {
        lx_printk("iwl_mvm: scan sin up MVM\n");
        return -1;
    }
    if (iwl_hw_rf_kill(iwl) != 0)
        return -1;

    if (iwl->scan_active) {
        iwl->scan_end = IWL_SCAN_END_ABORTED;
        iwl->scan_active = 0;
        lx_printk("iwl_mvm: scan previo abortado uid=0x%x\n",
                  (unsigned)iwl->scan_uid);
    }

    iwl->scan_count = 0;
    iwl->scan_complete = 0;
    iwl->scan_end = IWL_SCAN_END_NONE;
    iwl->scan_active = 1;

    if (iwl_mvm_send_scan_cfg(iwl) != 0) {
        iwl->scan_active = 0;
        return -1;
    }

    pay = iwl_mvm_build_scan_req(iwl, req, sizeof(req));
    if (pay == 0) {
        lx_printk("iwl_mvm: SCAN_REQ versión no soportada o no cabe\n");
        iwl->scan_active = 0;
        return -1;
    }

    lx_printk("iwl_mvm: SCAN_REQ_UMAC %u B uid=0x%x\n", pay, (unsigned)iwl->scan_uid);

    if (iwl_trans_send_cmd_wait(iwl, LONG_GROUP, SCAN_REQ_UMAC, req, pay, 1000) != 0) {
        lx_printk("iwl_mvm: SCAN_REQ_UMAC rechazado\n");
        iwl->scan_active = 0;
        iwl->scan_end = IWL_SCAN_END_NONE;
        return -1;
    }

    for (int i = 0; i < 400; i++) {
        iwl_trans_poll(iwl);
        if (iwl->scan_count != last_count) {
            lx_printk("iwl_mvm: scan_count=%d\n", iwl->scan_count);
            last_count = iwl->scan_count;
        }
        if (iwl->scan_end != IWL_SCAN_END_NONE)
            break;
        lx_mdelay(20);
    }

    if (iwl->scan_end == IWL_SCAN_END_NONE) {
        iwl->scan_end = IWL_SCAN_END_TIMEOUT;
        iwl->scan_active = 0;
    }

    lx_printk("iwl_mvm: scan fin count=%d end=%d uid=0x%x GP=0x%08x\n",
              iwl->scan_count, iwl->scan_end, (unsigned)iwl->scan_uid,
              iwl_read32(iwl, CSR_GP_CNTRL));

    if (iwl->scan_end == IWL_SCAN_END_NORMAL)
        return 0;
    if (iwl->scan_end == IWL_SCAN_END_ABORTED)
        return -1;
    return -2;
}

static int iwl_mvm_assoc(struct iwl_ax211_priv *iwl, const char *ssid, const uint8_t *bssid)
{
    if (!bssid)
        return -1;
    return iwl_mvm_assoc_prepare(iwl, ssid, bssid);
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
