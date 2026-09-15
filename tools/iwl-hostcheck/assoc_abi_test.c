/* Host: PHY (sin BINDING extra), MAC is_assoc=0, ADD_STA, SESSION_PROTECTION (AX200)
 * o TIME_EVENT (legacy), TX AUTH/ASSOC y MAC is_assoc=1 (Linux 6.6.32). */
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

struct tx_rec {
    uint16_t txq_id;
    uint16_t len;
    uint8_t payload[256];
};

static struct tx_rec g_tx[4];
static int g_tx_n;
static const uint16_t g_mock_mgmt_qid = 5u;
static size_t g_mgmt_bc_alloc_bytes;

void lx_printk(const char *fmt, ...) { (void)fmt; }
void lx_mdelay(unsigned int ms) { (void)ms; }

void *lx_dma_alloc_coherent(void *dev, size_t size, uint64_t *dma, unsigned gfp)
{
    (void)dev;
    (void)gfp;
    if (size >= IWL_SCD_BC_TBL_BYTES)
        g_mgmt_bc_alloc_bytes = size;
    void *p = aligned_alloc(IWL_SCD_DMA_ALIGN, (size + 4095u) & ~(size_t)4095u);
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

void iwl_trans_poll(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
}

void iwl_trans_txq_drain_mgmt(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
}

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
    if (id == TX_CMD) {
        fprintf(stderr, "TX_CMD no debe ir por cola HCMD (grp=%u id=0x%02x)\n",
                group, id);
        return -1;
    }
    if (group == DATA_PATH_GROUP && id == 0x01) {
        fprintf(stderr, "UPDATE_MU_GROUPS (5,0x01) no es TX\n");
        return -1;
    }
    if (group == LEGACY_GROUP && id == ADD_STA) {
        uint32_t status = ADD_STA_SUCCESS;

        iwl->cmd_resp_len = (uint16_t)sizeof(status);
        memcpy(iwl->cmd_resp, &status, sizeof(status));
    }
    if (group == LEGACY_GROUP && id == LQ_CMD) {
        iwl->cmd_resp_len = 0;
    }
    if (group == DATA_PATH_GROUP && id == TLC_MNG_CONFIG_CMD) {
        iwl->cmd_resp_len = 0;
    }
    if ((group == DATA_PATH_GROUP && id == SCD_QUEUE_CONFIG_CMD) ||
        (group == LEGACY_GROUP && id == SCD_QUEUE_CFG)) {
        struct iwl_tx_queue_cfg_rsp rsp;

        memset(&rsp, 0, sizeof(rsp));
        rsp.queue_number = g_mock_mgmt_qid;
        rsp.write_pointer = 0;
        iwl->cmd_resp_len = (uint16_t)sizeof(rsp);
        memcpy(iwl->cmd_resp, &rsp, sizeof(rsp));
    }
    return 0;
}

int iwl_trans_txq_alloc_mgmt(struct iwl_ax211_priv *iwl, uint8_t sta_id)
{
    unsigned tfd_bytes = IWL_MGMT_QUEUE_SIZE * IWL_TFH_TFD_SIZE;
    unsigned first_tb_bytes = IWL_MGMT_QUEUE_SIZE * IWL_FIRST_TB_SIZE_ALIGN;
    unsigned body_bytes = IWL_MGMT_QUEUE_SIZE * IWL_MGMT_TX_SLOT_SIZE;
    unsigned bc_bytes = IWL_SCD_BC_TBL_BYTES;
    int scd_ver;

    if (!iwl || !iwl->alive)
        return -1;
    if (iwl->mgmt_txq_ready)
        return (int)iwl->mgmt_txq_id;
    if (!iwl->mgmt_tfd_cpu) {
        iwl->invalid_tx_cmd_size = (uint16_t)sizeof(struct iwl_cmd_header_wide);
        iwl->invalid_tx_cmd_cpu = lx_dma_alloc_coherent(
            0, iwl->invalid_tx_cmd_size, &iwl->invalid_tx_cmd_dma, GFP_KERNEL);
        iwl->mgmt_tfd_cpu =
            lx_dma_alloc_coherent(0, tfd_bytes, &iwl->mgmt_tfd_dma, GFP_KERNEL);
        iwl->mgmt_first_tb_cpu = lx_dma_alloc_coherent(
            0, first_tb_bytes, &iwl->mgmt_first_tb_dma, GFP_KERNEL);
        iwl->mgmt_body_cpu =
            lx_dma_alloc_coherent(0, body_bytes, &iwl->mgmt_body_dma, GFP_KERNEL);
        iwl->mgmt_bc_cpu =
            lx_dma_alloc_coherent(0, bc_bytes, &iwl->mgmt_bc_dma, GFP_KERNEL);
        if (iwl->invalid_tx_cmd_cpu && iwl->mgmt_tfd_cpu) {
            unsigned i;

            iwl_invalid_tx_cmd_init(
                (struct iwl_cmd_header_wide *)iwl->invalid_tx_cmd_cpu);
            memset(iwl->mgmt_tfd_cpu, 0, tfd_bytes);
            for (i = 0; i < IWL_MGMT_QUEUE_SIZE; i++) {
                struct iwl_tfh_tfd_gen2 *tfd =
                    (struct iwl_tfh_tfd_gen2 *)((uint8_t *)iwl->mgmt_tfd_cpu +
                                                (size_t)i * IWL_TFH_TFD_SIZE);

                iwl_txq_set_tfd_invalid_gen2(tfd, iwl->invalid_tx_cmd_dma,
                                            iwl->invalid_tx_cmd_size);
            }
        }
    }
    if (!iwl->invalid_tx_cmd_cpu || !iwl->mgmt_tfd_cpu || !iwl->mgmt_first_tb_cpu ||
        !iwl->mgmt_body_cpu || !iwl->mgmt_bc_cpu)
        return -1;

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
        scd.u.add.bc_dram_addr = iwl->mgmt_bc_dma;
        scd.u.add.tfdq_dram_addr = iwl->mgmt_tfd_dma;
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
        cfg.byte_cnt_addr = iwl->mgmt_bc_dma;
        cfg.tfdq_addr = iwl->mgmt_tfd_dma;
        if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, SCD_QUEUE_CFG, &cfg,
                                    (uint16_t)sizeof(cfg),
                                    IWL_MVM_HCMD_TIMEOUT_MS) != 0)
            return -1;
    } else {
        return -1;
    }
    iwl->mgmt_txq_id = g_mock_mgmt_qid;
    iwl->mgmt_txq_write = 0;
    iwl->mgmt_txq_ready = 1;
    return (int)g_mock_mgmt_qid;
}

int iwl_trans_tx(struct iwl_ax211_priv *iwl, uint16_t txq_id,
                 const void *payload, uint16_t pay_len)
{
    uint8_t body[256];
    const struct iwl_cmd_header *hdr = (const struct iwl_cmd_header *)body;

    if (!iwl || !iwl->mgmt_txq_ready || txq_id != iwl->mgmt_txq_id)
        return -1;
    if (sizeof(struct iwl_cmd_header) != 4) {
        fprintf(stderr, "iwl_cmd_header debe ser 4 B\n");
        return -1;
    }
    if (TX_CMD != 0x1c) {
        fprintf(stderr, "TX_CMD debe ser 0x1c\n");
        return -1;
    }
    if (!payload || pay_len > sizeof(body) - sizeof(struct iwl_cmd_header))
        return -1;
    memset(body, 0, sizeof(body));
    ((struct iwl_cmd_header *)body)->cmd = TX_CMD;
    ((struct iwl_cmd_header *)body)->group_id = LEGACY_GROUP;
    ((struct iwl_cmd_header *)body)->sequence = 0;
    memcpy(body + sizeof(struct iwl_cmd_header), payload, pay_len);
    hdr = (const struct iwl_cmd_header *)body;
    if (hdr->cmd != 0x1c || hdr->group_id != 0) {
        fprintf(stderr, "TFD hdr cmd=0x%02x grp=%u (esperado 0x1c/0)\n",
                hdr->cmd, hdr->group_id);
        return -1;
    }
    if (g_tx_n < (int)(sizeof(g_tx) / sizeof(g_tx[0]))) {
        g_tx[g_tx_n].txq_id = txq_id;
        g_tx[g_tx_n].len = pay_len;
        if (payload && pay_len <= sizeof(g_tx[g_tx_n].payload))
            memcpy(g_tx[g_tx_n].payload, payload, pay_len);
        g_tx_n++;
    }
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
    if (group == DATA_PATH_GROUP && cmd == SCD_QUEUE_CONFIG_CMD)
        return 3;
    if (group == DATA_PATH_GROUP && cmd == RLC_CONFIG_CMD)
        return 3;
    if (group == DATA_PATH_GROUP && cmd == TLC_MNG_CONFIG_CMD)
        return 4;
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
    unsigned set = capa_bit / 32u;
    unsigned bit = capa_bit % 32u;

    if (!iwl || set >= IWL_FW_CAPA_SETS)
        return 0;
    return (iwl->fw_capa[set] & (1u << bit)) != 0;
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

static int find_idx_group_id(uint8_t group, uint8_t id)
{
    int i;

    for (i = 0; i < g_sent_n; i++) {
        if (g_sent[i].group == group && g_sent[i].id == id)
            return i;
    }
    return -1;
}

static const struct cmd_rec *find_cmd_group_id(uint8_t group, uint8_t id)
{
    int i = find_idx_group_id(group, id);
    return i < 0 ? NULL : &g_sent[i];
}

static void capa_set(struct iwl_ax211_priv *iwl, unsigned capa_bit)
{
    unsigned set = capa_bit / 32u;
    unsigned bit = capa_bit % 32u;

    if (iwl && set < IWL_FW_CAPA_SETS)
        iwl->fw_capa[set] |= (1u << bit);
}

static int run_assoc_case(int expect_tlc, int expect_lq)
{
    struct iwl_ax211_priv iwl;
    static const uint8_t bssid[6] = {0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff};
    static const uint8_t ptk[16] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16};
    const struct cmd_rec *mac;
    const struct cmd_rec *add;
    const struct cmd_rec *sess;
    const struct cmd_rec *key;
    const struct iwl_mac_ctx_cmd *mc;
    const struct iwl_mvm_add_sta_cmd *sta;
    const struct cmd_rec *phy;
    const struct iwl_phy_context_cmd *pc;
    const struct iwl_mvm_session_prot_cmd *spc;
    const struct iwl_mvm_add_sta_key_cmd *sk;
    int idx_phy;
    int idx_bind;
    int idx_mac;
    int idx_add;
    int idx_lq;
    int idx_tlc;
    int idx_sess;
    int idx_key;
    int idx_te_legacy;
    int idx_scd;
    int idx_rlc;
    const struct cmd_rec *scd;
    const struct cmd_rec *rlc;
    const struct iwl_scd_queue_cfg_cmd *sqc_v3;

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
    if (expect_tlc)
        capa_set(&iwl, IWL_UCODE_TLV_CAPA_TLC_OFFLOAD);
    capa_set(&iwl, IWL_UCODE_TLV_CAPA_SESSION_PROT_CMD);

    g_sent_n = 0;
    g_mgmt_bc_alloc_bytes = 0;
    g_tx_n = 0;
    if (iwl_mvm_assoc_prepare(&iwl, "Casa", bssid) != 0) {
        fprintf(stderr, "iwl_mvm_assoc_prepare falló\n");
        return 1;
    }

    phy = find_cmd(PHY_CONTEXT_CMD);
    mac = find_cmd(MAC_CONTEXT_CMD);
    add = find_cmd(ADD_STA);
    sess = find_cmd_group_id(MAC_CONF_GROUP, SESSION_PROTECTION_CMD);
    idx_phy = find_idx(PHY_CONTEXT_CMD);
    idx_bind = find_idx(BINDING_CONTEXT_CMD);
    idx_mac = find_idx(MAC_CONTEXT_CMD);
    idx_add = find_idx(ADD_STA);
    idx_lq = find_idx(LQ_CMD);
    idx_tlc = find_idx_group_id(DATA_PATH_GROUP, TLC_MNG_CONFIG_CMD);
    idx_sess = find_idx_group_id(MAC_CONF_GROUP, SESSION_PROTECTION_CMD);
    idx_te_legacy = find_idx(TIME_EVENT_CMD);
    idx_scd = find_idx_group_id(DATA_PATH_GROUP, SCD_QUEUE_CONFIG_CMD);
    idx_rlc = find_idx_group_id(DATA_PATH_GROUP, RLC_CONFIG_CMD);
    scd = find_cmd_group_id(DATA_PATH_GROUP, SCD_QUEUE_CONFIG_CMD);
    rlc = find_cmd_group_id(DATA_PATH_GROUP, RLC_CONFIG_CMD);
    if (!phy || !mac || !add || !scd || !sess) {
        fprintf(stderr,
                "faltan HCMD PHY=%d BIND=%d MAC=%d ADD=%d SCD=%d SESS=%d TE=%d\n",
                idx_phy, idx_bind, idx_mac, idx_add, idx_scd, idx_sess,
                idx_te_legacy);
        return 1;
    }
    if (find_idx_group_id(LEGACY_GROUP, SCD_QUEUE_CFG) >= 0) {
        fprintf(stderr, "SCD_QUEUE_CFG legacy 0x1d no debe emitirse (cc-a0-77 v3)\n");
        return 1;
    }
    if (idx_te_legacy >= 0) {
        fprintf(stderr, "TIME_EVENT 0x29 no debe emitirse con capa SESSION_PROT (idx=%d)\n",
                idx_te_legacy);
        return 1;
    }
    if (idx_bind >= 0) {
        fprintf(stderr, "BINDING 0x2b no debe emitirse si PHY0↔MAC0 ya existe (idx=%d)\n",
                idx_bind);
        return 1;
    }
    if (expect_tlc) {
        if (idx_lq >= 0) {
            fprintf(stderr, "LQ_CMD no debe emitirse con CAPA_TLC_OFFLOAD (idx=%d)\n",
                    idx_lq);
            return 1;
        }
        if (idx_tlc < 0) {
            fprintf(stderr, "falta TLC_MNG_CONFIG tras ADD_STA (TLC offload)\n");
            return 1;
        }
        if (!(idx_phy < idx_mac && idx_mac < idx_add && idx_add < idx_tlc &&
              idx_tlc < idx_scd && idx_scd < idx_sess)) {
            fprintf(stderr,
                    "orden HCMD PHY(%d) MAC(%d) ADD(%d) TLC(%d) SCD(%d) SESS(%d)\n",
                    idx_phy, idx_mac, idx_add, idx_tlc, idx_scd, idx_sess);
            return 1;
        }
    } else if (expect_lq) {
        if (idx_lq < 0) {
            fprintf(stderr, "falta LQ_CMD tras ADD_STA (sin TLC offload)\n");
            return 1;
        }
        if (idx_tlc >= 0) {
            fprintf(stderr, "TLC_MNG_CONFIG no debe emitirse sin TLC offload\n");
            return 1;
        }
        if (!(idx_phy < idx_mac && idx_mac < idx_add && idx_add < idx_lq &&
              idx_lq < idx_scd && idx_scd < idx_sess)) {
            fprintf(stderr,
                    "orden HCMD PHY(%d) MAC(%d) ADD(%d) LQ(%d) SCD(%d) SESS(%d)\n",
                    idx_phy, idx_mac, idx_add, idx_lq, idx_scd, idx_sess);
            return 1;
        }
        {
            const struct cmd_rec *lq = &g_sent[idx_lq];
            const struct iwl_lq_cmd {
                uint8_t sta_id;
                uint8_t reduced_tpc;
                uint16_t control;
                uint8_t flags;
                uint8_t mimo_delim;
                uint8_t single_stream_ant_msk;
                uint8_t dual_stream_ant_msk;
                uint8_t initial_rate_index[4];
                uint16_t agg_time_limit;
                uint8_t agg_disable_start_th;
                uint8_t agg_frame_cnt_limit;
                uint32_t reserved2;
                uint32_t rs_table[16];
                uint32_t ss_params;
            } *lqcmd;

            if (lq->group != LEGACY_GROUP || lq->len != 88u) {
                fprintf(stderr, "LQ_CMD group=%u len=%u (esperado LEGACY/88)\n",
                        lq->group, lq->len);
                return 1;
            }
            lqcmd = (const void *)lq->payload;
            if (lqcmd->sta_id != IWL_MVM_AP_STA_ID) {
                fprintf(stderr, "LQ_CMD sta_id=%u\n", lqcmd->sta_id);
                return 1;
            }
            if (lqcmd->rs_table[0] == 0) {
                fprintf(stderr, "LQ_CMD rs_table[0]=0\n");
                return 1;
            }
        }
    }
    if (scd->group != DATA_PATH_GROUP ||
        scd->len != sizeof(struct iwl_scd_queue_cfg_cmd)) {
        fprintf(stderr, "SCD_QUEUE_CONFIG group=%u len=%u\n", scd->group, scd->len);
        return 1;
    }
    sqc_v3 = (const struct iwl_scd_queue_cfg_cmd *)scd->payload;
    if (sqc_v3->operation != iwl_cpu_to_le32(IWL_SCD_QUEUE_ADD)) {
        fprintf(stderr, "SCD_QUEUE_CONFIG operation=0x%08x\n", sqc_v3->operation);
        return 1;
    }
    if (sqc_v3->u.add.sta_mask != iwl_cpu_to_le32(1u << IWL_MVM_AP_STA_ID)) {
        fprintf(stderr, "SCD_QUEUE_CONFIG sta_mask=0x%08x\n",
                sqc_v3->u.add.sta_mask);
        return 1;
    }
    if (sqc_v3->u.add.tid != IWL_MGMT_TID) {
        fprintf(stderr, "SCD_QUEUE_CONFIG tid=%u\n", sqc_v3->u.add.tid);
        return 1;
    }
    if (sqc_v3->u.add.flags != 0) {
        fprintf(stderr, "SCD_QUEUE_CONFIG flags=0x%08x (esperado 0)\n",
                sqc_v3->u.add.flags);
        return 1;
    }
    if (sqc_v3->u.add.cb_size !=
        iwl_cpu_to_le32(tfd_queue_cb_size(IWL_MGMT_QUEUE_SIZE))) {
        fprintf(stderr, "SCD_QUEUE_CONFIG cb_size=0x%08x\n", sqc_v3->u.add.cb_size);
        return 1;
    }
    if (sqc_v3->u.add.bc_dram_addr == 0 || sqc_v3->u.add.tfdq_dram_addr == 0) {
        fprintf(stderr, "SCD_QUEUE_CONFIG sin DMA (bc=0x%llx tfd=0x%llx)\n",
                (unsigned long long)sqc_v3->u.add.bc_dram_addr,
                (unsigned long long)sqc_v3->u.add.tfdq_dram_addr);
        return 1;
    }
    if (sqc_v3->u.add.bc_dram_addr != iwl.mgmt_bc_dma) {
        fprintf(stderr, "SCD_QUEUE_CONFIG bc_dram_addr distinto de mgmt_bc_dma\n");
        return 1;
    }
    if (g_mgmt_bc_alloc_bytes < IWL_SCD_BC_TBL_BYTES) {
        fprintf(stderr, "BC tbl alloc=%zu (esperado >= %u)\n",
                g_mgmt_bc_alloc_bytes, IWL_SCD_BC_TBL_BYTES);
        return 1;
    }
    if (!iwl.invalid_tx_cmd_cpu || iwl.invalid_tx_cmd_dma == 0 ||
        iwl.invalid_tx_cmd_size != (uint16_t)sizeof(struct iwl_cmd_header_wide)) {
        fprintf(stderr, "invalid_tx_cmd ausente (cpu=%p dma=0x%llx size=%u)\n",
                iwl.invalid_tx_cmd_cpu,
                (unsigned long long)iwl.invalid_tx_cmd_dma,
                (unsigned)iwl.invalid_tx_cmd_size);
        return 1;
    }
    {
        unsigned i;
        const struct iwl_cmd_header_wide *bad =
            (const struct iwl_cmd_header_wide *)iwl.invalid_tx_cmd_cpu;

        if (bad->cmd != INVALID_WR_PTR_CMD || bad->group_id != DEBUG_GROUP) {
            fprintf(stderr, "invalid_tx_cmd cmd=0x%02x grp=0x%02x\n",
                    bad->cmd, bad->group_id);
            return 1;
        }
        for (i = 0; i < IWL_MGMT_QUEUE_SIZE; i++) {
            const struct iwl_tfh_tfd_gen2 *tfd =
                (const struct iwl_tfh_tfd_gen2 *)((const uint8_t *)iwl.mgmt_tfd_cpu +
                                                   (size_t)i * IWL_TFH_TFD_SIZE);

            if (tfd->tbs[0].addr == 0 ||
                tfd->tbs[0].addr != iwl.invalid_tx_cmd_dma ||
                tfd->tbs[0].tb_len != iwl.invalid_tx_cmd_size ||
                tfd->num_tbs != 1) {
                fprintf(stderr,
                        "TFD[%u] TB0 addr=0x%llx len=%u n_tbs=%u (esperado invalid_tx_cmd)\n",
                        i, (unsigned long long)tfd->tbs[0].addr,
                        (unsigned)tfd->tbs[0].tb_len, (unsigned)tfd->num_tbs);
                return 1;
            }
        }
    }
    if (!iwl.mgmt_txq_ready || iwl.mgmt_txq_id != g_mock_mgmt_qid) {
        fprintf(stderr, "mgmt TXQ no lista (ready=%u qid=%u)\n",
                iwl.mgmt_txq_ready, iwl.mgmt_txq_id);
        return 1;
    }
    if (find_idx_n(TX_CMD, 0) >= 0) {
        fprintf(stderr, "TX_CMD no debe aparecer en cola HCMD\n");
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
    if (pc->rxchain_info != 0) {
        fprintf(stderr, "PHY rxchain_info=0x%08x (esperado 0 con RLC_CONFIG v3)\n",
                pc->rxchain_info);
        return 1;
    }
    if (!rlc || idx_rlc < 0) {
        fprintf(stderr, "falta RLC_CONFIG tras PHY (idx=%d)\n", idx_rlc);
        return 1;
    }
    if (rlc->group != DATA_PATH_GROUP ||
        rlc->len != sizeof(struct iwl_rlc_config_cmd)) {
        fprintf(stderr, "RLC_CONFIG group=%u len=%u\n", rlc->group, rlc->len);
        return 1;
    }
    if (!(idx_phy < idx_rlc && idx_rlc < idx_mac)) {
        fprintf(stderr, "orden PHY(%d) RLC(%d) MAC(%d)\n", idx_phy, idx_rlc, idx_mac);
        return 1;
    }
    {
        const struct iwl_rlc_config_cmd *rc =
            (const struct iwl_rlc_config_cmd *)rlc->payload;

        if (rc->phy_id != 0 || rc->rlc.rx_chain_info == 0) {
            fprintf(stderr, "RLC_CONFIG phy_id=0x%08x rx_chain=0x%08x\n",
                    rc->phy_id, rc->rlc.rx_chain_info);
            return 1;
        }
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
    if (sess->group != MAC_CONF_GROUP ||
        sess->len != sizeof(struct iwl_mvm_session_prot_cmd)) {
        fprintf(stderr, "SESSION_PROTECTION group=%u len=%u\n",
                sess->group, sess->len);
        return 1;
    }
    spc = (const struct iwl_mvm_session_prot_cmd *)sess->payload;
    if (spc->action != iwl_cpu_to_le32(FW_CTXT_ACTION_ADD) ||
        spc->conf_id != iwl_cpu_to_le32(SESSION_PROTECT_CONF_ASSOC)) {
        fprintf(stderr, "SESSION_PROTECTION action/conf_id incorrectos\n");
        return 1;
    }
    if (spc->duration_tu !=
        iwl_cpu_to_le32(MSEC_TO_TU(IWL_MVM_SESSION_PROTECTION_ASSOC_MS))) {
        fprintf(stderr, "SESSION_PROTECTION duration_tu=0x%08x (esperado %u)\n",
                spc->duration_tu,
                (unsigned)MSEC_TO_TU(IWL_MVM_SESSION_PROTECTION_ASSOC_MS));
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
    if ((mc->filter_flags & IWL_MAC_FILTER_IN_CONTROL_AND_MGMT) == 0) {
        fprintf(stderr, "MAC filter_flags=0x%x sin IN_CONTROL_AND_MGMT\n",
                mc->filter_flags);
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
    if ((sta->station_flags & iwl_cpu_to_le32(STA_FLG_CLASS_AUTH | STA_FLG_CLASS_ASSOC)) != 0) {
        fprintf(stderr, "ADD_STA station_flags=0x%08x con CLASS_AUTH|ASSOC (Linux no los pone)\n",
                sta->station_flags);
        return 1;
    }
    if ((sta->station_flags_msk & iwl_cpu_to_le32(STA_FLG_CLASS_AUTH | STA_FLG_CLASS_ASSOC)) != 0) {
        fprintf(stderr, "ADD_STA station_flags_msk=0x%08x con CLASS_AUTH|ASSOC\n",
                sta->station_flags_msk);
        return 1;
    }
    if ((sta->station_flags & iwl_cpu_to_le32(STA_FLG_FAT_EN_MSK)) !=
        iwl_cpu_to_le32(STA_FLG_FAT_EN_20MHZ)) {
        fprintf(stderr, "ADD_STA station_flags=0x%08x FAT no es 20 MHz\n",
                sta->station_flags);
        return 1;
    }
    if ((sta->station_flags_msk & iwl_cpu_to_le32(STA_FLG_FAT_EN_MSK | STA_FLG_MIMO_EN_MSK)) !=
        iwl_cpu_to_le32(STA_FLG_FAT_EN_MSK | STA_FLG_MIMO_EN_MSK)) {
        fprintf(stderr, "ADD_STA station_flags_msk=0x%08x sin FAT|MIMO\n",
                sta->station_flags_msk);
        return 1;
    }

    {
        unsigned tx_off = (unsigned)sizeof(struct iwl_tx_cmd_gen2);
        int idx_mac1 = find_idx_n(MAC_CONTEXT_CMD, 1);
        const struct tx_rec *tx0;
        const struct tx_rec *tx1;
        const struct cmd_rec *mac1;
        const struct iwl_mac_ctx_cmd *mc1;

        if (g_tx_n < 2 || idx_mac1 < 0) {
            fprintf(stderr, "faltan TX data AUTH/ASSOC=%d MAC1=%d\n",
                    g_tx_n, idx_mac1);
            return 1;
        }
        if (!(idx_sess < idx_mac1)) {
            fprintf(stderr, "orden SESS(%d) MAC1(%d)\n", idx_sess, idx_mac1);
            return 1;
        }
        tx0 = &g_tx[0];
        tx1 = &g_tx[1];
        mac1 = &g_sent[idx_mac1];
        if (tx0->txq_id == 0 || tx1->txq_id == 0) {
            fprintf(stderr, "TX mgmt qid=%u/%u (esperado != 0)\n",
                    tx0->txq_id, tx1->txq_id);
            return 1;
        }
        if (tx0->txq_id != g_mock_mgmt_qid || tx1->txq_id != g_mock_mgmt_qid) {
            fprintf(stderr, "TX mgmt qid=%u/%u (esperado %u)\n",
                    tx0->txq_id, tx1->txq_id, g_mock_mgmt_qid);
            return 1;
        }
        if (sizeof(struct iwl_cmd_header) != 4u) {
            fprintf(stderr, "iwl_cmd_header=%zu (esperado 4)\n",
                    sizeof(struct iwl_cmd_header));
            return 1;
        }
        if (TX_CMD != 0x1c) {
            fprintf(stderr, "TX_CMD=0x%02x (esperado 0x1c)\n", TX_CMD);
            return 1;
        }
        if (tx_off != 20u) {
            fprintf(stderr, "TX_CMD gen2 prefix=%u (esperado 20)\n", tx_off);
            return 1;
        }
        if ((unsigned)sizeof(struct iwl_cmd_header) + tx_off != 24u) {
            fprintf(stderr, "AUTH en TFD offset=%u (esperado 24)\n",
                    (unsigned)sizeof(struct iwl_cmd_header) + tx_off);
            return 1;
        }
        if (tx0->len <= tx_off || tx0->payload[tx_off] != (uint8_t)IEEE80211_STYPE_AUTH) {
            fprintf(stderr, "TX0 no es AUTH fc=0x%02x\n",
                    tx0->len > tx_off ? tx0->payload[tx_off] : 0);
            return 1;
        }
        {
            const struct iwl_tx_cmd_gen2 *txcmd =
                (const struct iwl_tx_cmd_gen2 *)tx0->payload;
            uint32_t tx_flags = txcmd->flags;

            if ((tx_flags & IWL_TX_FLAGS_CMD_RATE) == 0) {
                fprintf(stderr, "TX0 flags=0x%08x sin CMD_RATE (sin LQ/rate scale)\n",
                        tx_flags);
                return 1;
            }
            if ((tx_flags & IWL_TX_FLAGS_ENCRYPT_DIS) == 0 ||
                (tx_flags & IWL_TX_FLAGS_HIGH_PRI) == 0) {
                fprintf(stderr, "TX0 flags=0x%08x sin ENCRYPT_DIS|HIGH_PRI\n", tx_flags);
                return 1;
            }
            if (txcmd->offload_assist != (uint16_t)((24u / 2u) << TX_CMD_OFFLD_MH_SIZE)) {
                fprintf(stderr, "TX0 offload_assist=0x%04x (esperado 0x0c00 MH_SIZE)\n",
                        txcmd->offload_assist);
                return 1;
            }
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
    key = find_cmd_group_id(LEGACY_GROUP, ADD_STA_KEY);
    idx_key = find_idx_group_id(LEGACY_GROUP, ADD_STA_KEY);
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

    return 0;
}

int main(void)
{
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
    if (sizeof(struct iwl_mvm_session_prot_cmd) != 24) {
        fprintf(stderr, "SESSION_PROTECTION_CMD debe ser 24 B, tiene %zu\n",
                sizeof(struct iwl_mvm_session_prot_cmd));
        return 1;
    }
    if (sizeof(struct iwl_tx_queue_cfg_cmd) != 24) {
        fprintf(stderr, "SCD_QUEUE_CFG legacy debe ser 24 B, tiene %zu\n",
                sizeof(struct iwl_tx_queue_cfg_cmd));
        return 1;
    }
    if (sizeof(struct iwl_scd_queue_cfg_cmd) != 36) {
        fprintf(stderr, "SCD_QUEUE_CONFIG_CMD payload debe ser 36 B, tiene %zu\n",
                sizeof(struct iwl_scd_queue_cfg_cmd));
        return 1;
    }
    if (IWL_SCD_BC_TBL_BYTES != 640) {
        fprintf(stderr, "IWL_SCD_BC_TBL_BYTES=%u (esperado 640)\n",
                IWL_SCD_BC_TBL_BYTES);
        return 1;
    }

    if (run_assoc_case(1, 0) != 0)
        return 1;
    puts("OK: assoc AX200 TLC offload — sin LQ_CMD, TLC_MNG_CONFIG + SCD v3 + SESSION_PROT");

    if (run_assoc_case(0, 1) != 0)
        return 1;
    puts("OK: assoc legacy — LQ_CMD + SCD v3 + SESSION_PROT");
    return 0;
}
