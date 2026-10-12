/* Link quality (LQ_CMD) y TLC offload — Linux mvm/utils.c:253, rs-fw.c:577. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memset(void *dst, int c, unsigned long n);

#define LQ_MAX_RETRY_NUM               16u
#define IWL_LQ_AC_NUM                  4u
#define LINK_QUAL_AGG_FRAME_LIMIT_GEN2_DEF 255u
#define IWL_MVM_RS_AGG_TIME_LIMIT      4000u
#define IWL_MVM_RS_AGG_DISABLE_START   3u

#define IWL_TLC_MNG_CH_WIDTH_20MHZ     0
#define IWL_TLC_MNG_MODE_CCK             0
#define IWL_TLC_MNG_CHAIN_A_MSK          (1u << 0)
#define IWL_TLC_MNG_CHAIN_B_MSK          (1u << 1)
#define IWL_TLC_NSS_MAX                  2
#define IWL_TLC_MCS_PER_BW_NUM_V4        3

struct iwl_lq_cmd {
    uint8_t sta_id;
    uint8_t reduced_tpc;
    uint16_t control;
    uint8_t flags;
    uint8_t mimo_delim;
    uint8_t single_stream_ant_msk;
    uint8_t dual_stream_ant_msk;
    uint8_t initial_rate_index[IWL_LQ_AC_NUM];
    uint16_t agg_time_limit;
    uint8_t agg_disable_start_th;
    uint8_t agg_frame_cnt_limit;
    uint32_t reserved2;
    uint32_t rs_table[LQ_MAX_RETRY_NUM];
    uint32_t ss_params;
} __attribute__((packed));

struct iwl_tlc_config_cmd_v4 {
    uint8_t sta_id;
    uint8_t reserved1[3];
    uint8_t max_ch_width;
    uint8_t mode;
    uint8_t chains;
    uint8_t sgi_ch_width_supp;
    uint16_t flags;
    uint16_t non_ht_rates;
    uint16_t ht_rates[IWL_TLC_NSS_MAX][IWL_TLC_MCS_PER_BW_NUM_V4];
    uint16_t max_mpdu_len;
    uint16_t max_tx_op;
} __attribute__((packed));

static uint32_t lq_legacy_6m_rate(struct iwl_ax211_priv *iwl)
{
    uint32_t ant = RATE_MCS_ANT_A_MSK;
    int ver = iwl_fw_cmd_ver(iwl, LEGACY_GROUP, TX_CMD);

    if (ver > 8)
        return RATE_MCS_LEGACY_OFDM_MSK | RATE_LEGACY_OFDM_6M | ant;
    return RATE_LEGACY_PLCP_6M | ant;
}

static void iwl_mvm_fill_lq_ap(struct iwl_ax211_priv *iwl, struct iwl_lq_cmd *lq)
{
    uint32_t rate = lq_legacy_6m_rate(iwl);
    uint8_t ant = iwl->valid_tx_ant ? iwl->valid_tx_ant : 3u;
    unsigned i;

    memset(lq, 0, sizeof(*lq));
    lq->sta_id = iwl->ap_sta_id;
    lq->single_stream_ant_msk = ant & 3u;
    lq->agg_disable_start_th = IWL_MVM_RS_AGG_DISABLE_START;
    lq->agg_time_limit = iwl_cpu_to_le16((uint16_t)IWL_MVM_RS_AGG_TIME_LIMIT);
    lq->agg_frame_cnt_limit = LINK_QUAL_AGG_FRAME_LIMIT_GEN2_DEF;
    for (i = 0; i < LQ_MAX_RETRY_NUM; i++)
        lq->rs_table[i] = iwl_cpu_to_le32(rate);
}

int iwl_mvm_send_lq_cmd(struct iwl_ax211_priv *iwl, struct iwl_lq_cmd *lq)
{
    if (!iwl || !lq)
        return -1;
    if (iwl_fw_has_capa(iwl, IWL_UCODE_TLV_CAPA_TLC_OFFLOAD))
        return -1;
    /* Linux mvm/utils.c:253 — CMD_ASYNC; el FW no ACK LQ en cc-a0-77 si se espera. */
    return iwl_trans_send_cmd_async(iwl, LEGACY_GROUP, LQ_CMD, lq,
                                    (uint16_t)sizeof(*lq));
}

static int iwl_mvm_send_tlc_config_ap(struct iwl_ax211_priv *iwl)
{
    struct iwl_tlc_config_cmd_v4 cfg;
    uint8_t ant = iwl->valid_tx_ant ? iwl->valid_tx_ant : 3u;
    int ver;

    if (!iwl)
        return -1;
    ver = iwl_fw_cmd_ver(iwl, DATA_PATH_GROUP, TLC_MNG_CONFIG_CMD);
    if (ver != 4)
        return -1;

    memset(&cfg, 0, sizeof(cfg));
    cfg.sta_id = iwl->ap_sta_id;
    cfg.max_ch_width = IWL_TLC_MNG_CH_WIDTH_20MHZ;
    cfg.mode = IWL_TLC_MNG_MODE_CCK;
    if (ant & 1u)
        cfg.chains |= IWL_TLC_MNG_CHAIN_A_MSK;
    if (ant & 2u)
        cfg.chains |= IWL_TLC_MNG_CHAIN_B_MSK;
    cfg.non_ht_rates = iwl_cpu_to_le16(0x15fu);
    return iwl_trans_send_cmd_async(iwl, DATA_PATH_GROUP, TLC_MNG_CONFIG_CMD,
                                    &cfg, (uint16_t)sizeof(cfg));
}

int iwl_mvm_rate_init_ap_sta(struct iwl_ax211_priv *iwl)
{
    struct iwl_lq_cmd lq;
    int ret;

    if (!iwl)
        return -1;
    if (iwl_fw_has_capa(iwl, IWL_UCODE_TLV_CAPA_TLC_OFFLOAD)) {
        ret = iwl_mvm_send_tlc_config_ap(iwl);
        if (ret == 0)
            lx_printk("iwl_mvm: TLC_MNG_CONFIG AP sta_id=%u async (20 MHz legacy)\n",
                      (unsigned)iwl->ap_sta_id);
        else
            lx_printk("iwl_mvm: rates por TLC offload; LQ_CMD omitido\n");
        return ret;
    }
    iwl_mvm_fill_lq_ap(iwl, &lq);
    ret = iwl_mvm_send_lq_cmd(iwl, &lq);
    if (ret == 0)
        lx_printk("iwl_mvm: LQ_CMD AP sta_id=%u async (6 Mbps legacy)\n",
                  (unsigned)lq.sta_id);
    return ret;
}
