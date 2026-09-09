/* NVM: lectura MAC y plantilla probe request 802.11. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);

static uint32_t iwl_mmio_rd32(struct iwl_ax211_priv *iwl, uint32_t off)
{
    return iwl->mmio[off / 4];
}

static int mac_valid(const uint8_t mac[6])
{
    int i;
    int nz = 0;

    for (i = 0; i < 6; i++) {
        if (mac[i] != 0)
            nz = 1;
    }
    if (!nz)
        return 0;
    if (mac[0] == 0xff)
        return 0;
    return 1;
}

int iwl_mvm_nvm_read_mac(struct iwl_ax211_priv *iwl)
{
    struct iwl_nvm_access_cmd cmd;
    const struct iwl_nvm_access_resp *resp;
    const uint8_t *hw;
    unsigned data_len;
    uint16_t status;
    uint16_t bytes_read;

    memset(&cmd, 0, sizeof(cmd));
    cmd.op_code = (uint8_t)IWL_NVM_READ;
    cmd.target = (uint8_t)NVM_ACCESS_TARGET_CACHE;
    cmd.type = (uint16_t)IWL_NVM_SECTION_TYPE_HW;
    cmd.offset = 0;
    cmd.length = 256;

    if (iwl_trans_send_cmd_wait(iwl, REGULATORY_AND_NVM_GROUP, NVM_ACCESS_CMD,
                                &cmd, (uint16_t)sizeof(cmd), 2000) != 0) {
        lx_printk("iwl_mvm: NVM_ACCESS_CMD falló\n");
        return -1;
    }

    if (iwl->cmd_resp_len < (uint16_t)sizeof(struct iwl_nvm_access_resp)) {
        lx_printk("iwl_mvm: NVM resp corta (%u)\n", (unsigned)iwl->cmd_resp_len);
        return -1;
    }

    resp = (const struct iwl_nvm_access_resp *)iwl->cmd_resp;
    status = resp->status;
    bytes_read = resp->length;
    if (status != READ_NVM_CHUNK_SUCCEED) {
        lx_printk("iwl_mvm: NVM_ACCESS status=0x%04x\n", (unsigned)status);
        return -1;
    }

    hw = resp->data;
    data_len = bytes_read;
    if (iwl->cmd_resp_len > (uint16_t)sizeof(struct iwl_nvm_access_resp)) {
        unsigned avail = iwl->cmd_resp_len - (uint16_t)sizeof(struct iwl_nvm_access_resp);
        if (data_len > avail)
            data_len = avail;
    }

    if (data_len >= NVM_MAC_ADDR_OFFSET + 6u) {
        memcpy(iwl->mac, hw + NVM_MAC_ADDR_OFFSET, 6);
        if (mac_valid(iwl->mac)) {
            lx_printk("iwl_mvm: MAC NVM 0x%02x:%02x:%02x:%02x:%02x:%02x\n",
                      iwl->mac[0], iwl->mac[1], iwl->mac[2],
                      iwl->mac[3], iwl->mac[4], iwl->mac[5]);
            return 0;
        }
    }

    if (data_len >= 6u && mac_valid(hw)) {
        memcpy(iwl->mac, hw, 6);
        lx_printk("iwl_mvm: MAC NVM (offset 0)\n");
        return 0;
    }

    lx_printk("iwl_mvm: NVM sin MAC válida\n");
    return -1;
}

/* `iwl_flip_hw_address` + `iwl_set_hw_address_from_csr` (iwl-nvm-parse.c). */
static void iwl_flip_hw_address(uint32_t mac_addr0, uint32_t mac_addr1, uint8_t *dest)
{
    const uint8_t *hw;

    hw = (const uint8_t *)&mac_addr0;
    dest[0] = hw[3];
    dest[1] = hw[2];
    dest[2] = hw[1];
    dest[3] = hw[0];

    hw = (const uint8_t *)&mac_addr1;
    dest[4] = hw[1];
    dest[5] = hw[0];
}

static void iwl_mac_from_csr(struct iwl_ax211_priv *iwl, uint8_t mac[6])
{
    uint32_t mac_addr0;
    uint32_t mac_addr1;

    mac_addr0 = iwl_mmio_rd32(iwl, CSR_MAC_ADDR0_STRAP);
    mac_addr1 = iwl_mmio_rd32(iwl, CSR_MAC_ADDR1_STRAP);
    iwl_flip_hw_address(mac_addr0, mac_addr1, mac);
    if (mac_valid(mac))
        return;

    mac_addr0 = iwl_mmio_rd32(iwl, CSR_MAC_ADDR0_OTP);
    mac_addr1 = iwl_mmio_rd32(iwl, CSR_MAC_ADDR1_OTP);
    iwl_flip_hw_address(mac_addr0, mac_addr1, mac);
}

uint8_t iwl_mvm_valid_tx_ant(struct iwl_ax211_priv *iwl)
{
    uint8_t nvm = iwl->valid_tx_ant;
    uint8_t fw = iwl->fw_valid_tx_ant;

    if (nvm && fw) {
        return fw & nvm;
    }
    if (nvm) {
        return nvm;
    }
    if (fw) {
        return fw;
    }
    return 0x3; /* ANT_AB — fallback 2x2 AX200 */
}

uint8_t iwl_mvm_valid_rx_ant(struct iwl_ax211_priv *iwl)
{
    uint8_t nvm = iwl->valid_rx_ant;
    uint8_t fw = iwl->fw_valid_rx_ant;

    if (nvm && fw) {
        return fw & nvm;
    }
    if (nvm) {
        return nvm;
    }
    if (fw) {
        return fw;
    }
    return 0x3;
}

int iwl_mvm_init_mcc(struct iwl_ax211_priv *iwl)
{
    struct iwl_mcc_update_cmd cmd;
    const struct iwl_mcc_update_resp_v8 *rsp;

    if (!iwl->lar_enabled || iwl->mcc_done) {
        return 0;
    }

    memset(&cmd, 0, sizeof(cmd));
    /* `iwl_mvm_get_current_regdomain`: ZZ + GET_CURRENT → perfil NVM en FW. */
    cmd.mcc = iwl_cpu_to_le16((uint16_t)(('Z' << 8) | 'Z'));
    cmd.source_id = MCC_SOURCE_GET_CURRENT;

    if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, MCC_UPDATE_CMD,
                                &cmd, (uint16_t)sizeof(cmd),
                                IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
        lx_printk("iwl_mvm: MCC_UPDATE falló\n");
        return -1;
    }

    if (iwl->cmd_resp_len < (uint16_t)sizeof(*rsp)) {
        lx_printk("iwl_mvm: MCC_UPDATE resp corta (%u B)\n",
                  (unsigned)iwl->cmd_resp_len);
        return -1;
    }

    rsp = (const struct iwl_mcc_update_resp_v8 *)iwl->cmd_resp;
    iwl->mcc_done = 1;
    lx_printk("iwl_mvm: MCC_UPDATE ok status=%u mcc=%c%c\n",
              (unsigned)rsp->status,
              (char)(rsp->mcc >> 8), (char)(rsp->mcc & 0xff));
    return 0;
}

int iwl_mvm_send_tx_ant_cfg(struct iwl_ax211_priv *iwl)
{
    struct iwl_tx_ant_cfg_cmd cmd;
    uint8_t ant = iwl_mvm_valid_tx_ant(iwl);

    memset(&cmd, 0, sizeof(cmd));
    cmd.valid = iwl_cpu_to_le32(ant);
    if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, TX_ANT_CONFIGURATION_CMD,
                                &cmd, (uint16_t)sizeof(cmd),
                                IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
        lx_printk("iwl_mvm: TX_ANT_CONFIGURATION falló\n");
        return -1;
    }
    lx_printk("iwl_mvm: TX_ANT_CONFIGURATION ok (ant=0x%x)\n", (unsigned)ant);
    return 0;
}

static void nvm_apply_get_info_rsp(struct iwl_ax211_priv *iwl,
                                   const struct iwl_nvm_get_info_phy *phy,
                                   uint32_t lar_enabled)
{
    iwl->valid_tx_ant = (uint8_t)phy->tx_chains;
    iwl->valid_rx_ant = (uint8_t)phy->rx_chains;
    iwl->lar_enabled = lar_enabled ? 1u : 0u;
    lx_printk("iwl_mvm: NVM phy tx=0x%x rx=0x%x lar=%u\n",
              (unsigned)iwl->valid_tx_ant, (unsigned)iwl->valid_rx_ant,
              (unsigned)iwl->lar_enabled);
}

int iwl_mvm_nvm_get_info_mac(struct iwl_ax211_priv *iwl)
{
    struct iwl_nvm_get_info cmd;
    uint8_t mac[6];
    uint16_t rsp_len = iwl->cmd_resp_len;

    memset(&cmd, 0, sizeof(cmd));
    if (iwl_trans_send_cmd_wait(iwl, REGULATORY_AND_NVM_GROUP, NVM_GET_INFO,
                                &cmd, (uint16_t)sizeof(cmd), 2000) != 0) {
        lx_printk("iwl_mvm: NVM_GET_INFO falló\n");
        return -1;
    }

    rsp_len = iwl->cmd_resp_len;
    if (rsp_len >= (uint16_t)sizeof(struct iwl_nvm_get_info_rsp)) {
        const struct iwl_nvm_get_info_rsp *rsp =
            (const struct iwl_nvm_get_info_rsp *)iwl->cmd_resp;

        nvm_apply_get_info_rsp(iwl, &rsp->phy_sku, rsp->regulatory.lar_enabled);
        lx_printk("iwl_mvm: NVM_GET_INFO v4 nvm_ver=0x%04x\n",
                  (unsigned)rsp->general.nvm_version);
    } else if (rsp_len >= (uint16_t)sizeof(struct iwl_nvm_get_info_rsp_v3)) {
        const struct iwl_nvm_get_info_rsp_v3 *rsp =
            (const struct iwl_nvm_get_info_rsp_v3 *)iwl->cmd_resp;

        nvm_apply_get_info_rsp(iwl, &rsp->phy_sku, rsp->regulatory.lar_enabled);
        lx_printk("iwl_mvm: NVM_GET_INFO v3 nvm_ver=0x%04x\n",
                  (unsigned)rsp->general.nvm_version);
    } else {
        lx_printk("iwl_mvm: NVM_GET_INFO resp corta (%u B)\n", (unsigned)rsp_len);
    }

    memset(mac, 0, sizeof(mac));
    iwl_mac_from_csr(iwl, mac);
    if (!mac_valid(mac)) {
        lx_printk("iwl_mvm: CSR sin MAC válida tras NVM_GET_INFO\n");
        return -1;
    }

    memcpy(iwl->mac, mac, 6);
    lx_printk("iwl_mvm: MAC NVM 0x%02x:%02x:%02x:%02x:%02x:%02x\n",
              iwl->mac[0], iwl->mac[1], iwl->mac[2],
              iwl->mac[3], iwl->mac[4], iwl->mac[5]);
    return 0;
}

void iwl_mvm_fill_probe_req(struct iwl_ax211_priv *iwl, struct iwl_scan_probe_params_v4 *probe)
{
    uint8_t *f = probe->preq.buf;
    unsigned pos = 0;
    unsigned mac_end;
    unsigned band_start;
    unsigned band_end;

    memset(probe, 0, sizeof(*probe));

    f[pos++] = 0x40;
    f[pos++] = 0x00;
    f[pos++] = 0x00;
    f[pos++] = 0x00;
    memset(&f[pos], 0xff, 6);
    pos += 6;
    memcpy(&f[pos], iwl->mac, 6);
    pos += 6;
    memset(&f[pos], 0xff, 6);
    pos += 6;
    f[pos++] = 0x00;
    f[pos++] = 0x00;
    mac_end = pos;

    f[pos++] = 0x00;
    f[pos++] = 0x00;
    band_start = pos;
    f[pos++] = 0x01;
    f[pos++] = 0x08;
    f[pos++] = 0x82;
    f[pos++] = 0x84;
    f[pos++] = 0x8b;
    f[pos++] = 0x96;
    f[pos++] = 0x0c;
    f[pos++] = 0x12;
    f[pos++] = 0x18;
    f[pos++] = 0x24;
    f[pos++] = 0x03;
    f[pos++] = 0x01;
    f[pos++] = 0x06;
    band_end = pos;

    probe->preq.mac_header.offset = 0;
    probe->preq.mac_header.len = (uint16_t)mac_end;
    probe->preq.band_data[0].offset = (uint16_t)band_start;
    probe->preq.band_data[0].len = (uint16_t)(band_end - band_start);
    probe->preq.band_data[1].offset = 0;
    probe->preq.band_data[1].len = 0;
    probe->preq.common_data.offset = 0;
    probe->preq.common_data.len = 0;
}
