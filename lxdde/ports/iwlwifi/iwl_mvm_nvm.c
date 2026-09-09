/* NVM: lectura MAC y plantilla probe request 802.11. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);

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
