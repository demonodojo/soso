/* Host: contrato DMA del anillo RX por generación y camino de datos 802.11.
 *
 * Fija los cuatro defectos que la investigación del 15/09/2026 reprodujo con
 * sondas desechables (`target/wifi-investigation-2026-09-15/rx-audit.c`):
 *
 *   1. El descriptor de buffer libre de AX211 mide 16 B con el `rbid` fuera
 *      de la dirección, no 8 B con la dirección sola.
 *   2. El descriptor completado de AX211 mide 32 B con el `rbid` en el
 *      offset 4, no 2 B con un índice directo.
 *   3. El descriptor que precede a la MPDU son 48 B (AX200) o 56 (AX211),
 *      nunca los 4 de `iwl_rx_mpdu_res_start`.
 *   4. Las MPDU de datos tienen que llegar convertidas a Ethernet, y EAPOL
 *      por una cola distinta de la de la pila IP.
 *
 * Nada de esto acredita hardware: son contratos de formato y de conversión.
 */
#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "iwl_test_frames.h"

static uint32_t g_mmio_stub[IWL_TEST_MMIO_WORDS];
static unsigned g_data_rx;
static unsigned g_eapol_rx;
static uint8_t g_last_data[2048];
static int g_last_data_len;
static uint8_t g_last_eapol[2048];
static int g_last_eapol_len;

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
    if (group == DATA_PATH_GROUP && cmd == SCD_QUEUE_CONFIG_CMD)
        return 3;
    return 0;
}

void iwl_ax211_deliver_rx(const uint8_t *data, int len)
{
    g_data_rx++;
    g_last_data_len = len;
    if (len > 0 && len <= (int)sizeof(g_last_data))
        memcpy(g_last_data, data, (size_t)len);
}

void iwl_ax211_deliver_eapol(const uint8_t *data, int len)
{
    g_eapol_rx++;
    g_last_eapol_len = len;
    if (len > 0 && len <= (int)sizeof(g_last_eapol))
        memcpy(g_last_eapol, data, (size_t)len);
}

void lx_iwlwifi_set_alive(int alive) { (void)alive; }
void iwl_ax211_add_bss(const struct iwl_ax211_bss *bss) { (void)bss; }

void iwl_mvm_rx_scan_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
{
    (void)iwl;
    (void)frame;
    (void)len;
}

void iwl_mvm_rx_mlme_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
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

int iwl_fw_upload_sections(struct iwl_ax211_priv *iwl, struct iwl_context_info_dram *dram)
{
    (void)iwl;
    (void)dram;
    return 0;
}

#include "iwl_fw_pnvm_weak.c"
#include "iwl_trans.c"

static int fails;

static void check(int ok, const char *label)
{
    printf("%s: %s\n", ok ? "PASS" : "FAIL", label);
    if (!ok)
        fails++;
}

static const uint8_t STA_MAC[6] = { 0x02, 0x11, 0x22, 0x33, 0x44, 0x55 };
static const uint8_t AP_MAC[6] = { 0x06, 0xaa, 0xbb, 0xcc, 0xdd, 0xee };
static const uint8_t PEER_MAC[6] = { 0x0a, 0x01, 0x02, 0x03, 0x04, 0x05 };

static void priv_init(struct iwl_ax211_priv *iwl, int gen3)
{
    memset(iwl, 0, sizeof(*iwl));
    iwl->gen3 = gen3;
    iwl->mmio = g_mmio_stub;
    memcpy(iwl->mac, STA_MAC, 6);
    memcpy(iwl->bssid, AP_MAC, 6);
}

/* --- 1 y 2: formato del anillo RX ------------------------------------- */

static void test_free_bd(void)
{
    struct iwl_ax211_priv iwl;
    uint16_t rbid;
    uint64_t addr;

    priv_init(&iwl, 1);
    if (iwl_alloc_queues(&iwl) != 0)
        exit(2);
    memcpy(&rbid, iwl.rx_bd_cpu, 2);
    memcpy(&addr, (const uint8_t *)iwl.rx_bd_cpu + 8, 8);
    check(rbid == 1 && addr == iwl.rx_page_dma,
          "AX211: BD libre de 16 B, rbid@0 = 1, dirección@8");

    priv_init(&iwl, 0);
    if (iwl_alloc_queues(&iwl) != 0)
        exit(2);
    {
        uint64_t bd0;

        memcpy(&bd0, iwl.rx_bd_cpu, 8);
        check(bd0 == (iwl.rx_page_dma | 1u),
              "AX200: BD libre de 8 B, dirección | vid");
    }
}

/* Mete una notificación en el buffer `vid - 1` y la anuncia en la ranura 0. */
static void post_completion(struct iwl_ax211_priv *iwl, uint16_t vid,
                            const uint8_t *body, unsigned body_len)
{
    uint8_t *rb = (uint8_t *)iwl->rx_page_cpu + (size_t)(vid - 1u) * IWL_GEN2_RX_SZ;

    memset(rb, 0, IWL_GEN2_RX_SZ);
    memcpy(rb, body, body_len);
    memset(iwl->used_bd_cpu, 0, IWL_GEN2_RX_N * iwl_rx_cd_size(iwl));
    if (iwl->gen3) {
        struct iwl_rx_completion_desc *cd =
            (struct iwl_rx_completion_desc *)iwl->used_bd_cpu;

        cd[0].rbid = vid;
    } else {
        uint32_t *cd = (uint32_t *)iwl->used_bd_cpu;

        cd[0] = vid;
    }
    iwl->rx_read = 0;
    iwl->rb_stts[0] = 1;
}

static unsigned alive_notif(uint8_t *out)
{
    memset(out, 0, 16);
    out[0] = 4; /* len_n_flags: 4 B de cabecera, sin carga */
    out[4] = UCODE_ALIVE_NTFY;
    out[5] = 0; /* grupo legacy */
    return 8;
}

static void test_completion_desc(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t body[16];
    unsigned n = alive_notif(body);

    priv_init(&iwl, 1);
    if (iwl_alloc_queues(&iwl) != 0)
        exit(2);
    post_completion(&iwl, 2, body, n);
    drain_rx_gen2(&iwl);
    check(iwl.alive, "AX211: completado de 32 B, rbid@4 = 2 → ALIVE");

    priv_init(&iwl, 0);
    if (iwl_alloc_queues(&iwl) != 0)
        exit(2);
    post_completion(&iwl, 2, body, n);
    drain_rx_gen2(&iwl);
    check(iwl.alive, "AX200: completado de 4 B, vid = 2 → ALIVE (control)");
}

static void test_vid_invalido(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t body[16];
    unsigned n = alive_notif(body);

    priv_init(&iwl, 1);
    if (iwl_alloc_queues(&iwl) != 0)
        exit(2);
    post_completion(&iwl, 1, body, n);
    /* VID 0 no designa buffer: ni se procesa ni se recicla. */
    {
        struct iwl_rx_completion_desc *cd =
            (struct iwl_rx_completion_desc *)iwl.used_bd_cpu;
        uint16_t wr = iwl.rx_write;

        cd[0].rbid = 0;
        drain_rx_gen2(&iwl);
        check(!iwl.alive && iwl.rx_vid_drop == 1 && iwl.rx_write == wr,
              "VID 0 se salta sin reciclar un buffer inventado");
    }
    post_completion(&iwl, 1, body, n);
    {
        struct iwl_rx_completion_desc *cd =
            (struct iwl_rx_completion_desc *)iwl.used_bd_cpu;

        cd[0].rbid = IWL_GEN2_RX_N + 1;
        drain_rx_gen2(&iwl);
        check(!iwl.alive && iwl.rx_vid_drop == 2, "VID fuera de rango se salta");
    }
}

/* --- 3 y 4: descriptor de MPDU y camino de datos ---------------------- */

static void test_rx_datapath(int gen3)
{
    struct iwl_ax211_priv iwl;
    uint8_t frame[256];
    uint8_t pkt[512];
    const uint8_t carga[4] = { 0xde, 0xad, 0xbe, 0xef };
    unsigned flen;
    unsigned n;
    const char *chip = gen3 ? "AX211" : "AX200";
    char label[128];

    priv_init(&iwl, gen3);
    iwl.associated = 1;
    g_data_rx = g_eapol_rx = 0;

    flen = iwl_test_data_mpdu(frame, 0x0800, STA_MAC, AP_MAC, PEER_MAC, 0, carga, 4);
    n = iwl_test_rx_packet(pkt, gen3, frame, flen, 0);
    handle_gen2_rx(&iwl, pkt, n);
    snprintf(label, sizeof(label),
             "%s: MPDU de datos → Ethernet en la cola IP", chip);
    check(g_data_rx == 1 && g_eapol_rx == 0 && g_last_data_len == 14 + 4 &&
              memcmp(g_last_data, STA_MAC, 6) == 0 &&
              memcmp(g_last_data + 6, PEER_MAC, 6) == 0 &&
              g_last_data[12] == 0x08 && g_last_data[13] == 0x00 &&
              memcmp(g_last_data + 14, carga, 4) == 0,
          label);

    flen = iwl_test_data_mpdu(frame, ETH_P_EAPOL, STA_MAC, AP_MAC, PEER_MAC, 0, carga, 4);
    n = iwl_test_rx_packet(pkt, gen3, frame, flen, 0);
    handle_gen2_rx(&iwl, pkt, n);
    snprintf(label, sizeof(label), "%s: EAPOL va a su cola, no a la de IP", chip);
    check(g_eapol_rx == 1 && g_data_rx == 1 && g_last_eapol_len == 14 + 4, label);
}

static void test_rx_filtros(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t frame[256];
    uint8_t pkt[512];
    const uint8_t otro_bssid[6] = { 0x06, 0x99, 0x99, 0x99, 0x99, 0x99 };
    const uint8_t bcast[6] = { 0xff, 0xff, 0xff, 0xff, 0xff, 0xff };
    unsigned flen;
    unsigned n;

    priv_init(&iwl, 0);
    iwl.associated = 1;

    g_data_rx = 0;
    flen = iwl_test_data_mpdu(frame, 0x0800, STA_MAC, otro_bssid, PEER_MAC, 0, 0, 0);
    n = iwl_test_rx_packet(pkt, 0, frame, flen, 0);
    handle_gen2_rx(&iwl, pkt, n);
    check(g_data_rx == 0 && iwl.rx_data_drop == 1, "otro BSSID se descarta");

    flen = iwl_test_data_mpdu(frame, 0x0800, PEER_MAC, AP_MAC, PEER_MAC, 0, 0, 0);
    n = iwl_test_rx_packet(pkt, 0, frame, flen, 0);
    handle_gen2_rx(&iwl, pkt, n);
    check(g_data_rx == 0 && iwl.rx_data_drop == 2,
          "unicast dirigido a otra estación se descarta");

    flen = iwl_test_data_mpdu(frame, 0x0800, bcast, AP_MAC, PEER_MAC, 0, 0, 0);
    n = iwl_test_rx_packet(pkt, 0, frame, flen, 0);
    handle_gen2_rx(&iwl, pkt, n);
    check(g_data_rx == 1, "difusión del propio BSS se entrega");
}

static void test_rx_cifrado(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t frame[256];
    uint8_t pkt[512];
    const uint8_t carga[4] = { 1, 2, 3, 4 };
    uint32_t ok = IWL_RX_MPDU_STATUS_SEC_CCM | IWL_RX_MPDU_STATUS_DECRYPTED |
                  IWL_RX_MPDU_STATUS_MIC_OK;
    unsigned flen;
    unsigned n;

    priv_init(&iwl, 0);
    iwl.associated = 1;
    iwl.keys_installed = 1;

    g_data_rx = 0;
    flen = iwl_test_data_mpdu(frame, 0x0800, STA_MAC, AP_MAC, PEER_MAC, 1, carga, 4);
    n = iwl_test_rx_packet(pkt, 0, frame, flen, ok);
    handle_gen2_rx(&iwl, pkt, n);
    check(g_data_rx == 1 && g_last_data_len == 14 + 4 &&
              memcmp(g_last_data + 14, carga, 4) == 0,
          "CCMP descifrada: se salta la cabecera de 8 B y sale la carga");

    flen = iwl_test_data_mpdu(frame, 0x0800, STA_MAC, AP_MAC, PEER_MAC, 1, carga, 4);
    n = iwl_test_rx_packet(pkt, 0, frame, flen, ok & ~IWL_RX_MPDU_STATUS_MIC_OK);
    handle_gen2_rx(&iwl, pkt, n);
    check(g_data_rx == 1, "MIC en falso: la trama no llega a la pila");

    flen = iwl_test_data_mpdu(frame, 0x0800, STA_MAC, AP_MAC, PEER_MAC, 0, carga, 4);
    n = iwl_test_rx_packet(pkt, 0, frame, flen, 0);
    handle_gen2_rx(&iwl, pkt, n);
    check(g_data_rx == 1, "con claves puestas, datos en claro se descartan");

    /* Sin claves todavía (el 4-way en curso) sí se acepta lo no protegido. */
    iwl.keys_installed = 0;
    flen = iwl_test_data_mpdu(frame, 0x0800, STA_MAC, AP_MAC, PEER_MAC, 0, carga, 4);
    n = iwl_test_rx_packet(pkt, 0, frame, flen, 0);
    handle_gen2_rx(&iwl, pkt, n);
    check(g_data_rx == 2, "antes de instalar claves el 4-way puede pasar");
}

static void test_hdrlen(void)
{
    check(iwl_80211_hdrlen(0x0808) == 24, "cabecera de datos simple: 24 B");
    check(iwl_80211_hdrlen(0x0088) == 26, "datos QoS: 26 B");
    check(iwl_80211_hdrlen(0x8088) == 30, "datos QoS con HT Control: 30 B");
    check(iwl_80211_hdrlen(0x0308) == 30, "cuatro direcciones: 30 B");
}

/* --- TX ---------------------------------------------------------------- */

static void test_tx_conversion(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t eth[IWL_MAX_ETH_FRAME];
    uint8_t out[IWL_MAX_ETH_FRAME + 32];
    int n;
    unsigned i;

    priv_init(&iwl, 0);
    memcpy(eth, PEER_MAC, 6);
    memcpy(eth + 6, STA_MAC, 6);
    eth[12] = 0x08;
    eth[13] = 0x06;
    for (i = 14; i < IWL_MAX_ETH_FRAME; i++)
        eth[i] = (uint8_t)i;

    n = iwl_mvm_eth_to_80211(&iwl, eth, 60, out, (int)sizeof(out));
    check(n == 60 + 18, "802.3 → 802.11: +24 de cabecera y +8 de SNAP, −14 de Ethernet");
    check(out[0] == 0x08 && out[1] == 0x01, "ToDS, no FromDS");
    check(memcmp(out + 4, AP_MAC, 6) == 0, "addr1 = BSSID");
    check(memcmp(out + 10, STA_MAC, 6) == 0, "addr2 = estación");
    check(memcmp(out + 16, PEER_MAC, 6) == 0,
          "addr3 = destino Ethernet real, no el BSSID");
    check(out[24] == 0xaa && out[25] == 0xaa && out[26] == 0x03 &&
              out[30] == 0x08 && out[31] == 0x06,
          "LLC/SNAP con el ethertype original");
    check(memcmp(out + 32, eth + 14, 46) == 0, "carga sin la cabecera 802.3");

    n = iwl_mvm_eth_to_80211(&iwl, eth, IWL_MAX_ETH_FRAME, out, (int)sizeof(out));
    check(n == (int)IWL_MAX_ETH_FRAME + 18, "una trama de MTU completo cabe");
    check(iwl_mvm_eth_to_80211(&iwl, eth, 13, out, (int)sizeof(out)) < 0,
          "menos de 14 B no es una trama Ethernet");
    check(iwl_mvm_eth_to_80211(&iwl, eth, 200, out, 64) < 0,
          "si no cabe en el destino, se rechaza");
}

static void test_tx_ring(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t payload[64];
    unsigned enviados = 0;
    unsigned i;

    priv_init(&iwl, 0);
    iwl.alive = 1;
    iwl.mgmt_txq_ready = 1;
    iwl.mgmt_txq_id = 5;
    memset(payload, 0, sizeof(payload));

    check(iwl_trans_tx_space(&iwl) == IWL_MGMT_QUEUE_SIZE - 1u,
          "cola TX vacía: N-1 huecos");
    for (i = 0; i < IWL_MGMT_QUEUE_SIZE; i++)
        if (iwl_trans_tx(&iwl, 5, payload, sizeof(payload)) == 0)
            enviados++;
    check(enviados == IWL_MGMT_QUEUE_SIZE - 1u && iwl.tx_full_drop == 1,
          "sin respuestas del FW la cola se llena y deja de aceptar");

    /* La respuesta del firmware libera hasta ese índice. */
    iwl_trans_tx_reclaim(&iwl, (uint16_t)INDEX_TO_SEQ(0));
    check(iwl_trans_tx_space(&iwl) == 1, "una respuesta libera un hueco");
    check(iwl_trans_tx(&iwl, 5, payload, sizeof(payload)) == 0,
          "con hueco libre vuelve a aceptar");

    /* Una respuesta repetida no puede devolver TFDs en vuelo. */
    iwl_trans_tx_reclaim(&iwl, (uint16_t)INDEX_TO_SEQ(0));
    check(iwl_trans_tx_space(&iwl) == 0, "respuesta repetida no retrocede el consumidor");
}

int main(void)
{
    test_free_bd();
    test_completion_desc();
    test_vid_invalido();
    test_rx_datapath(0);
    test_rx_datapath(1);
    test_rx_filtros();
    test_rx_cifrado();
    test_hdrlen();
    test_tx_conversion();
    test_tx_ring();
    puts(fails ? "contratos de formato: FALLOS" :
                 "contratos de formato y conversión OK (no acredita hardware)");
    return fails ? 1 : 0;
}
