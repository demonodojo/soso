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

int iwl_mac_valid_unicast(const uint8_t mac[6])
{
    int i;
    int nz = 0;
    int not_ff = 0;

    if (mac[0] & 1u)
        return 0;
    for (i = 0; i < 6; i++) {
        if (mac[i] != 0)
            nz = 1;
        if (mac[i] != 0xff)
            not_ff = 1;
    }
    return nz && not_ff;
}

uint32_t iwl_mac_addr_from_csr(uint16_t device_id)
{
    switch (device_id) {
    case IWL_PCI_AX200:
    case 0x7f70:
    case 0x51f0:
    case 0x54f0:
        return CSR_MAC_ADDR_FROM_CSR_22000;
    default:
        return CSR_MAC_ADDR_FROM_CSR_22000;
    }
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
    if (iwl->cmd_resp_trunc) {
        lx_printk("iwl_mvm: NVM resp recortada (%u de %u B); no se interpreta\n",
                  (unsigned)iwl->cmd_resp_len, (unsigned)iwl->cmd_resp_wire_len);
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
        if (iwl_mac_valid_unicast(iwl->mac)) {
            lx_printk("iwl_mvm: MAC NVM 0x%02x:%02x:%02x:%02x:%02x:%02x\n",
                      iwl->mac[0], iwl->mac[1], iwl->mac[2],
                      iwl->mac[3], iwl->mac[4], iwl->mac[5]);
            return 0;
        }
    }

    if (data_len >= 6u && iwl_mac_valid_unicast(hw)) {
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

void iwl_mac_from_csr(struct iwl_ax211_priv *iwl, uint8_t mac[6])
{
    uint32_t base = iwl_mac_addr_from_csr(iwl->device_id);
    uint32_t mac_addr0;
    uint32_t mac_addr1;

    mac_addr0 = iwl_mmio_rd32(iwl, CSR_MAC_ADDR0_STRAP(base));
    mac_addr1 = iwl_mmio_rd32(iwl, CSR_MAC_ADDR1_STRAP(base));
    iwl_flip_hw_address(mac_addr0, mac_addr1, mac);
    if (iwl_mac_valid_unicast(mac))
        return;

    mac_addr0 = iwl_mmio_rd32(iwl, CSR_MAC_ADDR0_OTP(base));
    mac_addr1 = iwl_mmio_rd32(iwl, CSR_MAC_ADDR1_OTP(base));
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

/* R6: aplica la respuesta de MCC_UPDATE.
 *
 * Linux (`iwl_mvm_update_mcc`, mvm/nvm.c) elige el layout con la versión de
 * *notificación* y exige que el payload mida exactamente cabecera + 4·n_canales;
 * después `iwl_parse_nvm_mcc_info` construye el regdominio con esa lista, que
 * está indexada igual que el perfil de canales del NVM. Aquí se hace lo mismo:
 * sin validación completa no se aplica nada y `lar_regdom_set` sigue en 0.
 */
int iwl_mvm_apply_mcc_resp(struct iwl_ax211_priv *iwl, int notif_ver,
                           const uint8_t *resp, unsigned len)
{
    unsigned hdr;
    uint32_t status;
    uint32_t n_channels;
    uint16_t mcc;
    const uint32_t *channels;
    unsigned i;
    unsigned validos = 0;

    if (!iwl || !resp)
        return -1;

    if (notif_ver >= 8) {
        hdr = (unsigned)sizeof(struct iwl_mcc_update_resp_v8);
    } else if (notif_ver >= 4) {
        hdr = (unsigned)sizeof(struct iwl_mcc_update_resp_v4);
    } else {
        hdr = (unsigned)sizeof(struct iwl_mcc_update_resp_v3);
    }
    if (len < hdr) {
        lx_printk("iwl_mvm: MCC v%d resp corta (%u < %u B)\n", notif_ver, len, hdr);
        return -1;
    }

    /* status y mcc están en el mismo sitio en v3/v4/v8; n_channels no. */
    status = *(const uint32_t *)(const void *)resp;
    mcc = *(const uint16_t *)(const void *)(resp + 4);
    if (notif_ver >= 8) {
        n_channels = ((const struct iwl_mcc_update_resp_v8 *)(const void *)resp)->n_channels;
    } else if (notif_ver >= 4) {
        n_channels = ((const struct iwl_mcc_update_resp_v4 *)(const void *)resp)->n_channels;
    } else {
        n_channels = ((const struct iwl_mcc_update_resp_v3 *)(const void *)resp)->n_channels;
    }

    if (n_channels > IWL_NUM_CHANNELS) {
        lx_printk("iwl_mvm: MCC anuncia %u canales (máx %u); descartado\n",
                  (unsigned)n_channels, (unsigned)IWL_NUM_CHANNELS);
        return -1;
    }
    /* Igualdad exacta, como Linux: ni de más ni de menos. */
    if (len != hdr + n_channels * 4u) {
        lx_printk("iwl_mvm: MCC tamaño %u ≠ %u (%u canales); descartado\n",
                  len, hdr + n_channels * 4u, (unsigned)n_channels);
        return -1;
    }

    iwl->mcc_status = status;
    switch (status) {
    case MCC_RESP_NEW_CHAN_PROFILE:
    case MCC_RESP_SAME_CHAN_PROFILE:
    case MCC_RESP_ILLEGAL:
    case MCC_RESP_LOW_PRIORITY:
        break;
    default:
        /* INVALID / NVM_DISABLED / modos de test: no hay perfil que aplicar. */
        lx_printk("iwl_mvm: MCC status=%u sin perfil aplicable\n", (unsigned)status);
        return -1;
    }

    /* W/A de Linux: 0x0000 es el dominio mundial "00". */
    if (mcc == 0)
        mcc = 0x3030;

    channels = (const uint32_t *)(const void *)(resp + hdr);
    for (i = 0; i < n_channels; i++) {
        iwl->nvm_chan_flags[i] = channels[i];
        if (channels[i] & NVM_CHANNEL_VALID)
            validos++;
    }
    for (i = n_channels; i < IWL_NUM_CHANNELS; i++)
        iwl->nvm_chan_flags[i] = 0;
    iwl->nvm_n_channels = n_channels;
    iwl->mcc_applied = mcc;
    iwl->lar_regdom_set = 1;
    iwl->mcc_done = 1;
    iwl->chan_src = IWL_CHAN_SRC_MCC;
    lx_printk("iwl_mvm: MCC aplicado %c%c status=%u canales=%u válidos=%u\n",
              (char)(mcc >> 8), (char)(mcc & 0xff), (unsigned)status,
              (unsigned)n_channels, validos);
    return 0;
}

int iwl_mvm_init_mcc(struct iwl_ax211_priv *iwl)
{
    struct iwl_mcc_update_cmd cmd;
    int notif_ver;

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
    if (iwl->cmd_resp_trunc) {
        lx_printk("iwl_mvm: MCC_UPDATE recortada (%u de %u B)\n",
                  (unsigned)iwl->cmd_resp_len, (unsigned)iwl->cmd_resp_wire_len);
        return -1;
    }

    notif_ver = iwl_fw_notif_ver(iwl, LEGACY_GROUP, MCC_UPDATE_CMD);
    return iwl_mvm_apply_mcc_resp(iwl, notif_ver, iwl->cmd_resp,
                                  iwl->cmd_resp_len);
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

    if (iwl->cmd_resp_trunc) {
        lx_printk("iwl_mvm: NVM_GET_INFO recortada (%u de %u B)\n",
                  (unsigned)iwl->cmd_resp_len, (unsigned)iwl->cmd_resp_wire_len);
        return -1;
    }
    rsp_len = iwl->cmd_resp_len;
    if (rsp_len >= (uint16_t)sizeof(struct iwl_nvm_get_info_rsp)) {
        const struct iwl_nvm_get_info_rsp *rsp =
            (const struct iwl_nvm_get_info_rsp *)iwl->cmd_resp;
        uint32_t n = rsp->regulatory.n_channels;

        nvm_apply_get_info_rsp(iwl, &rsp->phy_sku, rsp->regulatory.lar_enabled);
        if (n > IWL_NUM_CHANNELS)
            n = IWL_NUM_CHANNELS;
        iwl->nvm_n_channels = n;
        memcpy(iwl->nvm_chan_flags, rsp->regulatory.channel_profile, n * sizeof(uint32_t));
        iwl->chan_src = IWL_CHAN_SRC_NVM;
        lx_printk("iwl_mvm: NVM_GET_INFO v4 nvm_ver=0x%04x nch=%u\n",
                  (unsigned)rsp->general.nvm_version, (unsigned)n);
    } else if (rsp_len >= (uint16_t)sizeof(struct iwl_nvm_get_info_rsp_v3)) {
        const struct iwl_nvm_get_info_rsp_v3 *rsp =
            (const struct iwl_nvm_get_info_rsp_v3 *)iwl->cmd_resp;
        unsigned i;
        unsigned n = IWL_NUM_CHANNELS_V1;

        nvm_apply_get_info_rsp(iwl, &rsp->phy_sku, rsp->regulatory.lar_enabled);
        iwl->nvm_n_channels = n;
        for (i = 0; i < n; i++)
            iwl->nvm_chan_flags[i] = rsp->regulatory.channel_profile[i];
        iwl->chan_src = IWL_CHAN_SRC_NVM;
        lx_printk("iwl_mvm: NVM_GET_INFO v3 nvm_ver=0x%04x\n",
                  (unsigned)rsp->general.nvm_version);
    } else {
        lx_printk("iwl_mvm: NVM_GET_INFO resp truncada (%u B)\n", (unsigned)rsp_len);
        return -1;
    }

    memset(mac, 0, sizeof(mac));
    iwl_mac_from_csr(iwl, mac);
    if (!iwl_mac_valid_unicast(mac)) {
        lx_printk("iwl_mvm: CSR sin MAC unicast tras NVM_GET_INFO\n");
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
    f[pos++] = iwl->channel ? iwl->channel : 1;
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
