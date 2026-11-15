#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include "iwl_internal.h"
#include "iwl_ax211.h"

static int fault;
static unsigned calls;
void lx_printk(const char *fmt, ...) { (void)fmt; }
uint32_t iwl_trans_8000_read_prph(struct iwl_ax211_priv *iwl, uint32_t addr)
{ (void)iwl; return addr == 0xa03080 ? 0x10203040 : 0x5060; }
int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t len, int timeout)
{
    const struct iwl_nvm_access_cmd *cmd = payload;
    struct iwl_nvm_access_resp *rsp = (void *)iwl->cmd_resp;
    unsigned section = cmd->type, off = cmd->offset, size = 0, n;
    (void)timeout;
    calls++;
    if (group != LEGACY_GROUP || id != NVM_ACCESS_CMD || len != sizeof(*cmd) ||
        cmd->op_code != 0 || cmd->target != 0 || cmd->length != 256)
        return -1;
    if (section == 1) size = 16;
    if (section == 3) size = 2600;
    if (section == 10) size = fault == 4 ? 9000 : 32;
    if (section == 12 && fault != 1) size = 16;
    memset(iwl->cmd_resp, 0, sizeof(iwl->cmd_resp));
    iwl->cmd_resp_trunc = 0;
    rsp->offset = off;
    rsp->type = section;
    if (off >= size) {
        rsp->status = 1;
        iwl->cmd_resp_len = sizeof(*rsp);
        return 0;
    }
    n = size - off;
    if (n > 256) n = 256;
    rsp->length = n;
    if (section == 1 && off == 0) { rsp->data[0] = 0x39; rsp->data[1] = 0x0e; }
    if (section == 3 && off == 0) { rsp->data[0] = 9; rsp->data[2] = 1; }
    if (section == 12 && off == 0) rsp->data[3] = 0x33;
    if (section == 3 && off <= 0xa0e && off + n > 0xa0e) rsp->data[0xa0e - off] = 1;
    iwl->cmd_resp_len = sizeof(*rsp) + n;
    if (fault == 2 && section == 3 && off == 256) iwl->cmd_resp_len--;
    if (fault == 3 && section == 3 && off == 256) rsp->offset++;
    return 0;
}
int main(void)
{
    struct iwl_ax211_priv iwl;
    unsigned i;
    for (i = 0; i < 5; i++) {
        memset(&iwl, 0, sizeof(iwl));
        fault = i; calls = 0;
        int rc = iwl_mvm_nvm_read_8000(&iwl);
        if (((i == 0 || i == 4) && (rc || !iwl.nvm_ready || iwl.nvm_n_channels != 51 ||
                     iwl.nvm_chan_flags[0] != 9 || !iwl.lar_enabled ||
                     iwl.valid_tx_ant != 3 || iwl.valid_rx_ant != 3 ||
                     iwl.mac[0] != 0x10 || iwl.mac[5] != 0x60 || calls < 20)) ||
            (i > 0 && i < 4 && (!rc || iwl.nvm_ready))) {
            fprintf(stderr, "NVM 8000 caso %u rc=%d ready=%u calls=%u\n", i, rc, iwl.nvm_ready, calls);
            return 1;
        }
    }
    puts("OK: NVM 8265 por secciones, límites de 32 KiB, respuesta truncada, offset incorrecto y PHY SKU ausente");
    return 0;
}
