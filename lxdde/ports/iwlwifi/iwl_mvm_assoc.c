/* MLME 802.11 AUTH+ASSOC, MAC_CONTEXT y claves CCMP. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);
extern int memcmp(const void *a, const void *b, unsigned long n);
extern char *strncpy(char *dst, const char *src, unsigned long n);

#define MAC_CONTEXT_CMD            0x28
#define IWL_MLME_WAIT_ITERS           50
#define IWL_MLME_BEACON_WAIT_ITERS   100
#define IWL_MLME_LISTEN_INT           10u
#define IWL_MLME_BI_DEFAULT          100u

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
        if (id == WLAN_EID_TIM && elen >= 2) {
            iwl->sync_dtim_count = frame[pos + 2];
            if (frame[pos + 3])
                iwl->dtim_period = frame[pos + 3];
            iwl->sync_beacon_seen = 1;
            lx_printk("iwl_mvm: beacon TIM dtim_count=%u dtim_period=%u bi=%u tsf=%llu gp2=%u\n",
                      (unsigned)iwl->sync_dtim_count, (unsigned)iwl->dtim_period,
                      (unsigned)iwl->beacon_int,
                      (unsigned long long)iwl->sync_tsf,
                      (unsigned)iwl->sync_device_ts);
            break;
        }
        pos += 2 + elen;
    }
}

static int mlme_frame_for_us(struct iwl_ax211_priv *iwl, const uint8_t *frame,
                             int len, uint16_t stype)
{
    if (!iwl || !frame || len < 24 || bssid_is_zero(iwl->bssid))
        return 0;
    if (stype == IEEE80211_STYPE_BEACON)
        return memcmp(frame + 16, iwl->bssid, 6) == 0;
    /* AUTH/ASSOC: addr1=STA, addr2/addr3=AP (Linux mac80211). */
    if (memcmp(frame + 4, iwl->mac, 6) != 0)
        return 0;
    if (memcmp(frame + 10, iwl->bssid, 6) != 0 &&
        memcmp(frame + 16, iwl->bssid, 6) != 0)
        return 0;
    return 1;
}

void iwl_mvm_rx_mlme_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
{
    uint16_t stype;

    if (!iwl || !frame || len < 24)
        return;
    stype = mgmt_stype(frame);
    if (!mlme_frame_for_us(iwl, frame, len, stype))
        return;
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
        lx_printk("iwl_mvm: rx AUTH seq=%u status=%u\n",
                  (unsigned)seq, (unsigned)status);
        if (seq == 2u && status == 0) {
            iwl->mlme_auth_ok = 1;
            lx_printk("iwl_mvm: mlme_auth_ok\n");
        }
        return;
    }
    if (stype == IEEE80211_STYPE_ASSOC_RESP) {
        uint16_t status;
        uint16_t aid;

        if (len < 30)
            return;
        status = (uint16_t)frame[26] | ((uint16_t)frame[27] << 8);
        aid = (uint16_t)frame[28] | ((uint16_t)frame[29] << 8);
        lx_printk("iwl_mvm: rx ASSOC status=%u aid=%u\n",
                  (unsigned)status, (unsigned)(aid & 0x3fffu));
        if (status == 0) {
            iwl->assoc_id = aid & 0x3fffu;
            if (!iwl->assoc_id)
                iwl->assoc_id = 1;
            iwl->mlme_assoc_ok = 1;
        }
    }
}

static void iwl_mvm_set_fw_dtim_tbtt(struct iwl_ax211_priv *iwl,
                                     uint64_t *dtim_tsf, uint32_t *dtim_time,
                                     uint32_t *assoc_beacon_arrive_time)
{
    uint32_t dtim_offs;

    dtim_offs = (uint32_t)iwl->sync_dtim_count * (uint32_t)iwl->beacon_int;
    dtim_offs *= 1024u;
    *dtim_tsf = iwl->sync_tsf + dtim_offs;
    *dtim_time = iwl->sync_device_ts + dtim_offs;
    *assoc_beacon_arrive_time = iwl->sync_device_ts;
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
    cmd.filter_flags = IWL_MAC_FILTER_ACCEPT_GRP;
    /* Linux mac-ctxt.c:680–690: is_assoc=1 solo con assoc y dtim_period. */
    if (is_assoc && iwl->dtim_period) {
        cmd.u.sta.is_assoc = 1u;
        iwl_mvm_set_fw_dtim_tbtt(iwl, &cmd.u.sta.dtim_tsf, &cmd.u.sta.dtim_time,
                                 &cmd.u.sta.assoc_beacon_arrive_time);
    } else {
        cmd.filter_flags |= IWL_MAC_FILTER_IN_BEACON;
        if (iwl->auth_ctl_filter)
            cmd.filter_flags |= IWL_MAC_FILTER_IN_CONTROL_AND_MGMT;
        cmd.u.sta.is_assoc = 0u;
    }
    /* Linux mac-ctxt.c:706–711: bi/dtim_interval/listen/aid siempre. */
    {
        uint32_t bi = iwl->beacon_int ? iwl->beacon_int : IWL_MLME_BI_DEFAULT;

        cmd.u.sta.bi = iwl_cpu_to_le32(bi);
        cmd.u.sta.dtim_interval = iwl_cpu_to_le32(bi * (uint32_t)iwl->dtim_period);
        cmd.u.sta.listen_interval = iwl_cpu_to_le32(IWL_MLME_LISTEN_INT);
        cmd.u.sta.assoc_id = iwl_cpu_to_le32(iwl->assoc_id ? iwl->assoc_id : 1u);
    }
    return iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, MAC_CONTEXT_CMD, &cmd,
                                   (uint16_t)sizeof(cmd), IWL_MVM_HCMD_TIMEOUT_MS);
}

static int iwl_mvm_add_sta_ap(struct iwl_ax211_priv *iwl, const uint8_t *bssid)
{
    struct iwl_mvm_add_sta_cmd sta;
    /* Linux mvm/sta.c:136–138: FAT|MIMO (y RTS_MIMO_PROT). CLASS_AUTH/ASSOC
     * significan «ya autenticada/asociada»; ningún .c de iwlwifi las pone en
     * el ADD. Sin IE HT almacenado, 20 MHz SISO (canal 2.4 típico). */
    uint32_t flags = STA_FLG_FAT_EN_20MHZ | STA_FLG_MIMO_EN_SISO;
    uint32_t flags_msk = STA_FLG_FAT_EN_MSK | STA_FLG_MIMO_EN_MSK;
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
    if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, ADD_STA, &sta,
                                (uint16_t)pay_len, IWL_MVM_HCMD_TIMEOUT_MS) != 0)
        return -1;
    if (iwl->cmd_resp_len < (uint16_t)sizeof(uint32_t)) {
        lx_printk("iwl_mvm: ADD_STA resp corta (%u B)\n",
                  (unsigned)iwl->cmd_resp_len);
        return -1;
    }
    {
        uint32_t status;

        memcpy(&status, iwl->cmd_resp, sizeof(status));

        lx_printk("iwl_mvm: ADD_STA status=0x%08x\n", status);
        if ((status & IWL_ADD_STA_STATUS_MASK) != ADD_STA_SUCCESS) {
            lx_printk("iwl_mvm: ADD_STA rechazado (status=0x%08x)\n", status);
            return -1;
        }
    }
    return 0;
}

static int iwl_mvm_tx_mgmt(struct iwl_ax211_priv *iwl, const uint8_t *frame, int flen)
{
    uint8_t buf[256];
    /* Sin rate scale (rs.c): Linux usa CMD_RATE cuando no hay STA/LQ. */
    uint32_t flags = IWL_TX_FLAGS_ENCRYPT_DIS | IWL_TX_FLAGS_HIGH_PRI |
                       IWL_TX_FLAGS_CMD_RATE;
    uint32_t rate = iwl_mvm_tx_rate_n_flags(iwl);
    unsigned hdr_off;
    uint16_t pay;
    /* Linux tx.c: mh_len/2 << TX_CMD_OFFLD_MH_SIZE; AUTH/ASSOC sin QoS = 24 B. */
    uint16_t offload_assist = (uint16_t)((24u / 2u) << TX_CMD_OFFLD_MH_SIZE);

    if (flen <= 0 || flen > 200)
        return -1;
    memset(buf, 0, sizeof(buf));
    if (iwl->gen3) {
        struct iwl_tx_cmd_gen3 *cmd = (struct iwl_tx_cmd_gen3 *)buf;

        hdr_off = (unsigned)sizeof(struct iwl_tx_cmd_gen3);
        cmd->len = iwl_cpu_to_le16((uint16_t)flen);
        cmd->flags = iwl_cpu_to_le16((uint16_t)flags);
        cmd->offload_assist = iwl_cpu_to_le32((uint32_t)offload_assist);
        cmd->rate_n_flags = iwl_cpu_to_le32(rate);
    } else {
        struct iwl_tx_cmd_gen2 *cmd = (struct iwl_tx_cmd_gen2 *)buf;

        hdr_off = (unsigned)sizeof(struct iwl_tx_cmd_gen2);
        cmd->len = iwl_cpu_to_le16((uint16_t)flen);
        cmd->offload_assist = offload_assist;
        cmd->flags = iwl_cpu_to_le32(flags);
        cmd->rate_n_flags = iwl_cpu_to_le32(rate);
    }
    if (hdr_off + (unsigned)flen > sizeof(buf))
        return -1;
    memcpy(buf + hdr_off, frame, (unsigned)flen);
    pay = (uint16_t)(hdr_off + (unsigned)flen);
    if (!iwl->mgmt_txq_ready)
        return -1;
    return iwl_trans_tx(iwl, iwl->mgmt_txq_id, buf, pay);
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

/* Cuerpo del RSN IE (sin id ni longitud): versión 1, grupo y par CCMP-128,
 * AKM PSK, capacidades a cero. */
static unsigned rsn_ie_body(uint8_t *rsn, unsigned max)
{
    unsigned n = 0;

    if (max < 20)
        return 0;
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
    return n;
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
        unsigned n = rsn_ie_body(rsn, sizeof(rsn));

        pos = append_ie(f, pos, WLAN_EID_RSN, rsn, n);
    }
    return pos;
}

/* El RSN IE completo (con el 0x30 y la longitud) que esta estación anuncia.
 * El supplicant lo repite en M2: si los bytes no coinciden con los de la
 * Association Request, el AP corta el handshake. */
int iwl_mvm_rsn_ie(const struct iwl_ax211_priv *iwl, uint8_t *out, int max)
{
    uint8_t body[20];
    unsigned n;

    (void)iwl;
    if (!out)
        return -1;
    n = rsn_ie_body(body, sizeof(body));
    if (max < (int)n + 2)
        return -1;
    out[0] = WLAN_EID_RSN;
    out[1] = (uint8_t)n;
    memcpy(out + 2, body, n);
    return (int)n + 2;
}

static uint8_t assoc_phy_band(uint8_t channel)
{
    if (channel >= 36 && channel <= 196)
        return PHY_BAND_5;
    return PHY_BAND_24;
}

static int iwl_mvm_protect_assoc(struct iwl_ax211_priv *iwl)
{
    /* Linux mac80211.c:2471 — AX200/cc-a0-77 declara SESSION_PROT (54) y no
     * implementa TIME_EVENT 0x29. */
    if (iwl_fw_has_capa(iwl, IWL_UCODE_TLV_CAPA_SESSION_PROT_CMD)) {
        struct iwl_mvm_session_prot_cmd sp;

        memset(&sp, 0, sizeof(sp));
        sp.id_and_color =
            iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(iwl->scan_mac_id, 0));
        sp.action = iwl_cpu_to_le32(FW_CTXT_ACTION_ADD);
        sp.conf_id = iwl_cpu_to_le32(SESSION_PROTECT_CONF_ASSOC);
        sp.duration_tu =
            iwl_cpu_to_le32(MSEC_TO_TU(IWL_MVM_SESSION_PROTECTION_ASSOC_MS));
        if (iwl_trans_send_cmd_wait(iwl, MAC_CONF_GROUP, SESSION_PROTECTION_CMD,
                                    &sp, (uint16_t)sizeof(sp),
                                    IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
            lx_printk("iwl_mvm: SESSION_PROTECTION assoc falló\n");
            return -1;
        }
        lx_printk("iwl_mvm: SESSION_PROTECTION CONF_ASSOC ok (%u TU)\n",
                  (unsigned)MSEC_TO_TU(IWL_MVM_SESSION_PROTECTION_ASSOC_MS));
        return 0;
    }

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
    }
    return 0;
}

static int wait_mlme_flag(struct iwl_ax211_priv *iwl, uint8_t *flag)
{
    int i;

    for (i = 0; i < IWL_MLME_WAIT_ITERS; i++) {
        unsigned p;

        if (*flag)
            return 0;
        for (p = 0; p < 4; p++)
            iwl_trans_poll(iwl);
        if (*flag)
            return 0;
        lx_mdelay(20);
    }
    return -1;
}

static int iwl_mvm_wait_assoc_beacon(struct iwl_ax211_priv *iwl)
{
    int i;

    for (i = 0; i < IWL_MLME_BEACON_WAIT_ITERS; i++) {
        unsigned p;

        if (iwl->dtim_period && iwl->sync_beacon_seen)
            return 0;
        for (p = 0; p < 4; p++)
            iwl_trans_poll(iwl);
        if (iwl->dtim_period && iwl->sync_beacon_seen)
            return 0;
        lx_mdelay(20);
    }
    lx_printk("iwl_mvm: beacon/DTIM timeout (dtim=%u sync=%u)\n",
              (unsigned)iwl->dtim_period, (unsigned)iwl->sync_beacon_seen);
    return -1;
}

static int iwl_mvm_mlme_auth_assoc(struct iwl_ax211_priv *iwl, const char *ssid,
                                   const uint8_t *bssid)
{
    uint8_t frame[256];
    int flen;
    int attempt;

    iwl->mlme_auth_ok = 0;
    iwl->mlme_assoc_ok = 0;
    iwl->assoc_id = 0;
    iwl->last_mgmt_tx_status = 0;
    /* auth_ctl_filter queda como lo dejó assoc_prepare (IN_CONTROL_AND_MGMT). */

    for (attempt = 0; attempt < 2; attempt++) {
        if (attempt == 1) {
            if (iwl->last_mgmt_tx_status != 0 &&
                iwl->last_mgmt_tx_status != TX_STATUS_SUCCESS) {
                lx_printk("iwl_mvm: AUTH sin retry ctl-filter (tx status=0x%02x)\n",
                          (unsigned)iwl->last_mgmt_tx_status);
                break;
            }
            if (iwl->last_mgmt_tx_status == 0)
                lx_printk("iwl_mvm: AUTH retry con IN_CONTROL_AND_MGMT (sin TX resp)\n");
            else
                lx_printk("iwl_mvm: AUTH retry con IN_CONTROL_AND_MGMT\n");
            iwl->auth_ctl_filter = 1;
            if (iwl_mvm_mac_context_assoc(iwl, bssid, 0) != 0) {
                iwl->auth_ctl_filter = 0;
                lx_printk("iwl_mvm: MAC_CONTEXT ctl-filter falló\n");
                return -1;
            }
            iwl->auth_ctl_filter = 0;
        }
        iwl->last_mgmt_tx_status = 0;
        flen = build_auth_req(frame, iwl->mac, bssid);
        if (iwl_mvm_tx_mgmt(iwl, frame, flen) != 0) {
            lx_printk("iwl_mvm: AUTH TX falló\n");
            return -1;
        }
        (void)iwl_trans_wait_mgmt_tx_resp(iwl, 40);
        if (wait_mlme_flag(iwl, &iwl->mlme_auth_ok) == 0)
            goto auth_ok;
    }
    lx_printk("iwl_mvm: AUTH timeout\n");
    return -1;

auth_ok:

    iwl->last_mgmt_tx_status = 0;
    flen = build_assoc_req(iwl, frame, ssid, bssid);
    if (iwl_mvm_tx_mgmt(iwl, frame, flen) != 0) {
        lx_printk("iwl_mvm: ASSOC TX falló\n");
        return -1;
    }
    (void)iwl_trans_wait_mgmt_tx_resp(iwl, 40);
    if (wait_mlme_flag(iwl, &iwl->mlme_assoc_ok) != 0) {
        lx_printk("iwl_mvm: ASSOC timeout\n");
        return -1;
    }
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
    iwl->dtim_period = 0;
    iwl->sync_tsf = 0;
    iwl->sync_device_ts = 0;
    iwl->sync_dtim_count = 0;
    iwl->sync_beacon_seen = 0;
    iwl->assoc_pending_beacon = 0;
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
    /* IN_CONTROL_AND_MGMT desde el primer MAC_CONTEXT de assoc (Auth Response). */
    iwl->auth_ctl_filter = 1;
    if (iwl_mvm_mac_context_assoc(iwl, bssid, 0) != 0) {
        iwl->auth_ctl_filter = 0;
        lx_printk("iwl_mvm: MAC_CONTEXT falló\n");
        return -1;
    }
    if (iwl_mvm_add_sta_ap(iwl, bssid) != 0) {
        iwl->auth_ctl_filter = 0;
        lx_printk("iwl_mvm: ADD_STA falló\n");
        return -1;
    }
    (void)iwl_mvm_rate_init_ap_sta(iwl);
    if (iwl_trans_txq_alloc_mgmt(iwl, IWL_MVM_AP_STA_ID) < 0) {
        iwl->auth_ctl_filter = 0;
        lx_printk("iwl_mvm: TXQ mgmt falló\n");
        return -1;
    }
    iwl_trans_txq_drain_mgmt(iwl);
    if (iwl_mvm_protect_assoc(iwl) != 0) {
        iwl->auth_ctl_filter = 0;
        return -1;
    }
    if (iwl_mvm_mlme_auth_assoc(iwl, ssid, bssid) != 0) {
        iwl->auth_ctl_filter = 0;
        return -1;
    }
    iwl->auth_ctl_filter = 0;
    if (iwl_mvm_mac_context_assoc(iwl, bssid, 0) != 0) {
        lx_printk("iwl_mvm: MAC_CONTEXT post-ASSOC is_assoc=0 falló\n");
        return -1;
    }
    iwl->assoc_pending_beacon = 1;
    if (iwl_mvm_wait_assoc_beacon(iwl) != 0) {
        iwl->assoc_pending_beacon = 0;
        return -1;
    }
    iwl->assoc_pending_beacon = 0;
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

/* ADD_STA_KEY. `mcast` separa la clave de grupo de la de pares: sin ese bit el
 * firmware trata la GTK como una segunda PTK y el tráfico de difusión —ARP y
 * las respuestas DHCP de algunos AP— se queda sin descifrar.
 *
 * `key_idx` es el Key ID de 802.11 (0 en la PTK, 1 o 2 en la GTK, tal y como
 * viene en el KDE). `key_offset` es la ranura del firmware, que debe ser
 * distinta para cada clave. `rsc` es el contador de recepción desde el que el
 * receptor valida el PN; sin él una GTK renovada rechaza las primeras tramas.
 * Referencia: Linux mvm/sta.c `iwl_mvm_send_sta_key`. */
static int iwl_mvm_send_sta_key(struct iwl_ax211_priv *iwl, const uint8_t key[16],
                                int key_idx, int mcast, const uint8_t rsc[8])
{
    struct iwl_mvm_add_sta_key_cmd k;
    uint16_t key_flags;
    int rc;

    if (!iwl->associated || !key)
        return -1;
    if (key_idx < 0 || key_idx > 3)
        return -1;
    memset(&k, 0, sizeof(k));
    key_flags = (uint16_t)(((unsigned)key_idx << STA_KEY_FLG_KEYID_POS) &
                           STA_KEY_FLG_KEYID_MSK);
    key_flags |= STA_KEY_FLG_WEP_KEY_MAP;
    key_flags |= STA_KEY_FLG_CCM;
    if (mcast)
        key_flags |= STA_KEY_MULTICAST;
    /* sta_id 0 es el AP; el ternario `ap_sta_id ? …` lo trataba como ausente. */
    k.common.sta_id = iwl->ap_sta_id;
    /* Ranuras separadas: la de pares en la 0 y la de grupo en la 1. Compartir
     * offset dejaba la última instalación pisando a la anterior. */
    k.common.key_offset = mcast ? 1u : 0u;
    k.common.key_flags = iwl_cpu_to_le16(key_flags);
    memcpy(k.common.key, key, 16);
    if (rsc)
        memcpy(k.common.rx_secur_seq_cnt, rsc, 8);
    rc = iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, ADD_STA_KEY, &k,
                                 (uint16_t)sizeof(k), IWL_MVM_HCMD_TIMEOUT_MS);
    if (rc != 0) {
        lx_printk("iwl_mvm: ADD_STA_KEY %s idx=%d falló\n",
                  mcast ? "grupo" : "pares", key_idx);
        return rc;
    }
    lx_printk("iwl_mvm: clave %s instalada idx=%d offset=%u\n",
              mcast ? "grupo" : "pares", key_idx, (unsigned)k.common.key_offset);
    /* A partir de la primera clave el firmware cifra: `iwl_mvm_tx_8023` deja de
     * poner ENCRYPT_DIS y el camino RX exige tramas protegidas. */
    iwl->keys_installed = 1;
    return 0;
}

int iwl_mvm_install_key(struct iwl_ax211_priv *iwl, const uint8_t key[16], int key_idx)
{
    return iwl_mvm_send_sta_key(iwl, key, key_idx, 0, 0);
}

int iwl_mvm_install_gtk(struct iwl_ax211_priv *iwl, const uint8_t key[16], int key_idx,
                        const uint8_t rsc[8])
{
    return iwl_mvm_send_sta_key(iwl, key, key_idx, 1, rsc);
}
