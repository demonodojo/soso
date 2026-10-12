/* Host: init unificado MVM no debe mandar PHY_CONFIGURATION_CMD (0x0b). */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

struct iwl_ax211_priv g_test;
int g_poll_calls;
int g_fast_ack;
int g_doorbell_n;
int g_block_pnvm;
int g_init_too_early;

static struct {
    uint8_t group;
    uint8_t id;
} g_sent[16];
static int g_sent_n;
int iwl_trans_send_cmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                       const void *payload, uint16_t pay_len)
{
    (void)payload;
    (void)pay_len;

    if ((iwl->sku_id[0] || iwl->sku_id[1] || iwl->sku_id[2]) &&
        group == SYSTEM_GROUP && id == INIT_EXTENDED_CFG_CMD &&
        !iwl->pnvm_complete) {
        g_init_too_early = 1;
    }

    if (g_sent_n < (int)(sizeof(g_sent) / sizeof(g_sent[0]))) {
        g_sent[g_sent_n].group = group;
        g_sent[g_sent_n].id = id;
        g_sent_n++;
    }

    /* Como `send_cmd_wait` drenando RX: INIT_COMPLETE puede llegar tras NVM_ACCESS. */
    if (group == REGULATORY_AND_NVM_GROUP && id == NVM_ACCESS_COMPLETE) {
        iwl->init_complete = 1;
    }

    iwl->cmd_pending = 1;
    iwl->cmd_status = g_fast_ack ? 1 : 0;
    return 0;
}

int iwl_trans_pnvm_publish(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 0;
}

void iwl_trans_pnvm_doorbell(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    g_doorbell_n++;
}

void iwl_trans_poll(struct iwl_ax211_priv *iwl)
{
    g_poll_calls++;
    if (g_doorbell_n && !g_block_pnvm) {
        iwl->pnvm_complete = 1;
    }
    if (g_poll_calls >= 2) {
        g_test.cmd_status = 1;
    }
}

int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t pay_len, int wait_ms);

int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t pay_len, int wait_ms)
{
    int t;

    if (iwl_trans_send_cmd(iwl, group, id, payload, pay_len) != 0) {
        return -1;
    }

    for (t = 0; t < wait_ms; t++) {
        iwl_trans_poll(iwl);
        if (iwl->cmd_status) {
            return 0;
        }
        lx_mdelay(1);
    }
    iwl->cmd_pending = 0;
    return -1;
}

int g_nvm_fail;

int iwl_mvm_nvm_get_info_mac(struct iwl_ax211_priv *iwl)
{
    if (g_nvm_fail) {
        (void)iwl;
        return -1;
    }
    iwl->nvm_ready = 1;
    return 0;
}

int iwl_mvm_up_minimal(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 0;
}

uint8_t iwl_mvm_scan_rx_ant(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 1;
}

uint8_t iwl_mvm_valid_tx_ant(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 1;
}

uint8_t iwl_mvm_valid_rx_ant(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 1;
}

int iwl_mvm_run_init(struct iwl_ax211_priv *iwl);

int mvm_init_hostcheck(void)
{
    int i;

    memset(&g_test, 0, sizeof(g_test));
    g_test.alive = 1;
    g_test.phy_sku = 0x330018u;
    g_test.n_scan_channels = 21;
    g_sent_n = 0;
    g_fast_ack = 1;
    g_poll_calls = 0;
    g_doorbell_n = 0;
    g_block_pnvm = 0;
    g_init_too_early = 0;

    if (iwl_mvm_run_init(&g_test) != 0) {
        fprintf(stderr, "iwl_mvm_run_init falló\n");
        return -1;
    }
    if (g_doorbell_n != 0) {
        fprintf(stderr, "SKU vacío no debe pisar el doorbell PNVM\n");
        return -1;
    }
    if (!g_test.radio_ready) {
        fprintf(stderr, "radio_ready no marcada tras INIT_COMPLETE\n");
        return -1;
    }

    for (i = 0; i < g_sent_n; i++) {
        if (g_sent[i].group == LEGACY_GROUP &&
            g_sent[i].id == PHY_CONFIGURATION_CMD) {
            fprintf(stderr,
                    "init unificado mandó PHY_CFG grp=0 id=0x%02x (prohibido)\n",
                    g_sent[i].id);
            return -1;
        }
        if (g_sent[i].group == LEGACY_GROUP &&
            g_sent[i].id == TX_ANT_CONFIGURATION_CMD) {
            fprintf(stderr,
                    "init unificado mandó TX_ANT grp=0 id=0x%02x (debe ir en up)\n",
                    g_sent[i].id);
            return -1;
        }
    }

    puts("OK: init unificado sin PHY_CFG ni TX_ANT (van en up_minimal)");

    memset(&g_test, 0, sizeof(g_test));
    g_test.alive = 1;
    g_sent_n = 0;
    g_fast_ack = 1;
    g_nvm_fail = 1;
    if (iwl_mvm_run_init(&g_test) == 0 || g_test.radio_ready || g_test.nvm_ready) {
        fprintf(stderr, "NVM fallido no debe dejar radio/NVM listo\n");
        return -1;
    }
    g_nvm_fail = 0;
    puts("OK: init no anuncia NVM listo si NVM/MAC falla");

    memset(&g_test, 0, sizeof(g_test));
    g_test.alive = 1;
    g_test.sku_id[0] = 0x11111111u;
    g_test.phy_sku = 0x330018u;
    g_test.n_scan_channels = 21;
    g_sent_n = 0;
    g_fast_ack = 1;
    g_poll_calls = 0;
    g_doorbell_n = 0;
    g_block_pnvm = 1;
    g_init_too_early = 0;
    if (iwl_mvm_run_init(&g_test) == 0 || g_sent_n != 0) {
        fprintf(stderr, "sin 0xFE no debe mandar INIT_EXTENDED_CFG\n");
        return -1;
    }
    if (g_doorbell_n != 1) {
        fprintf(stderr, "SKU no vacío debe pisar el doorbell una vez (n=%d)\n",
                g_doorbell_n);
        return -1;
    }
    puts("OK: SKU no vacío espera 0xFE antes de INIT_EXTENDED_CFG");

    memset(&g_test, 0, sizeof(g_test));
    g_test.alive = 1;
    g_test.sku_id[0] = 0x11111111u;
    g_test.phy_sku = 0x330018u;
    g_test.n_scan_channels = 21;
    g_sent_n = 0;
    g_fast_ack = 1;
    g_poll_calls = 0;
    g_doorbell_n = 0;
    g_block_pnvm = 0;
    g_init_too_early = 0;
    if (iwl_mvm_run_init(&g_test) != 0) {
        fprintf(stderr, "iwl_mvm_run_init con PNVM 0xFE falló\n");
        return -1;
    }
    if (g_init_too_early || g_doorbell_n != 1 || g_sent_n < 1 ||
        g_sent[0].group != SYSTEM_GROUP || g_sent[0].id != INIT_EXTENDED_CFG_CMD) {
        fprintf(stderr, "INIT_EXTENDED_CFG no siguió al doorbell (early=%d db=%d n=%d)\n",
                g_init_too_early, g_doorbell_n, g_sent_n);
        return -1;
    }
    puts("OK: doorbell PNVM y luego INIT_EXTENDED_CFG (tras 0xFE)");
    return 0;
}
