/* TX 802.3 (EAPOL / datos) sobre cola TVQM de datos. */

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);

uint32_t iwl_mvm_tx_rate_n_flags(struct iwl_ax211_priv *iwl)
{
    uint32_t ant = RATE_MCS_ANT_A_MSK;
    int ver = iwl_fw_cmd_ver(iwl, LEGACY_GROUP, TX_CMD);

    if (ver > 8)
        return RATE_MCS_LEGACY_OFDM_MSK | RATE_LEGACY_OFDM_6M | ant;
    return RATE_LEGACY_PLCP_6M | ant;
}

int iwl_mvm_tx_8023(struct iwl_ax211_priv *iwl, const uint8_t *buf, int len)
{
    uint8_t frame[IWL_MAX_ETH_FRAME + 32];
    uint8_t txbuf[IWL_MGMT_TX_SLOT_SIZE];
    uint32_t flags = IWL_TX_FLAGS_CMD_RATE | IWL_TX_FLAGS_HIGH_PRI;
    uint32_t rate;
    unsigned hdr_off;
    uint16_t pay;
    uint16_t txq_id;
    int flen;
    uint16_t offload_assist = (uint16_t)((24u / 2u) << TX_CMD_OFFLD_MH_SIZE);

    if (!iwl->associated || !buf || len <= 0)
        return -1;
    if (!iwl->data_txq_ready) {
        if (iwl_trans_txq_alloc_data(iwl, IWL_MVM_AP_STA_ID, IWL_TID_NON_QOS) < 0) {
            lx_printk("iwl_mvm: TXQ data (EAPOL) falló\n");
            return -1;
        }
        iwl_trans_txq_drain_data(iwl);
    }
    if (!iwl->data_txq_ready)
        return -1;
    txq_id = iwl->data_txq_id;
    if (len > (int)IWL_MAX_ETH_FRAME)
        return -1;

    flen = iwl_mvm_eth_to_80211(iwl, buf, len, frame, (int)sizeof(frame));
    if (flen <= 0)
        return -1;

    if (!iwl->keys_installed)
        flags |= IWL_TX_FLAGS_ENCRYPT_DIS;

    rate = iwl_mvm_tx_rate_n_flags(iwl);

    memset(txbuf, 0, sizeof(txbuf));
    if (iwl->gen3) {
        struct iwl_tx_cmd_gen3 *cmd = (struct iwl_tx_cmd_gen3 *)txbuf;

        hdr_off = (unsigned)sizeof(struct iwl_tx_cmd_gen3);
        cmd->len = iwl_cpu_to_le16((uint16_t)flen);
        cmd->flags = iwl_cpu_to_le16((uint16_t)flags);
        cmd->offload_assist = iwl_cpu_to_le32((uint32_t)offload_assist);
        cmd->rate_n_flags = iwl_cpu_to_le32(rate);
    } else {
        struct iwl_tx_cmd_gen2 *cmd = (struct iwl_tx_cmd_gen2 *)txbuf;

        hdr_off = (unsigned)sizeof(struct iwl_tx_cmd_gen2);
        cmd->len = iwl_cpu_to_le16((uint16_t)flen);
        cmd->offload_assist = offload_assist;
        cmd->flags = iwl_cpu_to_le32(flags);
        cmd->rate_n_flags = iwl_cpu_to_le32(rate);
    }
    if (hdr_off + (unsigned)flen > sizeof(txbuf))
        return -1;
    memcpy(txbuf + hdr_off, frame, (unsigned)flen);
    pay = (uint16_t)(hdr_off + (unsigned)flen);
    if (iwl_trans_tx(iwl, txq_id, txbuf, pay) != 0)
        return -1;
    return 0;
}
