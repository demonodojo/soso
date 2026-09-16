/* Host: AX200 cc-a0-77 declara BINDING_CDB (39) sin CDB (40).
 * Assoc ch6→ch40: BINDING REMOVE + PHY REMOVE/ADD con lmac_id=0 (Linux binding.c:168). */
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

static struct cmd_rec g_sent[32];
static int g_sent_n;

void lx_printk(const char *fmt, ...) { (void)fmt; }
void lx_mdelay(unsigned int ms) { (void)ms; }

void *lx_kmalloc(unsigned long size, unsigned gfp)
{
    (void)gfp;
    return malloc(size);
}

void lx_kfree(const void *p) { free((void *)p); }

void *lx_dma_alloc_coherent(void *dev, size_t size, uint64_t *dma, unsigned gfp)
{
    (void)dev;
    (void)gfp;
    void *p = aligned_alloc(4096, (size + 4095u) & ~(size_t)4095u);
    if (p && dma)
        *dma = (uint64_t)(uintptr_t)p;
    return p;
}

void lx_dma_free_coherent(void *dev, size_t size, void *cpu, uint64_t dma)
{
    (void)dev;
    (void)size;
    (void)dma;
    free(cpu);
}

static void inject_beacon(struct iwl_ax211_priv *iwl)
{
    uint8_t bc[64];
    int pos = 24 + 12;

    memset(bc, 0, sizeof(bc));
    bc[0] = (uint8_t)IEEE80211_STYPE_BEACON;
    memcpy(bc + 16, iwl->bssid, 6);
    bc[32] = 100;
    bc[33] = 0;
    bc[pos] = WLAN_EID_TIM;
    bc[pos + 1] = 4;
    bc[pos + 2] = 0;
    bc[pos + 3] = 1;
    iwl->sync_tsf = 1000000;
    iwl->sync_device_ts = 500000;
    iwl->sync_beacon_seen = 1;
    iwl_mvm_rx_mlme_frame(iwl, bc, pos + 6);
}

void iwl_trans_poll(struct iwl_ax211_priv *iwl)
{
    if (iwl && iwl->assoc_pending_beacon && !iwl->sync_beacon_seen)
        inject_beacon(iwl);
}

void iwl_trans_txq_drain_mgmt(struct iwl_ax211_priv *iwl) { (void)iwl; }

int iwl_trans_wait_mgmt_tx_resp(struct iwl_ax211_priv *iwl, unsigned iters)
{
    (void)iters;
    return iwl->last_mgmt_tx_status != 0 ? 0 : -1;
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
        return -1;
    if (group == LEGACY_GROUP && id == ADD_STA) {
        uint32_t status = ADD_STA_SUCCESS;

        iwl->cmd_resp_len = (uint16_t)sizeof(status);
        memcpy(iwl->cmd_resp, &status, sizeof(status));
    }
    if ((group == DATA_PATH_GROUP && id == SCD_QUEUE_CONFIG_CMD) ||
        (group == LEGACY_GROUP && id == SCD_QUEUE_CFG)) {
        struct iwl_tx_queue_cfg_rsp rsp;

        memset(&rsp, 0, sizeof(rsp));
        rsp.queue_number = 5;
        rsp.write_pointer = 0;
        iwl->cmd_resp_len = (uint16_t)sizeof(rsp);
        memcpy(iwl->cmd_resp, &rsp, sizeof(rsp));
    }
    return 0;
}

int iwl_trans_txq_alloc_mgmt(struct iwl_ax211_priv *iwl, uint8_t sta_id)
{
    int scd_ver;

    if (!iwl || !iwl->alive)
        return -1;
    if (iwl->mgmt_txq_ready)
        return (int)iwl->mgmt_txq_id;

    scd_ver = iwl_fw_cmd_ver(iwl, DATA_PATH_GROUP, SCD_QUEUE_CONFIG_CMD);
    if (scd_ver == 3) {
        struct iwl_scd_queue_cfg_cmd scd;

        memset(&scd, 0, sizeof(scd));
        scd.operation = iwl_cpu_to_le32(IWL_SCD_QUEUE_ADD);
        scd.u.add.sta_mask = iwl_cpu_to_le32(1u << sta_id);
        scd.u.add.tid = IWL_MGMT_TID;
        scd.u.add.flags = 0;
        scd.u.add.cb_size =
            iwl_cpu_to_le32(tfd_queue_cb_size(IWL_MGMT_QUEUE_SIZE));
        if (iwl_trans_send_cmd_wait(iwl, DATA_PATH_GROUP, SCD_QUEUE_CONFIG_CMD,
                                    &scd, (uint16_t)sizeof(scd),
                                    IWL_MVM_HCMD_TIMEOUT_MS) != 0)
            return -1;
    } else if (scd_ver == 0) {
        struct iwl_tx_queue_cfg_cmd cfg;

        memset(&cfg, 0, sizeof(cfg));
        cfg.sta_id = sta_id;
        cfg.tid = IWL_MGMT_TID;
        cfg.flags = iwl_cpu_to_le16(TX_QUEUE_CFG_ENABLE_QUEUE);
        cfg.cb_size = iwl_cpu_to_le32(tfd_queue_cb_size(IWL_MGMT_QUEUE_SIZE));
        if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, SCD_QUEUE_CFG, &cfg,
                                    (uint16_t)sizeof(cfg),
                                    IWL_MVM_HCMD_TIMEOUT_MS) != 0)
            return -1;
    } else {
        return -1;
    }
    iwl->mgmt_txq_id = 5;
    iwl->mgmt_txq_write = 0;
    iwl->mgmt_txq_ready = 1;
    return 5;
}

int iwl_trans_txq_alloc_data(struct iwl_ax211_priv *iwl, uint8_t sta_id, uint8_t tid)
{
    (void)sta_id;
    if (!iwl || !iwl->alive)
        return -1;
    if (tid != IWL_TID_NON_QOS)
        return -1;
    iwl->data_txq_id = 6;
    iwl->data_txq_write = 0;
    iwl->data_txq_read = 0;
    iwl->data_txq_ready = 1;
    return 6;
}

void iwl_trans_txq_drain_data(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
}

int iwl_trans_tx(struct iwl_ax211_priv *iwl, uint16_t txq_id,
                 const void *payload, uint16_t pay_len)
{
    (void)txq_id;
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

#include "iwl_fw_body.inc"

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

static int phy_action_at(int idx)
{
    const struct iwl_phy_context_cmd *pc;

    if (idx < 0 || g_sent[idx].id != PHY_CONTEXT_CMD)
        return -1;
    if (g_sent[idx].len != sizeof(struct iwl_phy_context_cmd))
        return -1;
    pc = (const struct iwl_phy_context_cmd *)g_sent[idx].payload;
    return (int)pc->action;
}

static uint32_t phy_lmac_at(int idx)
{
    const struct iwl_phy_context_cmd *pc;

    pc = (const struct iwl_phy_context_cmd *)g_sent[idx].payload;
    return pc->lmac_id;
}

static uint32_t bind_lmac_at(int idx)
{
    const struct iwl_binding_cmd *bc;

    if (g_sent[idx].len < (uint16_t)sizeof(struct iwl_binding_cmd))
        return 0xffffffffu;
    bc = (const struct iwl_binding_cmd *)g_sent[idx].payload;
    return bc->lmac_id;
}

static uint32_t bind_action_at(int idx)
{
    const struct iwl_binding_cmd_v1 *bc;

    bc = (const struct iwl_binding_cmd_v1 *)g_sent[idx].payload;
    return bc->action;
}

static int check_capa_bits(const struct iwl_ax211_priv *parsed)
{
    if (!iwl_fw_has_capa(parsed, IWL_UCODE_TLV_CAPA_BINDING_CDB_SUPPORT)) {
        fprintf(stderr, "ucode sin BINDING_CDB (39)\n");
        return -1;
    }
    if (iwl_fw_has_capa(parsed, IWL_UCODE_TLV_CAPA_CDB_SUPPORT)) {
        fprintf(stderr, "ucode declara CDB (40); test solo para AX200 sin CDB\n");
        return -1;
    }
    return 0;
}

static int run_assoc_cdb_lmac(const struct iwl_ax211_priv *parsed)
{
    struct iwl_ax211_priv iwl;
    static const uint8_t bssid[6] = {0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff};
    int idx_bind0;
    int idx_phy0;
    int idx_phy1;
    int idx_bind1;
    const struct iwl_phy_context_cmd *pc;

    memset(&iwl, 0, sizeof(iwl));
    memcpy(iwl.mac, (const uint8_t[]){0x84, 0x1b, 0x77, 0xe1, 0x20, 0x71}, 6);
    memcpy(iwl.fw_capa, parsed->fw_capa, sizeof(iwl.fw_capa));
    iwl.cmd_ver_count = parsed->cmd_ver_count;
    memcpy(iwl.cmd_ver, parsed->cmd_ver, sizeof(iwl.cmd_ver));
    iwl.scan_mac_id = 0;
    iwl.alive = 1;
    iwl.mac_ctxt_added = 1;
    iwl.phy_ctxt_added = 1;
    iwl.binding_added = 1;
    iwl.phy_channel = 6;
    iwl.phy_band = PHY_BAND_24;
    iwl.channel = 40;

    g_sent_n = 0;
    if (iwl_mvm_assoc_prepare(&iwl, "Casa", bssid) != 0) {
        fprintf(stderr, "iwl_mvm_assoc_prepare falló\n");
        return -1;
    }

    idx_bind0 = find_idx_n(BINDING_CONTEXT_CMD, 0);
    idx_phy0 = find_idx_n(PHY_CONTEXT_CMD, 0);
    idx_phy1 = find_idx_n(PHY_CONTEXT_CMD, 1);
    idx_bind1 = find_idx_n(BINDING_CONTEXT_CMD, 1);

    if (idx_bind0 < 0 || idx_phy0 < 0 || idx_phy1 < 0 || idx_bind1 < 0) {
        fprintf(stderr, "HCMD incompleto bind0=%d phy0=%d phy1=%d bind1=%d\n",
                idx_bind0, idx_phy0, idx_phy1, idx_bind1);
        return -1;
    }
    if (!(idx_bind0 < idx_phy0 && idx_phy0 < idx_phy1 && idx_phy1 < idx_bind1)) {
        fprintf(stderr, "orden bind0(%d) phy_rm(%d) phy_add(%d) bind1(%d)\n",
                idx_bind0, idx_phy0, idx_phy1, idx_bind1);
        return -1;
    }
    if (bind_action_at(idx_bind0) != iwl_cpu_to_le32(FW_CTXT_ACTION_REMOVE)) {
        fprintf(stderr, "BINDING[0] action=%u (esperado REMOVE=3)\n",
                bind_action_at(idx_bind0));
        return -1;
    }
    if (phy_action_at(idx_phy0) != (int)FW_CTXT_ACTION_REMOVE) {
        fprintf(stderr, "PHY[0] action=%d (esperado REMOVE=3)\n", phy_action_at(idx_phy0));
        return -1;
    }
    if (phy_action_at(idx_phy1) != (int)FW_CTXT_ACTION_ADD) {
        fprintf(stderr, "PHY[1] action=%d (esperado ADD=1)\n", phy_action_at(idx_phy1));
        return -1;
    }
    if (bind_action_at(idx_bind1) != iwl_cpu_to_le32(FW_CTXT_ACTION_ADD)) {
        fprintf(stderr, "BINDING[1] action=%u (esperado ADD=1)\n",
                bind_action_at(idx_bind1));
        return -1;
    }

    pc = (const struct iwl_phy_context_cmd *)g_sent[idx_phy0].payload;
    if (pc->ci.channel != iwl_cpu_to_le32(40) || pc->ci.band != PHY_BAND_5) {
        fprintf(stderr, "PHY REMOVE ch=%u band=%u (esperado ch40 band0)\n",
                pc->ci.channel, pc->ci.band);
        return -1;
    }
    if (phy_lmac_at(idx_phy0) != iwl_cpu_to_le32(IWL_LMAC_24G_INDEX)) {
        fprintf(stderr, "PHY REMOVE lmac_id=0x%08x (esperaba LMAC 0)\n",
                phy_lmac_at(idx_phy0));
        return -1;
    }
    if (phy_lmac_at(idx_phy1) != iwl_cpu_to_le32(IWL_LMAC_24G_INDEX)) {
        fprintf(stderr, "PHY ADD lmac_id=0x%08x (esperaba LMAC 0)\n",
                phy_lmac_at(idx_phy1));
        return -1;
    }
    if (bind_lmac_at(idx_bind1) != iwl_cpu_to_le32(IWL_LMAC_24G_INDEX)) {
        fprintf(stderr, "BINDING ADD lmac_id=0x%08x (esperaba LMAC 0)\n",
                bind_lmac_at(idx_bind1));
        return -1;
    }

    puts("OK: assoc ch6→ch40 BINDING/PHY REMOVE+ADD con lmac_id=0 (sin CDB 40)");
    return 0;
}

int main(int argc, char **argv)
{
    struct iwl_ax211_priv iwl;
    FILE *f;
    long sz;
    uint8_t *fw;

    if (argc != 2) {
        fprintf(stderr, "uso: %s firmware.ucode\n", argv[0]);
        return 1;
    }
    f = fopen(argv[1], "rb");
    if (!f) {
        perror(argv[1]);
        return 1;
    }
    fseek(f, 0, SEEK_END);
    sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    fw = malloc((size_t)sz);
    if (!fw || fread(fw, 1, (size_t)sz, f) != (size_t)sz) {
        fprintf(stderr, "lectura falló\n");
        return 1;
    }
    fclose(f);

    memset(&iwl, 0, sizeof(iwl));
    if (iwl_fw_parse_tlv(&iwl, fw, (unsigned long)sz) != 0) {
        fprintf(stderr, "parse TLV falló\n");
        free(fw);
        return 1;
    }
    free(fw);

    if (check_capa_bits(&iwl) != 0)
        return 1;

    return run_assoc_cdb_lmac(&iwl) != 0 ? 1 : 0;
}
