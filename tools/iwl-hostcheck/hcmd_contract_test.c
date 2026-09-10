/* C2: rechazo FW, respuesta ajena, notif, timeout tardío, 32+ envíos, dos solicitantes. */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

static uint8_t g_mcr_pool[IWL_CMD_SLOT_SIZE * IWL_CMD_QUEUE_SIZE];
static uint8_t g_mtr_pool[IWL_TFH_TFD_SIZE * IWL_CMD_QUEUE_SIZE];
static uint32_t g_mmio_stub[0x500];

void lx_printk(const char *fmt, ...) { (void)fmt; }
void lx_mdelay(unsigned int ms) { (void)ms; }
void lx_udelay(unsigned int us) { (void)us; }

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

int iwl_fw_cmd_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    (void)iwl;
    (void)group;
    (void)cmd;
    return 0;
}

void iwl_ax211_deliver_rx(const uint8_t *data, int len)
{
    (void)data;
    (void)len;
}

void lx_iwlwifi_set_alive(int alive) { (void)alive; }

void iwl_ax211_add_bss(const struct iwl_ax211_bss *bss) { (void)bss; }

void iwl_mvm_rx_scan_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
{
    (void)iwl;
    (void)frame;
    (void)len;
}

void iwl_mvm_on_scan_complete(struct iwl_ax211_priv *iwl, uint32_t uid, uint8_t status)
{
    (void)iwl;
    (void)uid;
    (void)status;
}

int iwl_fw_upload_sections(struct iwl_ax211_priv *iwl,
                           struct iwl_context_info_dram *dram)
{
    (void)iwl;
    (void)dram;
    return 0;
}

static void init_priv(struct iwl_ax211_priv *iwl)
{
    memset(iwl, 0, sizeof(*iwl));
    memset(g_mcr_pool, 0, sizeof(g_mcr_pool));
    memset(g_mtr_pool, 0, sizeof(g_mtr_pool));
    iwl->alive = 1;
    iwl->cmd_qid = IWL_MVM_DQA_CMD_QUEUE;
    iwl->mcr_cpu = g_mcr_pool;
    iwl->mtr_cpu = g_mtr_pool;
    iwl->mcr_dma = 0x1000;
    iwl->mmio = g_mmio_stub;
}

static void rx_resp(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                    uint16_t seq, uint32_t flags, const void *pay, uint16_t pay_len)
{
    uint8_t buf[64];
    uint32_t len_n_flags;

    memset(buf, 0, sizeof(buf));
    len_n_flags = (uint32_t)(pay_len + 4) | flags;
    memcpy(buf, &len_n_flags, 4);
    buf[4] = id;
    buf[5] = group;
    buf[6] = (uint8_t)seq;
    buf[7] = (uint8_t)(seq >> 8);
    if (pay && pay_len)
        memcpy(buf + 8, pay, pay_len > 32 ? 32 : pay_len);
    iwl_trans_rx_packet(iwl, buf, 8u + pay_len);
}

static int check_nvm_get_info_v4_len(void)
{
    struct iwl_ax211_priv iwl;
    struct iwl_nvm_get_info cmd;
    uint8_t rsp[512];
    uint16_t seq;

    init_priv(&iwl);
    memset(&cmd, 0, sizeof(cmd));
    memset(rsp, 0xaa, sizeof(rsp));
    if (iwl_trans_send_cmd(&iwl, REGULATORY_AND_NVM_GROUP, NVM_GET_INFO,
                           &cmd, (uint16_t)sizeof(cmd)) != 0) {
        fprintf(stderr, "NVM_GET_INFO no encoló\n");
        return -1;
    }
    seq = iwl.cmd_pending_seq;
    /* Payload 468 B → len=472; bit 6 del tamaño NO es rechazo FW (run14). */
    rx_resp(&iwl, REGULATORY_AND_NVM_GROUP, NVM_GET_INFO, seq, 0, rsp, 468);
    if (!iwl.cmd_status || iwl.cmd_fw_err) {
        fprintf(stderr, "NVM_GET_INFO 468 B marcado como error FW\n");
        return -1;
    }
    if (iwl.cmd_resp_len != 468) {
        fprintf(stderr, "NVM_GET_INFO resp=%u (esperaba 468)\n",
                (unsigned)iwl.cmd_resp_len);
        return -1;
    }
    puts("OK: NVM_GET_INFO v4 (468 B, len=472) no es falso rechazo FW");
    return 0;
}

static int check_foreign_and_notif(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t dummy = 0;

    init_priv(&iwl);
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, &dummy, 1) != 0)
        return -1;

    rx_resp(&iwl, LONG_GROUP, SCAN_REQ_UMAC, iwl.cmd_pending_seq, 0, NULL, 0);
    if (iwl.cmd_status || !iwl.cmd_pending) {
        fprintf(stderr, "respuesta de otro opcode completó HCMD\n");
        return -1;
    }

    rx_resp(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD,
            (uint16_t)(iwl.cmd_pending_seq | SEQ_RX_FRAME), 0, NULL, 0);
    if (iwl.cmd_status) {
        fprintf(stderr, "notificación con índice coincidente completó HCMD\n");
        return -1;
    }

    rx_resp(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, iwl.cmd_pending_seq, 0, NULL, 0);
    if (!iwl.cmd_status || iwl.cmd_pending) {
        fprintf(stderr, "respuesta propia no completó\n");
        return -1;
    }
    puts("OK: respuesta ajena y notif no cierran el pending");
    return 0;
}

static int check_timeout_late(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t dummy = 0;
    uint16_t old_seq;

    init_priv(&iwl);
    if (iwl_trans_send_cmd_wait(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD,
                                &dummy, 1, 0) == 0) {
        fprintf(stderr, "wait 0 ms no debe tener éxito\n");
        return -1;
    }
    old_seq = iwl.cmd_pending_seq;
    if (iwl.cmd_pending) {
        fprintf(stderr, "timeout debe soltar pending\n");
        return -1;
    }
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, NVM_GET_INFO, &dummy, 1) != 0) {
        fprintf(stderr, "segundo envío tras timeout falló\n");
        return -1;
    }
    rx_resp(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, old_seq, 0, NULL, 0);
    if (iwl.cmd_status) {
        fprintf(stderr, "respuesta tardía del cmd anterior completó el nuevo\n");
        return -1;
    }
    if (!iwl.cmd_pending) {
        fprintf(stderr, "nuevo pending perdido\n");
        return -1;
    }
    puts("OK: timeout + respuesta tardía no pisa el comando nuevo");
    return 0;
}

static int check_two_callers_and_wrap(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t dummy = 0;
    int i;

    init_priv(&iwl);
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, &dummy, 1) != 0)
        return -1;
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, NVM_GET_INFO, &dummy, 1) == 0) {
        fprintf(stderr, "segundo solicitante no debe pisar el pending\n");
        return -1;
    }
    rx_resp(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, iwl.cmd_pending_seq, 0, NULL, 0);

    for (i = 0; i < 40; i++) {
        if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, &dummy, 1) != 0) {
            fprintf(stderr, "envío %d falló\n", i);
            return -1;
        }
        rx_resp(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, iwl.cmd_pending_seq, 0, NULL, 0);
        if (!iwl.cmd_status) {
            fprintf(stderr, "envío %d sin complete\n", i);
            return -1;
        }
    }
    puts("OK: dos solicitantes y >32 envíos serializados");
    return 0;
}

static int check_extreme_sizes(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t big[IWL_CMD_SLOT_SIZE];

    init_priv(&iwl);
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, big,
                           (uint16_t)IWL_CMD_SLOT_SIZE) == 0) {
        fprintf(stderr, "payload = slot no debe caber con cabecera\n");
        return -1;
    }
    if (iwl.cmd_pending) {
        fprintf(stderr, "envío rechazado dejó pending\n");
        return -1;
    }
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, big, 0) != 0) {
        fprintf(stderr, "payload 0 debería enviarse\n");
        return -1;
    }
    puts("OK: tamaños extremos no pisan DMA");
    return 0;
}

#define REPLY_SF_CFG_CMD 0xd1

static int check_sf_async_then_tx_ant(void)
{
    struct iwl_ax211_priv iwl;
    struct iwl_sf_cfg_cmd {
        uint32_t state;
        uint32_t watermark[2];
        uint32_t long_delay_timeouts[5][2];
        uint32_t full_on_timeouts[5][2];
    } __attribute__((packed)) sf;
    struct iwl_tx_ant_cfg_cmd ant;
    const uint8_t *hdr;

    init_priv(&iwl);
    memset(&sf, 0, sizeof(sf));
    memset(&ant, 0, sizeof(ant));
    ant.valid = 1;

    if (iwl_trans_send_cmd_async(&iwl, LEGACY_GROUP, REPLY_SF_CFG_CMD, &sf,
                                 (uint16_t)sizeof(sf)) != 0) {
        fprintf(stderr, "SF async no encoló\n");
        return -1;
    }
    if (iwl.cmd_pending) {
        fprintf(stderr, "SF async cerró/dejó pending (Linux CMD_ASYNC no espera 0xd1)\n");
        return -1;
    }

    if (iwl_trans_send_cmd(&iwl, LEGACY_GROUP, TX_ANT_CONFIGURATION_CMD, &ant,
                           (uint16_t)sizeof(ant)) != 0) {
        fprintf(stderr, "TX_ANT sync bloqueado tras SF async\n");
        return -1;
    }
    if (!iwl.cmd_pending) {
        fprintf(stderr, "TX_ANT sync no dejó pending\n");
        return -1;
    }
    hdr = g_mcr_pool + IWL_CMD_SLOT_SIZE; /* segundo slot: SF usó el 0 */
    if (hdr[0] != TX_ANT_CONFIGURATION_CMD || hdr[1] != LEGACY_GROUP) {
        fprintf(stderr, "TX_ANT cabecera cmd/grp incorrecta\n");
        return -1;
    }
    if (*(const uint16_t *)(hdr + 4) != (uint16_t)sizeof(ant)) {
        fprintf(stderr, "TX_ANT length wide=%u (payload %zu)\n",
                (unsigned)*(const uint16_t *)(hdr + 4), sizeof(ant));
        return -1;
    }
    if (sizeof(struct iwl_cmd_header_wide) != 8 || sizeof(ant) != 4) {
        fprintf(stderr, "TX_ANT no es hdr 8 B + payload 4 B\n");
        return -1;
    }
    puts("OK: SF async no cierra pending; TX_ANT 8 B header + 4 B payload");
    return 0;
}

int main(void)
{
    if (check_nvm_get_info_v4_len() != 0)
        return 1;
    if (check_foreign_and_notif() != 0)
        return 1;
    if (check_timeout_late() != 0)
        return 1;
    if (check_two_callers_and_wrap() != 0)
        return 1;
    if (check_extreme_sizes() != 0)
        return 1;
    if (check_sf_async_then_tx_ant() != 0)
        return 1;
    return 0;
}
