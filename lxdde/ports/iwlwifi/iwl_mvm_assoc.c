/* MLME 802.11 AUTH+ASSOC, MAC_CONTEXT y claves CCMP. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);
extern int memcmp(const void *a, const void *b, unsigned long n);
extern char *strncpy(char *dst, const char *src, unsigned long n);

#define MAC_CONTEXT_CMD            0x28
#define IWL_MLME_WAIT_ITERS      50
#define IWL_MLME_LISTEN_INT     10u
#define IWL_MLME_BI_DEFAULT      100u
#define IWL_MLME_DTIM_DEFAULT    1u

static int bssid_is_zero(const uint8_t *bssid)
{
    return !bssid[0] && !bssid[1] && !bssid[2] &&
           !bssid[3] && !bssid[4] && !bssid[5];
}

static uint16_t mgmt_stype(const uint8_t *frame)
{
    uint16_t fc = (uint16_t)frame[0] | ((uint16_t)frame[1] << 8);

    return fc & IEEE80211_FC_STYPE_MASK;
}

static unsigned ssid_len_of(const char *ssid)
{
    unsigned n = 0;

    if (!ssid)
        return 0;
    while (ssid[n] && n < IWL_AX211_SSID_MAX)
        n++;
    return n;
}

static const struct iwl_ax211_bss *bss_by_bssid(struct iwl_ax211_priv *iwl,
                                                const uint8_t *bssid)
{
    int i;

    for (i = 0; i < iwl->scan_count; i++) {
        if (!memcmp(iwl->scan[i].bssid, bssid, 6))
            return &iwl->scan[i];
    }
    return 0;
}

static void parse_tim_ie(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
{
    int pos = 24 + 12;

    if (len < 24 + 12)
        return;
    iwl->beacon_int = (uint16_t)frame[32] | ((uint16_t)frame[33] << 8);
    if (!iwl->beacon_int)
        iwl->beacon_int = IWL_MLME_BI_DEFAULT;
    while (pos + 2 <= len) {
        uint8_t id = frame[pos];
        uint8_t elen = frame[pos + 1];

        if (pos + 2 + (int)elen > len)
            break;
        if (id == WLAN_EID_TIM && elen >= 2 && frame[pos + 3]) {
            iwl->dtim_period = frame[pos + 3];
            break;
        }
        pos += 2 + elen;
    }
}

void iwl_mvm_rx_mlme_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
{
    uint16_t stype;

    if (!iwl || !frame || len < 24 || bssid_is_zero(iwl->bssid))
        return;
    if (memcmp(frame + 16, iwl->bssid, 6) != 0)
        return;
    stype = mgmt_stype(frame);
    if (stype == IEEE80211_STYPE_BEACON) {
        parse_tim_ie(iwl, frame, len);
        return;
    }
    if (stype == IEEE80211_STYPE_AUTH) {
        uint16_t seq;
        uint16_t status;

        if (len < 30)
            return;
        seq = (uint16_t)frame[26] | ((uint16_t)frame[27] << 8);
        status = (uint16_t)frame[28] | ((uint16_t)frame[29] << 8);
        if (seq == 2u && status == 0)
            iwl->mlme_auth_ok = 1;
        return;
    }
    if (stype == IEEE80211_STYPE_ASSOC_RESP) {
        uint16_t status;
        uint16_t aid;

        if (len < 30)
            return;
        status = (uint16_t)frame[26] | ((uint16_t)frame[27] << 8);
        aid = (uint16_t)frame[28] | ((uint16_t)frame[29] << 8);
        if (status == 0) {
            iwl->assoc_id = aid & 0x3fffu;
            if (!iwl->assoc_id)
                iwl->assoc_id = 1;
            iwl->mlme_assoc_ok = 1;
        }
    }
}

static int iwl_mvm_mac_context_assoc(struct iwl_ax211_priv *iwl,
                                     const uint8_t *bssid, uint8_t is_assoc)
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
    iwl_mvm_mac_qos_defaults(cmd.ac);
    /* Linux mac-ctxt.c:680–711: is_assoc=1 solo con assoc y dtim_period. */
    if (is_assoc && iwl->dtim_period) {
        cmd.filter_flags = IWL_MAC_FILTER_ACCEPT_GRP;
        cmd.u.sta.is_assoc = 1u;
        cmd.u.sta.bi = iwl_cpu_to_le32(iwl->beacon_int);
        cmd.u.sta.dtim_interval =
            iwl_cpu_to_le32((uint32_t)iwl->beacon_int * iwl->dtim_period);
        cmd.u.sta.listen_interval = iwl_cpu_to_le32(IWL_MLME_LISTEN_INT);
        cmd.u.sta.assoc_id = iwl_cpu_to_le32(iwl->assoc_id);
    } else {
        cmd.filter_flags = IWL_MAC_FILTER_ACCEPT_GRP | IWL_MAC_FILTER_IN_BEACON;
        cmd.u.sta.is_assoc = 0u;
    }
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

static uint32_t mgmt_rate_n_flags(struct iwl_ax211_priv *iwl)
{
    uint32_t ant = RATE_MCS_ANT_A_MSK;
    int ver = iwl_fw_cmd_ver(iwl, DATA_PATH_GROUP, TX_CMD);

    if (ver > 8)
        return RATE_MCS_LEGACY_OFDM_MSK | RATE_LEGACY_OFDM_6M | ant;
    return RATE_LEGACY_PLCP_6M | ant;
}

static int iwl_mvm_tx_mgmt(struct iwl_ax211_priv *iwl, const uint8_t *frame, int flen)
{
    uint8_t buf[256];
    uint32_t flags = IWL_TX_FLAGS_CMD_RATE | IWL_TX_FLAGS_ENCRYPT_DIS |
                      IWL_TX_FLAGS_HIGH_PRI;
    uint32_t rate = mgmt_rate_n_flags(iwl);
    unsigned hdr_off;
    uint16_t pay;

    if (flen <= 0 || flen > 200)
        return -1;
    memset(buf, 0, sizeof(buf));
    if (iwl->gen3) {
        struct iwl_tx_cmd_gen3 *cmd = (struct iwl_tx_cmd_gen3 *)buf;

        hdr_off = (unsigned)sizeof(struct iwl_tx_cmd_gen3);
        cmd->len = iwl_cpu_to_le16((uint16_t)flen);
        cmd->flags = iwl_cpu_to_le16((uint16_t)flags);
        cmd->rate_n_flags = iwl_cpu_to_le32(rate);
    } else {
        struct iwl_tx_cmd_gen2 *cmd = (struct iwl_tx_cmd_gen2 *)buf;

        hdr_off = (unsigned)sizeof(struct iwl_tx_cmd_gen2);
        cmd->len = iwl_cpu_to_le16((uint16_t)flen);
        cmd->flags = iwl_cpu_to_le32(flags);
        cmd->rate_n_flags = iwl_cpu_to_le32(rate);
    }
    if (hdr_off + (unsigned)flen > sizeof(buf))
        return -1;
    memcpy(buf + hdr_off, frame, (unsigned)flen);
    pay = (uint16_t)(hdr_off + (unsigned)flen);
    return iwl_trans_send_cmd_async(iwl, DATA_PATH_GROUP, TX_CMD, buf, pay);
}

static void fill_mgmt_hdr(uint8_t *f, uint16_t fc, const uint8_t *sta,
                          const uint8_t *bssid)
{
    f[0] = (uint8_t)(fc & 0xff);
    f[1] = (uint8_t)(fc >> 8);
    f[2] = 0;
    f[3] = 0;
    memcpy(f + 4, bssid, 6);
    memcpy(f + 10, sta, 6);
    memcpy(f + 16, bssid, 6);
    f[22] = 0;
    f[23] = 0;
}

static int build_auth_req(uint8_t *f, const uint8_t *sta, const uint8_t *bssid)
{
    fill_mgmt_hdr(f, IEEE80211_STYPE_AUTH, sta, bssid);
    f[24] = 0;
    f[25] = 0;
    f[26] = 1;
    f[27] = 0;
    f[28] = 0;
    f[29] = 0;
    return 30;
}

static int append_ie(uint8_t *f, int pos, uint8_t id, const uint8_t *body, unsigned len)
{
    f[pos++] = id;
    f[pos++] = (uint8_t)len;
    if (body && len)
        memcpy(f + pos, body, len);
    return pos + (int)len;
}

static int build_assoc_req(struct iwl_ax211_priv *iwl, uint8_t *f,
                           const char *ssid, const uint8_t *bssid)
{
    const struct iwl_ax211_bss *bss = bss_by_bssid(iwl, bssid);
    uint8_t rates[8] = { 0x82, 0x84, 0x8b, 0x96, 0x0c, 0x12, 0x18, 0x24 };
    uint8_t ofdm[8] = { 0x0c, 0x12, 0x18, 0x24, 0x30, 0x48, 0x60, 0x6c };
    uint8_t rsn[20];
    unsigned slen = ssid_len_of(ssid);
    uint16_t cap = WLAN_CAPABILITY_ESS;
    int pos;
    int want_rsn = bss && !bss->open;

    if (want_rsn)
        cap |= WLAN_CAPABILITY_PRIVACY;
    fill_mgmt_hdr(f, IEEE80211_STYPE_ASSOC_REQ, iwl->mac, bssid);
    f[24] = (uint8_t)(cap & 0xff);
    f[25] = (uint8_t)(cap >> 8);
    f[26] = IWL_MLME_LISTEN_INT;
    f[27] = 0;
    pos = 28;
    pos = append_ie(f, pos, WLAN_EID_SSID, (const uint8_t *)ssid, slen);
    if (iwl->channel >= 36)
        pos = append_ie(f, pos, WLAN_EID_SUPP_RATES, ofdm, 8);
    else
        pos = append_ie(f, pos, WLAN_EID_SUPP_RATES, rates, 8);
    if (want_rsn) {
        unsigned n = 0;

        rsn[n++] = 1;
        rsn[n++] = 0;
        rsn[n++] = 0x00;
        rsn[n++] = 0x0f;
        rsn[n++] = 0xac;
        rsn[n++] = WLAN_CIPHER_CCMP128;
        rsn[n++] = 1;
        rsn[n++] = 0;
        rsn[n++] = 0x00;
        rsn[n++] = 0x0f;
        rsn[n++] = 0xac;
        rsn[n++] = WLAN_CIPHER_CCMP128;
        rsn[n++] = 1;
        rsn[n++] = 0;
        rsn[n++] = 0x00;
        rsn[n++] = 0x0f;
        rsn[n++] = 0xac;
        rsn[n++] = WLAN_AKM_PSK;
        rsn[n++] = 0;
        rsn[n++] = 0;
        pos = append_ie(f, pos, WLAN_EID_RSN, rsn, n);
    }
    return pos;
}

static uint8_t assoc_phy_band(uint8_t channel)
{
    if (channel >= 36 && channel <= 196)
        return PHY_BAND_5;
    return PHY_BAND_24;
}

static int iwl_mvm_protect_assoc(struct iwl_ax211_priv *iwl)
{
    struct iwl_time_event_cmd te;
    uint16_t policy;

    memset(&te, 0, sizeof(te));
    te.id_and_color =
        iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(iwl->scan_mac_id, 0));
    te.action = iwl_cpu_to_le32(FW_CTXT_ACTION_ADD);
    te.id = iwl_cpu_to_le32(TE_BSS_STA_AGGRESSIVE_ASSOC);
    te.max_delay = iwl_cpu_to_le32(IWL_MVM_TE_ASSOC_MAX_DELAY_MS);
    te.interval = iwl_cpu_to_le32(1);
    te.duration = iwl_cpu_to_le32(IWL_MVM_TE_SESSION_PROTECTION_MAX_TIME_MS);
    te.repeat = 1;
    te.max_frags = TE_V2_FRAG_NONE;
    policy = (uint16_t)(TE_V2_NOTIF_HOST_EVENT_START |
                        TE_V2_NOTIF_HOST_EVENT_END |
                        TE_V2_START_IMMEDIATELY);
    te.policy = iwl_cpu_to_le16(policy);
    if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, TIME_EVENT_CMD, &te,
                                (uint16_t)sizeof(te),
                                IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
        lx_printk("iwl_mvm: TIME_EVENT assoc falló\n");
        return -1;
    }
    lx_printk("iwl_mvm: TIME_EVENT TE_BSS_STA_AGGRESSIVE_ASSOC ok\n");
    return 0;
}

static int wait_mlme_flag(struct iwl_ax211_priv *iwl, uint8_t *flag)
{
    int i;

    for (i = 0; i < IWL_MLME_WAIT_ITERS; i++) {
        if (*flag)
            return 0;
        iwl_trans_poll(iwl);
        if (*flag)
            return 0;
        lx_mdelay(20);
    }
    return -1;
}

static int iwl_mvm_mlme_auth_assoc(struct iwl_ax211_priv *iwl, const char *ssid,
                                   const uint8_t *bssid)
{
    uint8_t frame[256];
    int flen;

    iwl->mlme_auth_ok = 0;
    iwl->mlme_assoc_ok = 0;
    iwl->assoc_id = 0;
    if (!iwl->beacon_int)
        iwl->beacon_int = IWL_MLME_BI_DEFAULT;
    if (!iwl->dtim_period)
        iwl->dtim_period = IWL_MLME_DTIM_DEFAULT;

    flen = build_auth_req(frame, iwl->mac, bssid);
    if (iwl_mvm_tx_mgmt(iwl, frame, flen) != 0) {
        lx_printk("iwl_mvm: AUTH TX falló\n");
        return -1;
    }
    if (wait_mlme_flag(iwl, &iwl->mlme_auth_ok) != 0) {
        lx_printk("iwl_mvm: AUTH timeout\n");
        return -1;
    }

    flen = build_assoc_req(iwl, frame, ssid, bssid);
    if (iwl_mvm_tx_mgmt(iwl, frame, flen) != 0) {
        lx_printk("iwl_mvm: ASSOC TX falló\n");
        return -1;
    }
    if (wait_mlme_flag(iwl, &iwl->mlme_assoc_ok) != 0) {
        lx_printk("iwl_mvm: ASSOC timeout\n");
        return -1;
    }
    if (!iwl->dtim_period)
        iwl->dtim_period = IWL_MLME_DTIM_DEFAULT;
    if (!iwl->beacon_int)
        iwl->beacon_int = IWL_MLME_BI_DEFAULT;
    if (!iwl->assoc_id)
        iwl->assoc_id = 1;
    return 0;
}

int iwl_mvm_assoc_prepare(struct iwl_ax211_priv *iwl, const char *ssid,
                          const uint8_t *bssid)
{
    iwl->associated = 0;
    iwl->mlme_auth_ok = 0;
    iwl->mlme_assoc_ok = 0;
    strncpy(iwl->ssid, ssid, IWL_AX211_SSID_MAX);
    iwl->ssid[IWL_AX211_SSID_MAX] = '\0';
    memcpy(iwl->bssid, bssid, 6);
    iwl->ap_sta_id = IWL_MVM_AP_STA_ID;

    {
        uint8_t new_band = assoc_phy_band(iwl->channel);
        int cdb_band_change = iwl->binding_added &&
            iwl_fw_has_capa(iwl, IWL_UCODE_TLV_CAPA_BINDING_CDB_SUPPORT) &&
            iwl->phy_band != new_band;

        if (cdb_band_change) {
            if (iwl_mvm_binding_send(iwl, FW_CTXT_ACTION_REMOVE) != 0) {
                lx_printk("iwl_mvm: BINDING assoc REMOVE falló\n");
                return -1;
            }
        }
        if (iwl_mvm_phy_ctxt_changed(iwl, iwl->channel) != 0) {
            lx_printk("iwl_mvm: PHY_CONTEXT assoc falló\n");
            return -1;
        }
        if (!iwl->binding_added) {
            if (iwl_mvm_binding_send(iwl, FW_CTXT_ACTION_ADD) != 0) {
                lx_printk("iwl_mvm: BINDING assoc ADD falló\n");
                return -1;
            }
        }
    }
    if (iwl_mvm_mac_context_assoc(iwl, bssid, 0) != 0) {
        lx_printk("iwl_mvm: MAC_CONTEXT falló\n");
        return -1;
    }
    if (iwl_mvm_add_sta_ap(iwl, bssid) != 0) {
        lx_printk("iwl_mvm: ADD_STA falló\n");
        return -1;
    }
    if (iwl_mvm_protect_assoc(iwl) != 0)
        return -1;
    if (iwl_mvm_mlme_auth_assoc(iwl, ssid, bssid) != 0)
        return -1;
    if (iwl_mvm_mac_context_assoc(iwl, bssid, 1) != 0) {
        lx_printk("iwl_mvm: MAC_CONTEXT is_assoc=1 falló\n");
        return -1;
    }
    iwl->associated = 1;
    lx_printk("iwl_mvm: asociado a '%s' aid=%u dtim=%u bi=%u\n",
              ssid, (unsigned)iwl->assoc_id, (unsigned)iwl->dtim_period,
              (unsigned)iwl->beacon_int);
    return 0;
}

int iwl_mvm_install_key(struct iwl_ax211_priv *iwl, const uint8_t key[16], int key_idx)
{
    struct iwl_mvm_add_sta_key_cmd k;
    uint16_t key_flags;

    if (!iwl->associated || !key)
        return -1;
    if (key_idx < 0 || key_idx > 3)
        return -1;
    memset(&k, 0, sizeof(k));
    key_flags = (uint16_t)(((unsigned)key_idx << STA_KEY_FLG_KEYID_POS) &
                           STA_KEY_FLG_KEYID_MSK);
    key_flags |= STA_KEY_FLG_WEP_KEY_MAP;
    key_flags |= STA_KEY_FLG_CCM;
    k.common.sta_id = iwl->ap_sta_id ? iwl->ap_sta_id : IWL_MVM_AP_STA_ID;
    k.common.key_offset = (uint8_t)key_idx;
    k.common.key_flags = iwl_cpu_to_le16(key_flags);
    memcpy(k.common.key, key, 16);
    return iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, ADD_STA_KEY, &k,
                                   (uint16_t)sizeof(k), IWL_MVM_HCMD_TIMEOUT_MS);
}
