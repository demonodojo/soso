/* Host: PHY (sin BINDING extra), MAC is_assoc=0, ADD_STA, TIME_EVENT, TX AUTH/ASSOC
 * y MAC is_assoc=1 con DTIM (Linux 6.6.32 phy-ctxt.c:285, mac80211.c:2453). */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

#define MAC_CONTEXT_CMD 0x28

struct cmd_rec {
    uint8_t group;
    uint8_t id;
    uint16_t len;
    uint8_t payload[256];
};

static struct cmd_rec g_sent[20];
static int g_sent_n;

void lx_printk(const char *fmt, ...) { (void)fmt; }
void lx_mdelay(unsigned int ms) { (void)ms; }

void iwl_trans_poll(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
}

static void inject_mlme_rx(struct iwl_ax211_priv *iwl, const void *payload,
                            uint16_t pay_len)
{
    unsigned off = iwl->gen3 ? (unsigned)sizeof(struct iwl_tx_cmd_gen3)
                               : (unsigned)sizeof(struct iwl_tx_cmd_gen2);
    const uint8_t *hdr;
    uint8_t rx[32];

    if (!payload || pay_len <= off)
        return;
    hdr = (const uint8_t *)payload + off;
    memset(rx, 0, sizeof(rx));
    memcpy(rx + 4, hdr + 10, 6);
    memcpy(rx + 10, hdr + 16, 6);
    memcpy(rx + 16, hdr + 16, 6);
    if (hdr[0] == (uint8_t)IEEE80211_STYPE_AUTH) {
        rx[0] = (uint8_t)IEEE80211_STYPE_AUTH;
        rx[26] = 2;
        iwl_mvm_rx_mlme_frame(iwl, rx, 30);
        return;
    }
    rx[0] = (uint8_t)IEEE80211_STYPE_ASSOC_RESP;
    rx[24] = (uint8_t)WLAN_CAPABILITY_ESS;
    rx[28] = 1;
    iwl->beacon_int = 100;
    iwl->dtim_period = 1;
    iwl_mvm_rx_mlme_frame(iwl, rx, 30);
}

int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t pay_len, int wait_ms)
{
    (void)wait_ms;
    if (g_sent_n < (int)(sizeof(g_sent) / sizeof(g_sent[0]))) {
        g_sent[g_sent_n].group = group;
        g_sent[g_sent_n].id = id;
        g_sent[g_sent_n].len = pay_len;
        if (payload && pay_len <= sizeof(g_sent[g_sent_n].payload))
            memcpy(g_sent[g_sent_n].payload, payload, pay_len);
        g_sent_n++;
    }
    if (id == TX_CMD)
        inject_mlme_rx(iwl, payload, pay_len);
    return 0;
}

int iwl_trans_send_cmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                       const void *payload, uint16_t pay_len)
{
    return iwl_trans_send_cmd_wait(iwl, group, id, payload, pay_len, 0);
}

int iwl_trans_send_cmd_async(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                             const void *payload, uint16_t pay_len)
{
    return iwl_trans_send_cmd_wait(iwl, group, id, payload, pay_len, 0);
}

int iwl_fw_cmd_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    (void)iwl;
    (void)group;
    if (cmd == ADD_STA)
        return 12;
    if (cmd == PHY_CONTEXT_CMD)
        return 4;
    if (cmd == TX_CMD)
        return 9;
    return 0;
}

int iwl_fw_has_capa(const struct iwl_ax211_priv *iwl, unsigned capa_bit)
{
    (void)iwl;
    (void)capa_bit;
    return 0;
}

uint8_t iwl_mvm_valid_tx_ant(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 3;
}

uint8_t iwl_mvm_valid_rx_ant(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 3;
}

int iwl_mvm_send_tx_ant_cfg(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 0;
}

int iwl_mvm_send_scan_cfg(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 0;
}

int iwl_mvm_init_mcc(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 0;
}

static int find_idx_n(uint8_t id, int which)
{
    int i;
    int n = 0;

    for (i = 0; i < g_sent_n; i++) {
        if (g_sent[i].id == id) {
            if (n == which)
                return i;
            n++;
        }
    }
    return -1;
}

static int find_idx(uint8_t id)
{
    return find_idx_n(id, 0);
}

static const struct cmd_rec *find_cmd(uint8_t id)
{
    int i = find_idx(id);
    return i < 0 ? NULL : &g_sent[i];
}

int main(void)
{
    struct iwl_ax211_priv iwl;
    static const uint8_t bssid[6] = {0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff};
    static const uint8_t ptk[16] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16};
    const struct cmd_rec *mac;
    const struct cmd_rec *add;
    const struct cmd_rec *te;
    const struct cmd_rec *key;
    const struct iwl_mac_ctx_cmd *mc;
    const struct iwl_mvm_add_sta_cmd *sta;
    const struct cmd_rec *phy;
    const struct iwl_phy_context_cmd *pc;
    const struct iwl_time_event_cmd *tec;
    const struct iwl_mvm_add_sta_key_cmd *sk;
    int idx_phy;
    int idx_bind;
    int idx_mac;
    int idx_add;
    int idx_te;
    int idx_key;

    memset(&iwl, 0, sizeof(iwl));
    memcpy(iwl.mac, (const uint8_t[]){0x84, 0x1b, 0x77, 0xe1, 0x20, 0x71}, 6);
    iwl.scan_mac_id = 0;
    iwl.alive = 1;
    iwl.mac_ctxt_added = 1;
    iwl.phy_ctxt_added = 1;
    iwl.binding_added = 1;
    iwl.phy_channel = 6;
    iwl.phy_band = PHY_BAND_24;
    iwl.channel = 40;

    if (sizeof(struct iwl_mvm_add_sta_cmd) != 48) {
        fprintf(stderr, "ADD_STA v10 debe ser 48 B, tiene %zu\n",
                sizeof(struct iwl_mvm_add_sta_cmd));
        return 1;
    }
    if (sizeof(struct iwl_mvm_add_sta_key_cmd) != 76) {
        fprintf(stderr, "ADD_STA_KEY v2 debe ser 76 B, tiene %zu\n",
                sizeof(struct iwl_mvm_add_sta_key_cmd));
        return 1;
    }
    if (sizeof(struct iwl_time_event_cmd) != 36) {
        fprintf(stderr, "TIME_EVENT_CMD debe ser 36 B, tiene %zu\n",
                sizeof(struct iwl_time_event_cmd));
        return 1;
    }

    g_sent_n = 0;
    if (iwl_mvm_assoc_prepare(&iwl, "Casa", bssid) != 0) {
        fprintf(stderr, "iwl_mvm_assoc_prepare falló\n");
        return 1;
    }

    phy = find_cmd(PHY_CONTEXT_CMD);
    mac = find_cmd(MAC_CONTEXT_CMD);
    add = find_cmd(ADD_STA);
    te = find_cmd(TIME_EVENT_CMD);
    idx_phy = find_idx(PHY_CONTEXT_CMD);
    idx_bind = find_idx(BINDING_CONTEXT_CMD);
    idx_mac = find_idx(MAC_CONTEXT_CMD);
    idx_add = find_idx(ADD_STA);
    idx_te = find_idx(TIME_EVENT_CMD);
    if (!phy || !mac || !add || !te) {
        fprintf(stderr, "faltan HCMD PHY=%d BIND=%d MAC=%d ADD=%d TE=%d\n",
                idx_phy, idx_bind, idx_mac, idx_add, idx_te);
        return 1;
    }
    if (idx_bind >= 0) {
        fprintf(stderr, "BINDING 0x2b no debe emitirse si PHY0↔MAC0 ya existe (idx=%d)\n",
                idx_bind);
        return 1;
    }
    if (!(idx_phy < idx_mac && idx_mac < idx_add && idx_add < idx_te)) {
        fprintf(stderr, "orden HCMD PHY(%d) MAC(%d) ADD(%d) TE(%d)\n",
                idx_phy, idx_mac, idx_add, idx_te);
        return 1;
    }
    if (phy->len != sizeof(struct iwl_phy_context_cmd)) {
        fprintf(stderr, "PHY_CONTEXT len=%u esperado %zu\n",
                phy->len, sizeof(struct iwl_phy_context_cmd));
        return 1;
    }
    pc = (const struct iwl_phy_context_cmd *)phy->payload;
    if (pc->action != iwl_cpu_to_le32(FW_CTXT_ACTION_MODIFY)) {
        fprintf(stderr, "PHY action=%u (esperado MODIFY)\n", pc->action);
        return 1;
    }
    if (pc->ci.channel != iwl_cpu_to_le32(40) || pc->ci.band != PHY_BAND_5) {
        fprintf(stderr, "PHY ch=%u band=%u (esperado ch40 5 GHz)\n",
                pc->ci.channel, pc->ci.band);
        return 1;
    }
    if (mac->len != sizeof(struct iwl_mac_ctx_cmd)) {
        fprintf(stderr, "MAC_CONTEXT len=%u esperado %zu\n",
                mac->len, sizeof(struct iwl_mac_ctx_cmd));
        return 1;
    }
    if (add->len != 48) {
        fprintf(stderr, "ADD_STA len=%u esperado 48\n", add->len);
        return 1;
    }
    if (te->group != LEGACY_GROUP || te->len != sizeof(struct iwl_time_event_cmd)) {
        fprintf(stderr, "TIME_EVENT group=%u len=%u\n", te->group, te->len);
        return 1;
    }
    tec = (const struct iwl_time_event_cmd *)te->payload;
    if (tec->id != iwl_cpu_to_le32(TE_BSS_STA_AGGRESSIVE_ASSOC) ||
        tec->action != iwl_cpu_to_le32(FW_CTXT_ACTION_ADD)) {
        fprintf(stderr, "TIME_EVENT id/action incorrectos\n");
        return 1;
    }

    mc = (const struct iwl_mac_ctx_cmd *)mac->payload;
    if (mc->id_and_color != FW_CMD_ID_AND_COLOR(0, 0)) {
        fprintf(stderr, "MAC id_and_color=0x%08x\n", mc->id_and_color);
        return 1;
    }
    if (mc->action != FW_CTXT_ACTION_MODIFY) {
        fprintf(stderr, "MAC action=%u (esperado MODIFY=2)\n", mc->action);
        return 1;
    }
    if (memcmp(mc->bssid_addr, bssid, 6) != 0) {
        fprintf(stderr, "MAC bssid distinto\n");
        return 1;
    }
    if (mc->u.sta.is_assoc != 0u) {
        fprintf(stderr, "MAC is_assoc=%u (esperado 0: sin DTIM, Linux mac-ctxt.c:680)\n",
                mc->u.sta.is_assoc);
        return 1;
    }
    if ((mc->filter_flags & IWL_MAC_FILTER_IN_BEACON) == 0) {
        fprintf(stderr, "MAC filter_flags=0x%x sin IN_BEACON\n", mc->filter_flags);
        return 1;
    }
    if ((mc->filter_flags & IWL_MAC_FILTER_ACCEPT_GRP) == 0) {
        fprintf(stderr, "MAC filter_flags=0x%x sin ACCEPT_GRP\n", mc->filter_flags);
        return 1;
    }

    sta = (const struct iwl_mvm_add_sta_cmd *)add->payload;
    if (sta->add_modify != 0) {
        fprintf(stderr, "ADD_STA add_modify=%u (esperado ADD=0)\n", sta->add_modify);
        return 1;
    }
    if (sta->sta_id != IWL_MVM_AP_STA_ID) {
        fprintf(stderr, "ADD_STA sta_id=%u\n", sta->sta_id);
        return 1;
    }
    if (sta->mac_id_n_color != iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(0, 0))) {
        fprintf(stderr, "ADD_STA mac_id_n_color=0x%08x\n", sta->mac_id_n_color);
        return 1;
    }
    if (memcmp(sta->addr, bssid, 6) != 0) {
        fprintf(stderr, "ADD_STA addr distinto\n");
        return 1;
    }
    if (sta->station_type != IWL_STA_LINK) {
        fprintf(stderr, "ADD_STA station_type=%u\n", sta->station_type);
        return 1;
    }
    if (sta->tid_disable_tx != iwl_cpu_to_le16(0xffff)) {
        fprintf(stderr, "ADD_STA tid_disable_tx=0x%04x\n", sta->tid_disable_tx);
        return 1;
    }
    if ((sta->station_flags & iwl_cpu_to_le32(STA_FLG_CLASS_AUTH | STA_FLG_CLASS_ASSOC)) !=
        iwl_cpu_to_le32(STA_FLG_CLASS_AUTH | STA_FLG_CLASS_ASSOC)) {
        fprintf(stderr, "ADD_STA station_flags=0x%08x sin CLASS_AUTH|ASSOC\n",
                sta->station_flags);
        return 1;
    }

    {
        unsigned tx_off = (unsigned)sizeof(struct iwl_tx_cmd_gen2);
        int idx_tx0 = find_idx_n(TX_CMD, 0);
        int idx_tx1 = find_idx_n(TX_CMD, 1);
        int idx_mac1 = find_idx_n(MAC_CONTEXT_CMD, 1);
        const struct cmd_rec *tx0;
        const struct cmd_rec *tx1;
        const struct cmd_rec *mac1;
        const struct iwl_mac_ctx_cmd *mc1;

        if (idx_tx0 < 0 || idx_tx1 < 0 || idx_mac1 < 0) {
            fprintf(stderr, "faltan TX AUTH=%d TX ASSOC=%d MAC1=%d\n",
                    idx_tx0, idx_tx1, idx_mac1);
            return 1;
        }
        if (!(idx_te < idx_tx0 && idx_tx0 < idx_tx1 && idx_tx1 < idx_mac1)) {
            fprintf(stderr, "orden TE(%d) TX(%d/%d) MAC1(%d)\n",
                    idx_te, idx_tx0, idx_tx1, idx_mac1);
            return 1;
        }
        tx0 = &g_sent[idx_tx0];
        tx1 = &g_sent[idx_tx1];
        mac1 = &g_sent[idx_mac1];
        if (tx0->group != DATA_PATH_GROUP || tx1->group != DATA_PATH_GROUP) {
            fprintf(stderr, "TX_CMD group=%u/%u (esperado DATA_PATH %u)\n",
                    tx0->group, tx1->group, (unsigned)DATA_PATH_GROUP);
            return 1;
        }
        if (tx_off != 20u) {
            fprintf(stderr, "TX_CMD gen2 prefix=%u (esperado 20)\n", tx_off);
            return 1;
        }
        if (tx0->len <= tx_off || tx0->payload[tx_off] != (uint8_t)IEEE80211_STYPE_AUTH) {
            fprintf(stderr, "TX0 no es AUTH fc=0x%02x\n",
                    tx0->len > tx_off ? tx0->payload[tx_off] : 0);
            return 1;
        }
        if (tx1->len <= tx_off || tx1->payload[tx_off] != (uint8_t)IEEE80211_STYPE_ASSOC_REQ) {
            fprintf(stderr, "TX1 no es ASSOC req fc=0x%02x\n",
                    tx1->len > tx_off ? tx1->payload[tx_off] : 0);
            return 1;
        }
        if (mac1->len != sizeof(struct iwl_mac_ctx_cmd)) {
            fprintf(stderr, "MAC1 len=%u\n", mac1->len);
            return 1;
        }
        mc1 = (const struct iwl_mac_ctx_cmd *)mac1->payload;
        if (mc1->u.sta.is_assoc != 1u) {
            fprintf(stderr, "MAC1 is_assoc=%u (esperado 1 tras AUTH+ASSOC)\n",
                    mc1->u.sta.is_assoc);
            return 1;
        }
        if (mc1->u.sta.dtim_interval == 0) {
            fprintf(stderr, "MAC1 dtim_interval=0 (Linux mac-ctxt.c:706)\n");
            return 1;
        }
        if (mc1->filter_flags & IWL_MAC_FILTER_IN_BEACON) {
            fprintf(stderr, "MAC1 filter sigue con IN_BEACON=0x%x\n", mc1->filter_flags);
            return 1;
        }
        if ((mc1->filter_flags & IWL_MAC_FILTER_ACCEPT_GRP) == 0) {
            fprintf(stderr, "MAC1 filter_flags=0x%x sin ACCEPT_GRP\n", mc1->filter_flags);
            return 1;
        }
    }
    if (!iwl.associated) {
        fprintf(stderr, "associated sigue en 0 tras MAC is_assoc=1\n");
        return 1;
    }

    if (iwl_mvm_install_key(&iwl, ptk, 0) != 0) {
        fprintf(stderr, "iwl_mvm_install_key falló\n");
        return 1;
    }
    key = find_cmd(ADD_STA_KEY);
    idx_key = find_idx(ADD_STA_KEY);
    if (!key || idx_key < 0) {
        fprintf(stderr, "falta ADD_STA_KEY\n");
        return 1;
    }
    if (key->group != LEGACY_GROUP || key->len != sizeof(struct iwl_mvm_add_sta_key_cmd)) {
        fprintf(stderr, "ADD_STA_KEY group=%u len=%u\n", key->group, key->len);
        return 1;
    }
    sk = (const struct iwl_mvm_add_sta_key_cmd *)key->payload;
    if (sk->common.sta_id != IWL_MVM_AP_STA_ID) {
        fprintf(stderr, "ADD_STA_KEY sta_id=%u\n", sk->common.sta_id);
        return 1;
    }
    if ((iwl_cpu_to_le16(sk->common.key_flags) & STA_KEY_FLG_CCM) == 0) {
        fprintf(stderr, "ADD_STA_KEY sin STA_KEY_FLG_CCM\n");
        return 1;
    }
    if (memcmp(sk->common.key, ptk, 16) != 0) {
        fprintf(stderr, "ADD_STA_KEY material distinto\n");
        return 1;
    }

    puts("OK: assoc PHY + MAC is_assoc=0 + ADD_STA + TE + AUTH/ASSOC + MAC is_assoc=1 + ADD_STA_KEY");
    return 0;
}
