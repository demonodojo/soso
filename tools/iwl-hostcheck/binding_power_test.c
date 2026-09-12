/* Host: up mínimo manda BINDING (0x2b) PHY0↔MAC0 y POWER 0x77/0xA9 en orden Linux. */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

#define FW_CTXT_ID_POS             0
#define FW_CTXT_COLOR_POS          8
#define FW_CMD_ID_AND_COLOR(id, color) \
    (((uint32_t)(id) << FW_CTXT_ID_POS) | ((uint32_t)(color) << FW_CTXT_COLOR_POS))
#define FW_CTXT_ACTION_ADD         1
#define MAC_CONTEXT_CMD            0x28

struct cmd_rec {
    uint8_t group;
    uint8_t id;
    uint16_t len;
    uint8_t payload[64];
};

static struct cmd_rec g_sent[24];
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

int iwl_trans_send_cmd_async(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                             const void *payload, uint16_t pay_len)
{
    (void)iwl;
    (void)group;
    (void)id;
    (void)payload;
    (void)pay_len;
    return 0;
}

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

int iwl_mvm_send_scan_cfg(struct iwl_ax211_priv *iwl)
{
    if (g_sent_n < (int)(sizeof(g_sent) / sizeof(g_sent[0]))) {
        g_sent[g_sent_n].group = LONG_GROUP;
        g_sent[g_sent_n].id = SCAN_CFG_CMD;
        g_sent[g_sent_n].len = 0;
        g_sent_n++;
    }
    iwl->scan_cfg_sent = 1;
    return 0;
}

#include "iwl_fw_body.inc"

static int find_cmd(uint8_t group, uint8_t id, int after)
{
    int i;

    for (i = after + 1; i < g_sent_n; i++) {
        if (g_sent[i].group == group && g_sent[i].id == id)
            return i;
    }
    return -1;
}

static int check_binding_payload(int idx)
{
    const struct iwl_binding_cmd_v1 *cmd;
    uint32_t phy0;
    uint32_t mac0;

    if (g_sent[idx].len != IWL_BINDING_CMD_SIZE_V1 &&
        g_sent[idx].len != (uint16_t)sizeof(struct iwl_binding_cmd)) {
        fprintf(stderr, "BINDING payload=%u (esperaba V1 %u o CDB %zu)\n",
                (unsigned)g_sent[idx].len, (unsigned)IWL_BINDING_CMD_SIZE_V1,
                sizeof(struct iwl_binding_cmd));
        return -1;
    }
    cmd = (const struct iwl_binding_cmd_v1 *)g_sent[idx].payload;
    phy0 = FW_CMD_ID_AND_COLOR(0, 0);
    mac0 = FW_CMD_ID_AND_COLOR(0, 0);
    if (cmd->action != iwl_cpu_to_le32(FW_CTXT_ACTION_ADD) ||
        cmd->id_and_color != iwl_cpu_to_le32(phy0) ||
        cmd->phy != iwl_cpu_to_le32(phy0) ||
        cmd->macs[0] != iwl_cpu_to_le32(mac0) ||
        cmd->macs[1] != iwl_cpu_to_le32(FW_CTXT_INVALID) ||
        cmd->macs[2] != iwl_cpu_to_le32(FW_CTXT_INVALID)) {
        fprintf(stderr, "BINDING payload inválido (action/phy/mac)\n");
        return -1;
    }
    if (g_sent[idx].len == (uint16_t)sizeof(struct iwl_binding_cmd)) {
        const struct iwl_binding_cmd *cdb = (const struct iwl_binding_cmd *)cmd;

        if (cdb->lmac_id != iwl_cpu_to_le32(0)) {
            fprintf(stderr, "BINDING CDB lmac_id distinto de 0\n");
            return -1;
        }
    }
    return 0;
}

static int check_device_power(int idx)
{
    const struct iwl_device_power_cmd *cmd;

    if (g_sent[idx].len != sizeof(*cmd)) {
        fprintf(stderr, "POWER_TABLE len=%u (esperaba %zu)\n",
                (unsigned)g_sent[idx].len, sizeof(*cmd));
        return -1;
    }
    cmd = (const struct iwl_device_power_cmd *)g_sent[idx].payload;
    if (cmd->flags != 0) {
        fprintf(stderr, "POWER_TABLE flags=0x%x (esperaba PS off)\n",
                (unsigned)cmd->flags);
        return -1;
    }
    return 0;
}

static int check_mac_power(int idx)
{
    const struct iwl_mac_power_cmd *cmd;

    if (g_sent[idx].len != sizeof(*cmd)) {
        fprintf(stderr, "MAC_PM len=%u (esperaba %zu)\n",
                (unsigned)g_sent[idx].len, sizeof(*cmd));
        return -1;
    }
    cmd = (const struct iwl_mac_power_cmd *)g_sent[idx].payload;
    if (cmd->id_and_color != iwl_cpu_to_le32(FW_CMD_ID_AND_COLOR(0, 0)) ||
        cmd->keep_alive_seconds != iwl_cpu_to_le16(POWER_KEEP_ALIVE_PERIOD_SEC) ||
        cmd->flags != 0) {
        fprintf(stderr, "MAC_PM id/keep_alive/flags no cuadran\n");
        return -1;
    }
    return 0;
}

static int run_up_binding_power(const struct iwl_ax211_priv *parsed)
{
    struct iwl_ax211_priv iwl;
    int idx_mac;
    int idx_bind;
    int idx_devpwr;
    int idx_macpwr;
    int idx_scan;

    memset(&iwl, 0, sizeof(iwl));
    memcpy(iwl.fw_capa, parsed->fw_capa, sizeof(iwl.fw_capa));
    iwl.radio_ready = 1;
    iwl.phy_sku = parsed->phy_sku;
    iwl.fw_valid_tx_ant = parsed->fw_valid_tx_ant ? parsed->fw_valid_tx_ant : 3;
    iwl.fw_valid_rx_ant = parsed->fw_valid_rx_ant ? parsed->fw_valid_rx_ant : 3;
    iwl.lar_enabled = 0;
    iwl.scan_mac_id = 0;
    iwl.cmd_ver_count = parsed->cmd_ver_count;
    memcpy(iwl.cmd_ver, parsed->cmd_ver, sizeof(iwl.cmd_ver));

    g_sent_n = 0;
    if (iwl_mvm_up_minimal(&iwl) != 0) {
        fprintf(stderr, "iwl_mvm_up_minimal falló\n");
        return -1;
    }
    if (!iwl.binding_added || !iwl.mac_ctxt_added || !iwl.phy_ctxt_added) {
        fprintf(stderr, "flags binding/mac/phy no marcadas\n");
        return -1;
    }

    idx_mac = find_cmd(LEGACY_GROUP, MAC_CONTEXT_CMD, -1);
    idx_bind = find_cmd(LEGACY_GROUP, BINDING_CONTEXT_CMD, idx_mac);
    idx_devpwr = find_cmd(LEGACY_GROUP, POWER_TABLE_CMD, -1);
    idx_macpwr = find_cmd(LEGACY_GROUP, MAC_PM_POWER_TABLE, idx_bind);
    idx_scan = find_cmd(LONG_GROUP, SCAN_CFG_CMD, idx_macpwr);

    if (idx_mac < 0 || idx_bind < 0 || idx_devpwr < 0 || idx_macpwr < 0 ||
        idx_scan < 0) {
        fprintf(stderr, "orden HCMD incompleto mac=%d bind=%d dev=%d macpm=%d scan=%d\n",
                idx_mac, idx_bind, idx_devpwr, idx_macpwr, idx_scan);
        return -1;
    }
    if (!(idx_mac < idx_bind && idx_bind < idx_macpwr && idx_macpwr < idx_scan)) {
        fprintf(stderr, "orden incorrecto: MAC(%d) BIND(%d) MACPM(%d) SCAN(%d)\n",
                idx_mac, idx_bind, idx_macpwr, idx_scan);
        return -1;
    }
    if (idx_devpwr >= idx_mac) {
        fprintf(stderr, "POWER_TABLE dispositivo debe ir antes de MAC_CONTEXT\n");
        return -1;
    }

    if (check_binding_payload(idx_bind) != 0 ||
        check_device_power(idx_devpwr) != 0 ||
        check_mac_power(idx_macpwr) != 0) {
        return -1;
    }

    puts("OK: BINDING V1 PHY0↔MAC0 + POWER 0x77/0xA9 antes de SCAN_CFG");
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
        return 1;
    }
    free(fw);
    return run_up_binding_power(&iwl) != 0 ? 1 : 0;
}
