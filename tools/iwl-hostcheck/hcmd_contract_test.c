/* R1: banco HCMD con paquetes de tamaño real, contenido NVM completo y casos
 * malformados (truncado, longitud anunciada > recibida, límites RX).
 * C2: rechazo FW, respuesta ajena, notif, timeout tardío, 32+ envíos, dos solicitantes. */
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

/* Errores del propio banco (fixture) frente a errores del parser: los primeros
 * abortan con «fixture:» para no confundirse con un fallo del driver. */
static int g_fixture_err;

static uint8_t g_pkt[IWL_GEN2_RX_SZ];

/* Construye un paquete completo: cabecera de 8 B + payload íntegro, y entrega
 * exactamente los bytes que existen. `announce` permite anunciar en
 * len_n_flags una longitud distinta de la entregada (paquete truncado). */
static void rx_build(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                     uint16_t seq, uint32_t flags, const void *pay,
                     unsigned pay_len, unsigned announce, unsigned deliver)
{
    uint32_t len_n_flags;

    if (pay_len > sizeof(g_pkt) - 8u || deliver > sizeof(g_pkt)) {
        fprintf(stderr, "fixture: paquete de %u B no cabe en %zu\n",
                pay_len, sizeof(g_pkt));
        g_fixture_err = 1;
        return;
    }
    memset(g_pkt, 0, sizeof(g_pkt));
    len_n_flags = (uint32_t)(announce + 4u) | flags;
    memcpy(g_pkt, &len_n_flags, 4);
    g_pkt[4] = id;
    g_pkt[5] = group;
    g_pkt[6] = (uint8_t)seq;
    g_pkt[7] = (uint8_t)(seq >> 8);
    if (pay && pay_len)
        memcpy(g_pkt + 8, pay, pay_len);
    iwl_trans_rx_packet(iwl, g_pkt, deliver);
}

static void rx_resp(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                    uint16_t seq, uint32_t flags, const void *pay, uint16_t pay_len)
{
    rx_build(iwl, group, id, seq, flags, pay, pay_len, pay_len, 8u + pay_len);
}

/* Rellena una respuesta NVM_GET_INFO v4 (468 B) con valores reconocibles. */
static void fill_nvm_v4(struct iwl_nvm_get_info_rsp *rsp)
{
    unsigned i;

    memset(rsp, 0, sizeof(*rsp));
    rsp->general.flags = 0x1;
    rsp->general.nvm_version = 0x1234;
    rsp->general.board_type = 0x07;
    rsp->general.n_hw_addrs = 2;
    rsp->mac_sku.mac_sku_flags = 0x00000003;
    rsp->phy_sku.tx_chains = 0x3;
    rsp->phy_sku.rx_chains = 0x3;
    rsp->regulatory.lar_enabled = 1;
    rsp->regulatory.n_channels = IWL_NUM_CHANNELS;
    for (i = 0; i < IWL_NUM_CHANNELS; i++)
        rsp->regulatory.channel_profile[i] = 0x00010000u | (i + 1u);
}

static int check_nvm_get_info_v4_len(void)
{
    struct iwl_ax211_priv iwl;
    struct iwl_nvm_get_info cmd;
    struct iwl_nvm_get_info_rsp rsp;
    const struct iwl_nvm_get_info_rsp *got;
    uint16_t seq;
    unsigned i;

    if (sizeof(rsp) != 468) {
        fprintf(stderr, "fixture: iwl_nvm_get_info_rsp=%zu B (esperaba 468)\n",
                sizeof(rsp));
        return -1;
    }
    init_priv(&iwl);
    memset(&cmd, 0, sizeof(cmd));
    fill_nvm_v4(&rsp);
    if (iwl_trans_send_cmd(&iwl, REGULATORY_AND_NVM_GROUP, NVM_GET_INFO,
                           &cmd, (uint16_t)sizeof(cmd)) != 0) {
        fprintf(stderr, "NVM_GET_INFO no encoló\n");
        return -1;
    }
    seq = iwl.cmd_pending_seq;
    /* Payload 468 B → len=472; bit 6 del tamaño NO es rechazo FW (run14). */
    rx_resp(&iwl, REGULATORY_AND_NVM_GROUP, NVM_GET_INFO, seq, 0, &rsp,
            (uint16_t)sizeof(rsp));
    if (g_fixture_err)
        return -1;
    if (!iwl.cmd_status || iwl.cmd_fw_err) {
        fprintf(stderr, "NVM_GET_INFO 468 B marcado como error FW\n");
        return -1;
    }
    if (iwl.cmd_resp_len != 468 || iwl.cmd_resp_wire_len != 468 ||
        iwl.cmd_resp_trunc) {
        fprintf(stderr, "NVM_GET_INFO resp=%u wire=%u trunc=%u (esperaba 468/468/0)\n",
                (unsigned)iwl.cmd_resp_len, (unsigned)iwl.cmd_resp_wire_len,
                (unsigned)iwl.cmd_resp_trunc);
        return -1;
    }
    /* Contenido completo, no solo la longitud. */
    if (memcmp(iwl.cmd_resp, &rsp, sizeof(rsp)) != 0) {
        fprintf(stderr, "NVM_GET_INFO payload alterado en la copia\n");
        return -1;
    }
    got = (const struct iwl_nvm_get_info_rsp *)iwl.cmd_resp;
    if (got->general.nvm_version != 0x1234 || got->general.n_hw_addrs != 2 ||
        got->phy_sku.tx_chains != 0x3 || got->phy_sku.rx_chains != 0x3 ||
        got->regulatory.lar_enabled != 1 ||
        got->regulatory.n_channels != IWL_NUM_CHANNELS) {
        fprintf(stderr, "NVM_GET_INFO cabecera/phy/regulatory no coinciden\n");
        return -1;
    }
    for (i = 0; i < IWL_NUM_CHANNELS; i++) {
        if (got->regulatory.channel_profile[i] != (0x00010000u | (i + 1u))) {
            fprintf(stderr, "NVM_GET_INFO canal %u = 0x%08x\n", i,
                    (unsigned)got->regulatory.channel_profile[i]);
            return -1;
        }
    }
    if (iwl.rx_trunc_drop != 0) {
        fprintf(stderr, "NVM_GET_INFO válido contado como truncado\n");
        return -1;
    }
    puts("OK: NVM_GET_INFO v4 (468 B, len=472) íntegro y sin falso rechazo FW");
    return 0;
}

/* Longitud anunciada > recibida, payload truncado y paquete sin cabecera:
 * el driver los descarta sin leer fuera del buffer y sin cerrar el pending. */
static int check_malformed_rx(void)
{
    struct iwl_ax211_priv iwl;
    struct iwl_nvm_get_info cmd;
    struct iwl_nvm_get_info_rsp rsp;
    uint16_t seq;
    uint32_t drops;

    init_priv(&iwl);
    memset(&cmd, 0, sizeof(cmd));
    fill_nvm_v4(&rsp);
    if (iwl_trans_send_cmd(&iwl, REGULATORY_AND_NVM_GROUP, NVM_GET_INFO,
                           &cmd, (uint16_t)sizeof(cmd)) != 0)
        return -1;
    seq = iwl.cmd_pending_seq;

    /* Anuncia 468 B de payload pero solo llegan 100 B (8 + 92). */
    rx_build(&iwl, REGULATORY_AND_NVM_GROUP, NVM_GET_INFO, seq, 0, &rsp, 92,
             468, 100);
    if (g_fixture_err)
        return -1;
    if (iwl.cmd_status || !iwl.cmd_pending) {
        fprintf(stderr, "respuesta truncada cerró el HCMD\n");
        return -1;
    }
    if (iwl.rx_trunc_drop != 1) {
        fprintf(stderr, "truncado no contabilizado (drop=%u)\n",
                (unsigned)iwl.rx_trunc_drop);
        return -1;
    }

    /* Cabecera incompleta (5 B): descartado sin tocar el pending. */
    rx_build(&iwl, REGULATORY_AND_NVM_GROUP, NVM_GET_INFO, seq, 0, NULL, 0, 0, 5);
    if (iwl.cmd_status || !iwl.cmd_pending || iwl.rx_trunc_drop != 2) {
        fprintf(stderr, "paquete sin cabecera no descartado (drop=%u)\n",
                (unsigned)iwl.rx_trunc_drop);
        return -1;
    }

    /* Solo cabecera, payload 0 anunciado: respuesta válida y vacía. */
    rx_build(&iwl, REGULATORY_AND_NVM_GROUP, NVM_GET_INFO, seq, 0, NULL, 0, 0, 8);
    if (!iwl.cmd_status || iwl.cmd_pending) {
        fprintf(stderr, "respuesta vacía legítima no completó\n");
        return -1;
    }
    if (iwl.cmd_resp_len != 0 || iwl.cmd_resp_trunc) {
        fprintf(stderr, "respuesta vacía con resp_len=%u trunc=%u\n",
                (unsigned)iwl.cmd_resp_len, (unsigned)iwl.cmd_resp_trunc);
        return -1;
    }
    drops = iwl.rx_trunc_drop;
    if (drops != 2) {
        fprintf(stderr, "respuesta vacía contada como truncada\n");
        return -1;
    }
    puts("OK: truncado y cabecera corta se descartan; vacío legítimo completa");
    return 0;
}

/* Límite RX: payload máximo de un RB y respuesta mayor que cmd_resp[]. */
static int check_rx_limits(void)
{
    struct iwl_ax211_priv iwl;
    static uint8_t big[IWL_GEN2_RX_SZ - 8u];
    uint8_t dummy = 0;
    unsigned i;
    uint16_t seq;

    for (i = 0; i < sizeof(big); i++)
        big[i] = (uint8_t)(i * 7u + 1u);

    init_priv(&iwl);
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, &dummy, 1) != 0)
        return -1;
    seq = iwl.cmd_pending_seq;
    rx_build(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, seq, 0, big,
             (unsigned)sizeof(big), (unsigned)sizeof(big), IWL_GEN2_RX_SZ);
    if (g_fixture_err)
        return -1;
    if (!iwl.cmd_status || iwl.cmd_pending) {
        fprintf(stderr, "payload máximo de RB no completó\n");
        return -1;
    }
    if (iwl.cmd_resp_wire_len != (uint16_t)sizeof(big)) {
        fprintf(stderr, "wire_len=%u (esperaba %zu)\n",
                (unsigned)iwl.cmd_resp_wire_len, sizeof(big));
        return -1;
    }
    if (iwl.cmd_resp_len != (uint16_t)sizeof(iwl.cmd_resp) || !iwl.cmd_resp_trunc) {
        fprintf(stderr, "resp_len=%u trunc=%u (esperaba %zu/1)\n",
                (unsigned)iwl.cmd_resp_len, (unsigned)iwl.cmd_resp_trunc,
                sizeof(iwl.cmd_resp));
        return -1;
    }
    if (memcmp(iwl.cmd_resp, big, sizeof(iwl.cmd_resp)) != 0) {
        fprintf(stderr, "prefijo copiado no coincide\n");
        return -1;
    }
    if (iwl.rx_trunc_drop != 0) {
        fprintf(stderr, "payload máximo contado como truncado\n");
        return -1;
    }
    puts("OK: límite RX (4088 B) copia 512 B y marca trunc, sin desbordar");
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

/* Timeout: el slot queda envenenado, la cola bloqueada y una respuesta tardía
 * no completa nada. Solo iwl_trans_recover() + recarga de FW la reabre. */
static int check_timeout_late(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t dummy = 0;
    uint16_t old_seq;
    unsigned slot;

    init_priv(&iwl);
    if (iwl_trans_send_cmd_wait(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD,
                                &dummy, 1, 0) == 0) {
        fprintf(stderr, "wait 0 ms no debe tener éxito\n");
        return -1;
    }
    old_seq = iwl.cmd_pending_seq;
    slot = SEQ_TO_INDEX(old_seq) % IWL_CMD_QUEUE_SIZE;
    if (iwl.cmd_pending) {
        fprintf(stderr, "timeout debe soltar pending\n");
        return -1;
    }
    if (iwl.cmd_slot_state[slot] != IWL_SLOT_POISON || iwl.cmd_poisoned != 1) {
        fprintf(stderr, "timeout no envenenó el slot %u (estado %u)\n",
                slot, (unsigned)iwl.cmd_slot_state[slot]);
        return -1;
    }
    if (!iwl_trans_needs_recover(&iwl) || iwl.mvm_up_done) {
        fprintf(stderr, "timeout no marcó recuperación / no paró MVM\n");
        return -1;
    }
    /* Sin recuperar, nada de reciclar DMA todavía expuesto al FW. */
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, NVM_GET_INFO, &dummy, 1) == 0) {
        fprintf(stderr, "envío admitido con la cola bloqueada\n");
        return -1;
    }
    /* Respuesta tardía del comando expirado: ni completa ni libera el slot. */
    rx_resp(&iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, old_seq, 0, NULL, 0);
    if (iwl.cmd_status || iwl.cmd_pending) {
        fprintf(stderr, "respuesta tardía completó un comando\n");
        return -1;
    }
    if (iwl.cmd_slot_state[slot] != IWL_SLOT_POISON) {
        fprintf(stderr, "respuesta tardía liberó el slot envenenado\n");
        return -1;
    }

    /* Recuperación: el transporte se reinicia (alive=0) y la cola queda libre. */
    if (iwl_trans_recover(&iwl) != 0)
        return -1;
    if (iwl_trans_needs_recover(&iwl) || iwl.alive || iwl.cmd_poisoned) {
        fprintf(stderr, "recover no dejó el transporte reiniciable\n");
        return -1;
    }
    if (iwl_trans_cmd_space(&iwl) != IWL_CMD_QUEUE_SIZE - 1u) {
        fprintf(stderr, "recover no liberó la cola (espacio=%u)\n",
                iwl_trans_cmd_space(&iwl));
        return -1;
    }
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, NVM_GET_INFO, &dummy, 1) == 0) {
        fprintf(stderr, "se admitió comando sin ALIVE tras recover\n");
        return -1;
    }
    /* Recarga de FW (iwl_ax211_start_firmware) → ALIVE de nuevo. */
    iwl.alive = 1;
    if (iwl_trans_send_cmd(&iwl, SYSTEM_GROUP, NVM_GET_INFO, &dummy, 1) != 0) {
        fprintf(stderr, "tras recover + ALIVE no se pudo reenviar\n");
        return -1;
    }
    if (!iwl.cmd_pending) {
        fprintf(stderr, "nuevo pending perdido\n");
        return -1;
    }
    rx_resp(&iwl, SYSTEM_GROUP, NVM_GET_INFO, iwl.cmd_pending_seq, 0, NULL, 0);
    if (!iwl.cmd_status || iwl.cmd_pending) {
        fprintf(stderr, "el comando posterior a recover no completó\n");
        return -1;
    }
    puts("OK: timeout envenena y bloquea; recover + ALIVE reabre la cola");
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
    if (hdr[0] != TX_ANT_CONFIGURATION_CMD || hdr[1] != LONG_GROUP) {
        fprintf(stderr, "TX_ANT cabecera cmd/grp incorrecta (DEF_ID → grp=1)\n");
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
    puts("OK: SF async no cierra pending; TX_ANT DEF_ID grp=1 + 4 B payload");
    return 0;
}

int main(void)
{
    if (check_nvm_get_info_v4_len() != 0)
        return 1;
    if (check_malformed_rx() != 0)
        return 1;
    if (check_rx_limits() != 0)
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
    if (g_fixture_err) {
        fprintf(stderr, "fixture: el banco construyó paquetes inválidos\n");
        return 1;
    }
    return 0;
}
