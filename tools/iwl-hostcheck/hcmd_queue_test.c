/*
 * R4: propiedad de slots de la cola HCMD y recuperación.
 *
 * Cubre backpressure con >32 async sin consumir, solicitantes intercalados,
 * RX concurrente durante el envío (anillo RX simulado), liberación en orden
 * del DMA y reentrada rechazada.
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

static uint8_t g_mcr_pool[IWL_CMD_SLOT_SIZE * IWL_CMD_QUEUE_SIZE];
static uint8_t g_mtr_pool[IWL_TFH_TFD_SIZE * IWL_CMD_QUEUE_SIZE];
static uint32_t g_mmio_stub[0x2000];

/* Anillo RX simulado: permite que un drenaje entregue paquetes de verdad. */
static uint8_t g_rx_page[IWL_GEN2_RX_N * IWL_GEN2_RX_SZ];
static uint32_t g_rx_used[IWL_GEN2_RX_N];
static uint64_t g_rx_bd[IWL_GEN2_RX_N];
static uint16_t g_rb_stts[8];
static unsigned g_rx_queued;

/* Envío anidado que el stub de RX intenta durante un drenaje. */
static struct iwl_ax211_priv *g_reentry_target;
static int g_reentry_rc;
static int g_reentry_tries;

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

/* Se invoca desde el drenaje RX; aprovecha para intentar un envío anidado. */
void iwl_mvm_on_scan_complete(struct iwl_ax211_priv *iwl, uint32_t uid, uint8_t status)
{
    uint8_t dummy = 0;

    if (!iwl->scan_active || uid != iwl->scan_uid)
        return;
    /* Igual que iwl_mvm_on_scan_complete real, más el envío anidado. */
    iwl->scan_end = (status == IWL_SCAN_OFFLOAD_COMPLETED) ? IWL_SCAN_END_NORMAL
                                                           : IWL_SCAN_END_ABORTED;
    iwl->scan_complete = 1;
    iwl->scan_active = 0;
    if (!g_reentry_target || iwl != g_reentry_target)
        return;
    g_reentry_tries++;
    g_reentry_rc = iwl_trans_send_cmd(iwl, SYSTEM_GROUP, NVM_GET_INFO, &dummy, 1);
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
    memset(g_rx_used, 0, sizeof(g_rx_used));
    memset(g_rb_stts, 0, sizeof(g_rb_stts));
    g_rx_queued = 0;
    g_reentry_target = 0;
    g_reentry_rc = 0;
    g_reentry_tries = 0;
    iwl->alive = 1;
    iwl->cmd_qid = IWL_MVM_DQA_CMD_QUEUE;
    iwl->mcr_cpu = g_mcr_pool;
    iwl->mtr_cpu = g_mtr_pool;
    iwl->mcr_dma = 0x1000;
    iwl->mmio = g_mmio_stub;
}

/* Conecta el anillo RX simulado (ruta gen2: used32 con vid = idx+1). */
static void init_rx_ring(struct iwl_ax211_priv *iwl)
{
    iwl->gen3 = 0;
    iwl->rx_page_cpu = g_rx_page;
    iwl->used_bd_cpu = g_rx_used;
    iwl->rx_bd_cpu = g_rx_bd;
    iwl->rb_stts = g_rb_stts;
    iwl->rx_page_dma = 0x200000;
    iwl->rx_read = 0;
    iwl->rx_write = 0;
}

/* Encola un paquete en el anillo RX sin drenarlo. */
static void rx_queue(uint8_t group, uint8_t id, uint16_t seq,
                     const void *pay, unsigned pay_len)
{
    unsigned idx = g_rx_queued % IWL_GEN2_RX_N;
    uint8_t *page = g_rx_page + (size_t)idx * IWL_GEN2_RX_SZ;
    uint32_t len_n_flags = (uint32_t)(pay_len + 4u);

    memset(page, 0, IWL_GEN2_RX_SZ);
    memcpy(page, &len_n_flags, 4);
    page[4] = id;
    page[5] = group;
    page[6] = (uint8_t)seq;
    page[7] = (uint8_t)(seq >> 8);
    if (pay && pay_len)
        memcpy(page + 8, pay, pay_len);
    g_rx_used[idx] = idx + 1u;
    g_rx_queued++;
    g_rb_stts[0] = (uint16_t)(g_rx_queued % IWL_GEN2_RX_N);
}

static void rx_resp(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                    uint16_t seq, const void *pay, uint16_t pay_len)
{
    uint8_t pkt[IWL_GEN2_RX_SZ];
    uint32_t len_n_flags = (uint32_t)(pay_len + 4u);

    memset(pkt, 0, sizeof(pkt));
    memcpy(pkt, &len_n_flags, 4);
    pkt[4] = id;
    pkt[5] = group;
    pkt[6] = (uint8_t)seq;
    pkt[7] = (uint8_t)(seq >> 8);
    if (pay && pay_len)
        memcpy(pkt + 8, pay, pay_len);
    iwl_trans_rx_packet(iwl, pkt, 8u + pay_len);
}

static unsigned slot_of(uint16_t seq)
{
    return SEQ_TO_INDEX(seq) % IWL_CMD_QUEUE_SIZE;
}

static const uint8_t *slot_hdr(unsigned slot)
{
    return g_mcr_pool + (size_t)slot * IWL_CMD_SLOT_SIZE;
}

/* >32 async sin consumir: backpressure y ninguna sobrescritura de DMA. */
static int check_async_backpressure(void)
{
    struct iwl_ax211_priv iwl;
    uint16_t seqs[IWL_CMD_QUEUE_SIZE];
    unsigned cap = IWL_CMD_QUEUE_SIZE - 1u;
    unsigned i;
    uint8_t pay;

    init_priv(&iwl);
    for (i = 0; i < cap; i++) {
        pay = (uint8_t)(i + 1u);
        if (iwl_trans_send_cmd_async(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD,
                                     &pay, 1) != 0) {
            fprintf(stderr, "async %u falló con cola libre\n", i);
            return -1;
        }
        seqs[i] = iwl.cmd_slot_seq[i];
        if (iwl.cmd_slot_state[i] != IWL_SLOT_ASYNC) {
            fprintf(stderr, "slot %u no quedó en propiedad async\n", i);
            return -1;
        }
    }
    if (iwl_trans_cmd_space(&iwl) != 0) {
        fprintf(stderr, "espacio=%u tras %u async\n",
                iwl_trans_cmd_space(&iwl), cap);
        return -1;
    }
    /* 32.º y siguientes: rechazo, no sobrescritura del slot más antiguo. */
    for (i = 0; i < 5; i++) {
        pay = 0xee;
        if (iwl_trans_send_cmd_async(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD,
                                     &pay, 1) == 0) {
            fprintf(stderr, "envío %u admitido con la cola llena\n", cap + i);
            return -1;
        }
    }
    if (iwl.cmd_backpressure != 5) {
        fprintf(stderr, "backpressure=%u (esperaba 5)\n",
                (unsigned)iwl.cmd_backpressure);
        return -1;
    }
    for (i = 0; i < cap; i++) {
        const uint8_t *h = slot_hdr(i);

        if (h[0] != INIT_EXTENDED_CFG_CMD ||
            h[sizeof(struct iwl_cmd_header_wide)] != (uint8_t)(i + 1u)) {
            fprintf(stderr, "slot %u pisado (payload=0x%02x)\n", i,
                    h[sizeof(struct iwl_cmd_header_wide)]);
            return -1;
        }
        if (iwl.cmd_slot_seq[i] != seqs[i]) {
            fprintf(stderr, "slot %u perdió su secuencia\n", i);
            return -1;
        }
    }

    /* Consumir en orden libera slots; el DMA se recicla solo entonces. */
    for (i = 0; i < 4; i++) {
        rx_resp(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, seqs[i], NULL, 0);
        if (iwl.cmd_slot_state[i] != IWL_SLOT_FREE) {
            fprintf(stderr, "slot %u no liberado tras responder\n", i);
            return -1;
        }
    }
    if (iwl_trans_cmd_space(&iwl) != 4) {
        fprintf(stderr, "espacio=%u tras 4 respuestas\n", iwl_trans_cmd_space(&iwl));
        return -1;
    }
    /* Los 4 huecos se reparten en orden de anillo: 31 (nunca usado) y luego
     * los liberados 0, 1 y 2. El quinto vuelve a dar backpressure. */
    {
        const unsigned esperados[4] = { 31, 0, 1, 2 };
        unsigned k;

        for (k = 0; k < 4; k++) {
            pay = (uint8_t)(0x70u + k);
            if (iwl_trans_send_cmd_async(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD,
                                         &pay, 1) != 0) {
                fprintf(stderr, "no se reutilizó el hueco %u\n", k);
                return -1;
            }
            if (iwl.cmd_slot_state[esperados[k]] != IWL_SLOT_ASYNC ||
                slot_of(iwl.cmd_slot_seq[esperados[k]]) != esperados[k]) {
                fprintf(stderr, "el reenvío %u no aterrizó en el slot %u\n",
                        k, esperados[k]);
                return -1;
            }
            if (slot_hdr(esperados[k])[sizeof(struct iwl_cmd_header_wide)] !=
                (uint8_t)(0x70u + k)) {
                fprintf(stderr, "el slot %u no tiene el payload nuevo\n",
                        esperados[k]);
                return -1;
            }
        }
        if (iwl_trans_send_cmd_async(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD,
                                     &pay, 1) == 0) {
            fprintf(stderr, "quinto envío admitido sin huecos\n");
            return -1;
        }
    }
    puts("OK: 31 async en vuelo, backpressure sin pisar DMA, liberación en orden");
    return 0;
}

/* Respuestas fuera de orden: el DMA no se recicla antes de que le toque. */
static int check_out_of_order_release(void)
{
    struct iwl_ax211_priv iwl;
    uint16_t seq0;
    uint16_t seq1;
    uint16_t seq2;
    uint8_t pay = 1;

    init_priv(&iwl);
    if (iwl_trans_send_cmd_async(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, &pay, 1) != 0 ||
        iwl_trans_send_cmd_async(&iwl, SYSTEM_GROUP, NVM_GET_INFO, &pay, 1) != 0 ||
        iwl_trans_send_cmd_async(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, &pay, 1) != 0)
        return -1;
    seq0 = iwl.cmd_slot_seq[0];
    seq1 = iwl.cmd_slot_seq[1];
    seq2 = iwl.cmd_slot_seq[2];

    rx_resp(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, seq2, NULL, 0);
    if (iwl.cmd_slot_state[2] != IWL_SLOT_DONE || iwl.cmd_read != 0) {
        fprintf(stderr, "respuesta del slot 2 liberó fuera de orden (read=%u)\n",
                (unsigned)iwl.cmd_read);
        return -1;
    }
    rx_resp(&iwl, SYSTEM_GROUP, NVM_GET_INFO, seq1, NULL, 0);
    if (iwl.cmd_slot_state[0] != IWL_SLOT_ASYNC || iwl.cmd_read != 0) {
        fprintf(stderr, "el slot 0 en vuelo se liberó por otro\n");
        return -1;
    }
    rx_resp(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, seq0, NULL, 0);
    if (iwl.cmd_read != 3 || iwl_trans_cmd_space(&iwl) != IWL_CMD_QUEUE_SIZE - 1u) {
        fprintf(stderr, "la cabeza no arrastró los slots respondidos (read=%u)\n",
                (unsigned)iwl.cmd_read);
        return -1;
    }
    puts("OK: respuestas fuera de orden no reciclan DMA antes de tiempo");
    return 0;
}

/* Sync + async intercalados: distinto slot, distinta respuesta. */
static int check_interleaved_owners(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t sync_pay = 0xa1;
    uint8_t async_pay = 0xb2;
    uint8_t resp_sync[4] = { 0xde, 0xad, 0xbe, 0xef };
    uint8_t resp_async[4] = { 0x11, 0x22, 0x33, 0x44 };
    uint16_t sync_seq;
    uint16_t async_seq;

    init_priv(&iwl);
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, NVM_GET_INFO, &sync_pay, 1) != 0)
        return -1;
    sync_seq = iwl.cmd_pending_seq;
    if (iwl_trans_send_cmd_async(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD,
                                 &async_pay, 1) != 0) {
        fprintf(stderr, "async bloqueado por un sync en vuelo\n");
        return -1;
    }
    async_seq = iwl.cmd_slot_seq[1];
    if (slot_of(sync_seq) == slot_of(async_seq)) {
        fprintf(stderr, "sync y async comparten slot\n");
        return -1;
    }
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, NVM_ACCESS_CMD, &sync_pay, 1) == 0) {
        fprintf(stderr, "segundo sync admitido con cmd_resp ocupado\n");
        return -1;
    }

    /* La respuesta del async no toca cmd_resp[] ni cierra el sync. */
    rx_resp(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, async_seq, resp_async, 4);
    if (iwl.cmd_status || !iwl.cmd_pending) {
        fprintf(stderr, "la respuesta async cerró el sync\n");
        return -1;
    }
    if (iwl.cmd_resp_len != 0) {
        fprintf(stderr, "la respuesta async escribió cmd_resp (%u B)\n",
                (unsigned)iwl.cmd_resp_len);
        return -1;
    }
    rx_resp(&iwl, SYSTEM_GROUP, NVM_GET_INFO, sync_seq, resp_sync, 4);
    if (!iwl.cmd_status || iwl.cmd_pending) {
        fprintf(stderr, "el sync no completó con su propia respuesta\n");
        return -1;
    }
    if (iwl.cmd_resp_len != 4 || memcmp(iwl.cmd_resp, resp_sync, 4) != 0) {
        fprintf(stderr, "cmd_resp no es la respuesta del sync\n");
        return -1;
    }
    if (iwl_trans_cmd_space(&iwl) != IWL_CMD_QUEUE_SIZE - 1u) {
        fprintf(stderr, "cola no vaciada tras ambas respuestas\n");
        return -1;
    }
    puts("OK: sync y async intercalados no comparten slot ni respuesta");
    return 0;
}

/* RX concurrente durante el envío: el drenaje entrega y la reentrada se
 * rechaza sin tocar el anillo de comandos. */
static int check_rx_during_send(void)
{
    struct iwl_ax211_priv iwl;
    struct iwl_umac_scan_complete done;
    uint8_t pay = 0x5a;
    uint16_t seq;
    uint16_t write_before;

    init_priv(&iwl);
    init_rx_ring(&iwl);
    iwl.scan_active = 1;
    iwl.scan_uid = 7;
    g_reentry_target = &iwl;

    memset(&done, 0, sizeof(done));
    done.uid = 7;
    done.status = IWL_SCAN_OFFLOAD_COMPLETED;
    rx_queue(LONG_GROUP, SCAN_COMPLETE_UMAC, SEQ_RX_FRAME, &done, sizeof(done));

    /* El envío drena el RX: el stub de scan-complete intenta un envío anidado. */
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, NVM_GET_INFO, &pay, 1) != 0) {
        fprintf(stderr, "envío con RX pendiente falló\n");
        return -1;
    }
    seq = iwl.cmd_pending_seq;
    write_before = iwl.cmd_write;
    if (g_reentry_tries == 0) {
        fprintf(stderr, "fixture: el drenaje no llamó al stub de scan-complete\n");
        return -1;
    }
    if (g_reentry_rc == 0) {
        fprintf(stderr, "envío reentrante admitido durante el drenaje\n");
        return -1;
    }
    if (iwl.cmd_reentry_reject == 0) {
        fprintf(stderr, "reentrada no contabilizada\n");
        return -1;
    }
    if (iwl.cmd_write != write_before || iwl.cmd_pending_seq != seq) {
        fprintf(stderr, "la reentrada movió el anillo de comandos\n");
        return -1;
    }
    if (!iwl.scan_complete || iwl.scan_end != IWL_SCAN_END_NORMAL) {
        fprintf(stderr, "el drenaje no procesó SCAN_COMPLETE (end=%d)\n",
                iwl.scan_end);
        return -1;
    }
    if (iwl.in_trans) {
        fprintf(stderr, "in_trans quedó activo tras el envío\n");
        return -1;
    }

    /* La respuesta del comando llega por el anillo RX, no inyectada. */
    rx_queue(SYSTEM_GROUP, NVM_GET_INFO, seq, &pay, 1);
    iwl_trans_poll(&iwl);
    if (!iwl.cmd_status || iwl.cmd_pending) {
        fprintf(stderr, "la respuesta por anillo RX no completó el comando\n");
        return -1;
    }
    if (iwl.cmd_slot_state[slot_of(seq)] != IWL_SLOT_FREE) {
        fprintf(stderr, "slot no liberado tras la respuesta del anillo\n");
        return -1;
    }
    puts("OK: RX concurrente entrega notificaciones y rechaza la reentrada");
    return 0;
}

/* Tras timeout, la cola no admite nada hasta recuperar; el scan se repite
 * después del reinicio sin reciclar el DMA envenenado. */
static int check_recover_then_scan(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t pay = 1;
    uint16_t bad_seq;
    unsigned bad_slot;

    init_priv(&iwl);
    if (iwl_trans_send_cmd_async(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, &pay, 1) != 0)
        return -1;
    if (iwl_trans_send_cmd_wait(&iwl, LONG_GROUP, SCAN_REQ_UMAC, &pay, 1, 0) == 0) {
        fprintf(stderr, "wait 0 ms no debe tener éxito\n");
        return -1;
    }
    bad_seq = iwl.cmd_pending_seq;
    bad_slot = slot_of(bad_seq);
    if (iwl.cmd_slot_state[bad_slot] != IWL_SLOT_POISON) {
        fprintf(stderr, "el slot expirado no quedó envenenado\n");
        return -1;
    }
    /* Incluso liberando el async previo, el envenenado bloquea la cabeza. */
    rx_resp(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, iwl.cmd_slot_seq[0], NULL, 0);
    if (iwl.cmd_slot_state[0] != IWL_SLOT_FREE) {
        fprintf(stderr, "el async previo no se liberó\n");
        return -1;
    }
    if (iwl_trans_send_cmd(&iwl, LONG_GROUP, SCAN_REQ_UMAC, &pay, 1) == 0) {
        fprintf(stderr, "scan admitido con la cola bloqueada\n");
        return -1;
    }
    if (iwl.cmd_slot_state[bad_slot] != IWL_SLOT_POISON) {
        fprintf(stderr, "el slot envenenado se reutilizó\n");
        return -1;
    }

    iwl_trans_recover(&iwl);
    iwl.alive = 1; /* recarga de firmware */
    if (iwl_trans_send_cmd(&iwl, LONG_GROUP, SCAN_REQ_UMAC, &pay, 1) != 0) {
        fprintf(stderr, "scan no repetible tras recover\n");
        return -1;
    }
    rx_resp(&iwl, LONG_GROUP, SCAN_REQ_UMAC, iwl.cmd_pending_seq, NULL, 0);
    if (!iwl.cmd_status) {
        fprintf(stderr, "el scan posterior a recover no completó\n");
        return -1;
    }
    if (iwl.cmd_recover != 1) {
        fprintf(stderr, "recover=%u (esperaba 1)\n", (unsigned)iwl.cmd_recover);
        return -1;
    }
    puts("OK: timeout bloquea la cola; recover permite repetir el scan");
    return 0;
}

int main(void)
{
    if (check_async_backpressure() != 0)
        return 1;
    if (check_out_of_order_release() != 0)
        return 1;
    if (check_interleaved_owners() != 0)
        return 1;
    if (check_rx_during_send() != 0)
        return 1;
    if (check_recover_then_scan() != 0)
        return 1;
    return 0;
}
