/* Host: ADD_STA v10 (48 B) y MAC_CONTEXT MODIFY en assoc (Linux 6.6.32). */
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

static struct cmd_rec g_sent[8];
static int g_sent_n;

void lx_printk(const char *fmt, ...) { (void)fmt; }
void lx_mdelay(unsigned int ms) { (void)ms; }

int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t pay_len, int wait_ms)
{
    (void)iwl;
    (void)wait_ms;
    if (g_sent_n < (int)(sizeof(g_sent) / sizeof(g_sent[0]))) {
        g_sent[g_sent_n].group = group;
        g_sent[g_sent_n].id = id;
        g_sent[g_sent_n].len = pay_len;
        if (payload && pay_len <= sizeof(g_sent[g_sent_n].payload))
            memcpy(g_sent[g_sent_n].payload, payload, pay_len);
        g_sent_n++;
    }
    return 0;
}

int iwl_trans_send_cmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
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
    return 0;
}

static const struct cmd_rec *find_cmd(uint8_t id)
{
    int i;
    for (i = 0; i < g_sent_n; i++) {
        if (g_sent[i].id == id)
            return &g_sent[i];
    }
    return NULL;
}

int main(void)
{
    struct iwl_ax211_priv iwl;
    static const uint8_t bssid[6] = {0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff};
    const struct cmd_rec *mac;
    const struct cmd_rec *add;
    const struct iwl_mac_ctx_cmd *mc;
    const struct iwl_mvm_add_sta_cmd *sta;

    memset(&iwl, 0, sizeof(iwl));
    memcpy(iwl.mac, (const uint8_t[]){0x84, 0x1b, 0x77, 0xe1, 0x20, 0x71}, 6);
    iwl.scan_mac_id = 0;
    iwl.alive = 1;
    iwl.mac_ctxt_added = 1;

    if (sizeof(struct iwl_mvm_add_sta_cmd) != 48) {
        fprintf(stderr, "ADD_STA v10 debe ser 48 B, tiene %zu\n",
                sizeof(struct iwl_mvm_add_sta_cmd));
        return 1;
    }

    g_sent_n = 0;
    if (iwl_mvm_assoc_prepare(&iwl, "Casa", bssid) != 0) {
        fprintf(stderr, "iwl_mvm_assoc_prepare falló\n");
        return 1;
    }

    mac = find_cmd(MAC_CONTEXT_CMD);
    add = find_cmd(ADD_STA);
    if (!mac || !add) {
        fprintf(stderr, "faltan HCMD MAC_CONTEXT o ADD_STA\n");
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
    if (mc->u.sta.is_assoc != 1u) {
        fprintf(stderr, "MAC is_assoc=%u\n", mc->u.sta.is_assoc);
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

    puts("OK: assoc MAC MODIFY + ADD_STA v10 48 B (Linux 6.6.32)");
    return 0;
}
