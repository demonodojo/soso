/* Host: los anillos RX y TX bajo carga, contra un firmware modelado.
 *
 * Los demás bancos comprueban **formatos**: un descriptor armado a mano entra y
 * se mira qué sale. Eso no ve la clase de fallo que aparece cuando hay dos
 * partes avanzando índices a la vez sobre memoria compartida: reciclar un
 * buffer que el firmware aún tiene, publicar dos veces el mismo, perder uno en
 * la vuelta del anillo, o reutilizar un TFD sin confirmar.
 *
 * Aquí el firmware es un modelo que **consume el anillo con el mismo formato
 * que escribe el driver** y mantiene su propia contabilidad. Cada vuelta
 * comprueba invariantes de propiedad y conservación:
 *
 *   - Un VID leído del anillo de BD libres tiene que estar libre de verdad.
 *     Si el driver lo publicó dos veces, o publicó uno que el firmware todavía
 *     tenía, salta aquí.
 *   - La dirección del BD y su `rbid` describen el mismo buffer.
 *   - Los 32 buffers están siempre en exactamente uno de tres sitios (anillo,
 *     firmware, completado sin drenar): ni se duplican ni se pierden.
 *   - El firmware nunca ve un buffer más allá del último doorbell.
 *   - Cada paquete entregado llega una vez y en orden: sin huecos ni repetidos.
 *   - En TX, el driver nunca escribe en un TFD que el firmware no ha
 *     reconocido, y el hueco que declara coincide con lo que hay en vuelo.
 *
 * El modelo **no es el hardware**: está escrito desde la misma lectura de la
 * especificación que el driver, así que no puede descubrir que el formato es
 * otro. Lo que sí descubre es que las dos mitades del driver se contradigan
 * entre sí bajo carga, que es lo que ningún banco de formato alcanza.
 *
 * `./ring_soak [semilla] [vueltas]` — la semilla se imprime para repetir.
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
static uint32_t g_entregados;
static uint32_t g_ultimo_id;
static int g_orden_roto;

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

/* Cada paquete lleva su número de secuencia en la carga: si el anillo pierde,
 * repite o desordena algo, se ve aquí y no en un contador agregado. */
void iwl_ax211_deliver_rx(const uint8_t *data, int len)
{
    uint32_t id;

    if (len < 14 + 4) {
        g_orden_roto = 1;
        return;
    }
    memcpy(&id, data + 14, 4);
    if (id != g_ultimo_id + 1u)
        g_orden_roto = 1;
    g_ultimo_id = id;
    g_entregados++;
}

void iwl_ax211_deliver_eapol(const uint8_t *data, int len)
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

/* --- Modelo de firmware para el anillo RX ------------------------------ */

enum buf_estado {
    BUF_EN_ANILLO,  /* publicado por el driver, sin consumir */
    BUF_EN_FW,      /* el firmware lo tiene */
    BUF_COMPLETADO, /* anunciado al driver, sin drenar */
};

struct fw_rx {
    enum buf_estado estado[IWL_GEN2_RX_N + 1]; /* indexado por VID (1..N) */
    unsigned bd_next;                          /* siguiente BD a consumir */
    uint16_t propios[IWL_GEN2_RX_N];           /* VIDs en poder del FW, FIFO */
    unsigned n_propios;
    unsigned cd_write;      /* productor del anillo de completados */
    unsigned pendientes;    /* completados sin drenar */
    uint32_t id_siguiente;  /* numeración de los paquetes */
    const char *error;
};

/* Distancia circular de `a` a `b` en el anillo de N ranuras. */
static unsigned dist(unsigned a, unsigned b)
{
    return (b + IWL_GEN2_RX_N - a) % IWL_GEN2_RX_N;
}

/* El firmware lee un BD del anillo de libres, con el formato de la generación. */
static int fw_toma_bd(struct iwl_ax211_priv *iwl, struct fw_rx *fw)
{
    uint16_t vid;
    uint64_t addr;
    unsigned slot = fw->bd_next % IWL_GEN2_RX_N;

    if (iwl->gen3) {
        const struct iwl_rx_transfer_desc *bd =
            (const struct iwl_rx_transfer_desc *)iwl->rx_bd_cpu;

        vid = bd[slot].rbid;
        addr = bd[slot].addr;
    } else {
        const uint64_t *bd = (const uint64_t *)iwl->rx_bd_cpu;

        vid = (uint16_t)(bd[slot] & 0xfffu);
        addr = bd[slot] & ~(uint64_t)0xfffu;
    }
    if (vid == 0 || vid > IWL_GEN2_RX_N) {
        fw->error = "BD con un VID que no designa buffer";
        return -1;
    }
    if (addr != iwl->rx_page_dma + (uint64_t)(vid - 1u) * IWL_GEN2_RX_SZ) {
        fw->error = "la dirección del BD y su VID describen buffers distintos";
        return -1;
    }
    if (fw->estado[vid] != BUF_EN_ANILLO) {
        fw->error = "el driver publicó un buffer que el firmware aún tenía";
        return -1;
    }
    fw->estado[vid] = BUF_EN_FW;
    fw->propios[fw->n_propios++] = vid;
    fw->bd_next = (fw->bd_next + 1u) % IWL_GEN2_RX_N;
    return 0;
}

/* El firmware entrega un paquete en su buffer más antiguo. */
static int fw_completa(struct iwl_ax211_priv *iwl, struct fw_rx *fw)
{
    uint8_t frame[64];
    uint8_t pkt[8 + IWL_RX_DESC_SIZE_V3 + 64];
    uint16_t vid;
    unsigned flen;
    unsigned n;
    uint8_t *rb;
    unsigned i;
    uint32_t id;

    if (!fw->n_propios)
        return 0;
    /* El anillo de completados tiene N ranuras y el driver distingue vacío de
     * lleno por igualdad: con N pendientes daría la vuelta y parecería vacío.
     * El hardware tampoco puede pasar de ahí, porque no tiene más buffers. */
    if (fw->pendientes >= IWL_GEN2_RX_N - 1u)
        return 0;

    vid = fw->propios[0];
    for (i = 1; i < fw->n_propios; i++)
        fw->propios[i - 1] = fw->propios[i];
    fw->n_propios--;

    id = ++fw->id_siguiente;
    flen = iwl_test_data_mpdu(frame, 0x0800, STA_MAC, AP_MAC, PEER_MAC, 0,
                              (const uint8_t *)&id, 4);
    n = iwl_test_rx_packet(pkt, iwl->gen3, frame, flen, 0);

    rb = (uint8_t *)iwl->rx_page_cpu + (size_t)(vid - 1u) * IWL_GEN2_RX_SZ;
    memset(rb, 0, IWL_GEN2_RX_SZ);
    memcpy(rb, pkt, n);

    if (iwl->gen3) {
        struct iwl_rx_completion_desc *cd =
            (struct iwl_rx_completion_desc *)iwl->used_bd_cpu;

        memset(&cd[fw->cd_write], 0, sizeof(cd[fw->cd_write]));
        cd[fw->cd_write].rbid = vid;
    } else {
        uint32_t *cd = (uint32_t *)iwl->used_bd_cpu;

        cd[fw->cd_write] = vid;
    }
    fw->estado[vid] = BUF_COMPLETADO;
    fw->cd_write = (fw->cd_write + 1u) % IWL_GEN2_RX_N;
    fw->pendientes++;
    iwl->rb_stts[0] = (uint16_t)fw->cd_write;
    return 1;
}

/* Tras drenar, los BD que el driver acaba de republicar vuelven al anillo. */
static int registra_republicados(struct iwl_ax211_priv *iwl, struct fw_rx *fw,
                                 uint16_t write_antes, unsigned drenados)
{
    unsigned k;

    for (k = 0; k < drenados; k++) {
        unsigned slot = (write_antes + k) % IWL_GEN2_RX_N;
        uint16_t vid;

        if (iwl->gen3) {
            const struct iwl_rx_transfer_desc *bd =
                (const struct iwl_rx_transfer_desc *)iwl->rx_bd_cpu;

            vid = bd[slot].rbid;
        } else {
            const uint64_t *bd = (const uint64_t *)iwl->rx_bd_cpu;

            vid = (uint16_t)(bd[slot] & 0xfffu);
        }
        if (vid == 0 || vid > IWL_GEN2_RX_N || fw->estado[vid] != BUF_COMPLETADO) {
            fw->error = "el driver republicó un buffer que no acababa de drenar";
            return -1;
        }
        fw->estado[vid] = BUF_EN_ANILLO;
        fw->pendientes--;
    }
    return 0;
}

/* Conservación: los N buffers están siempre repartidos, sin perderse. */
static int conservacion_ok(struct fw_rx *fw)
{
    unsigned anillo = 0, en_fw = 0, comp = 0;
    unsigned v;

    for (v = 1; v <= IWL_GEN2_RX_N; v++) {
        switch (fw->estado[v]) {
        case BUF_EN_ANILLO:
            anillo++;
            break;
        case BUF_EN_FW:
            en_fw++;
            break;
        case BUF_COMPLETADO:
            comp++;
            break;
        }
    }
    if (anillo + en_fw + comp != IWL_GEN2_RX_N) {
        fw->error = "se ha perdido o duplicado un buffer";
        return 0;
    }
    if (en_fw != fw->n_propios || comp != fw->pendientes) {
        fw->error = "la contabilidad del modelo no cuadra con los estados";
        return 0;
    }
    return 1;
}

static void soak_rx(int gen3, uint32_t semilla, unsigned vueltas)
{
    struct iwl_ax211_priv iwl;
    struct fw_rx fw;
    uint32_t rnd = semilla;
    unsigned v;
    unsigned i;
    char label[160];
    unsigned max_vistos = 0;

    memset(&iwl, 0, sizeof(iwl));
    iwl.gen3 = gen3;
    iwl.mmio = g_mmio_stub;
    iwl.associated = 1;
    memcpy(iwl.mac, STA_MAC, 6);
    memcpy(iwl.bssid, AP_MAC, 6);
    if (iwl_alloc_queues(&iwl) != 0)
        exit(2);

    memset(&fw, 0, sizeof(fw));
    for (v = 1; v <= IWL_GEN2_RX_N; v++)
        fw.estado[v] = BUF_EN_ANILLO;
    g_entregados = 0;
    g_ultimo_id = 0;
    g_orden_roto = 0;

    /* Doorbell inicial: el driver anuncia hasta `rx_write & ~7`. */
    iwl_write32(&iwl, RFH_Q0_FRBDCB_WIDX_TRG, (uint32_t)(iwl.rx_write & ~7u));

    for (i = 0; i < vueltas && !fw.error; i++) {
        unsigned widx = g_mmio_stub[RFH_Q0_FRBDCB_WIDX_TRG / 4] & 0xffffu;
        unsigned disponibles = dist(fw.bd_next, widx % IWL_GEN2_RX_N);
        unsigned toma = iwl_test_rand(&rnd) % 5u;
        unsigned completa = iwl_test_rand(&rnd) % 5u;
        uint16_t write_antes;
        unsigned entregados_antes;
        unsigned drenados;

        /* El firmware coge buffers, pero nunca más allá del doorbell. */
        if (toma > disponibles)
            toma = disponibles;
        while (toma-- && !fw.error)
            if (fw_toma_bd(&iwl, &fw) != 0)
                break;
        if (fw.error)
            break;
        if (fw.n_propios > max_vistos)
            max_vistos = fw.n_propios;

        while (completa-- && fw_completa(&iwl, &fw))
            ;

        write_antes = iwl.rx_write;
        entregados_antes = g_entregados;
        drenados = fw.pendientes;
        drain_rx_gen2(&iwl);
        if (registra_republicados(&iwl, &fw, write_antes, drenados) != 0)
            break;
        if (g_entregados != entregados_antes + drenados) {
            fw.error = "el drenaje no entregó todos los paquetes anunciados";
            break;
        }
        if (!conservacion_ok(&fw))
            break;
    }

    snprintf(label, sizeof(label),
             "%s: %u vueltas de anillo RX sin romper propiedad ni orden (%u paquetes, "
             "pico de %u buffers en el FW)",
             gen3 ? "AX211" : "AX200", vueltas, g_entregados, max_vistos);
    if (fw.error)
        printf("      motivo: %s (semilla 0x%08x)\n", fw.error, semilla);
    check(!fw.error && !g_orden_roto && g_entregados > vueltas / 2, label);
}

/* --- Modelo de firmware para el anillo TX ------------------------------ */

static void soak_tx(int gen3, uint32_t semilla, unsigned vueltas)
{
    struct iwl_ax211_priv iwl;
    uint32_t rnd = semilla;
    uint8_t payload[128];
    uint8_t pkt[8 + 48];
    /* Modelo: TFDs en vuelo, en orden de envío. */
    unsigned vuelo[IWL_MGMT_QUEUE_SIZE];
    unsigned n_vuelo = 0;
    int ocupado[IWL_MGMT_QUEUE_SIZE];
    const char *error = 0;
    unsigned enviados = 0;
    unsigned confirmados = 0;
    unsigned rechazos = 0;
    unsigned repetidas = 0;
    /* Historial de TFD ya confirmados, para repetir respuestas.
     *
     * Sólo se repiten los que **ya no están en vuelo**. La secuencia sólo
     * lleva 8 bits de índice y ningún número de vuelta, así que una respuesta
     * repetida de un índice que entretanto se ha reutilizado es indistinguible
     * de la legítima: ni este driver ni `iwl_txq_reclaim` de Linux pueden
     * defenderse de eso, y pedírselo sería inventar un requisito. Lo que sí
     * tiene que ignorar es la que apunta por detrás de la cabeza de la cola. */
    unsigned hist[8];
    unsigned n_hist = 0;
    unsigned i;
    char label[160];
    const uint16_t QID = 5;

    memset(&iwl, 0, sizeof(iwl));
    iwl.gen3 = gen3;
    iwl.mmio = g_mmio_stub;
    iwl.alive = 1;
    iwl.mgmt_txq_ready = 1;
    iwl.mgmt_txq_id = QID;
    memset(ocupado, 0, sizeof(ocupado));
    memset(payload, 0xa5, sizeof(payload));

    for (i = 0; i < vueltas && !error; i++) {
        int quiere_enviar = (iwl_test_rand(&rnd) % 3u) != 0;
        int quiere_responder = (iwl_test_rand(&rnd) % 2u) != 0;

        if (iwl_trans_tx_space(&iwl) != IWL_MGMT_QUEUE_SIZE - 1u - n_vuelo) {
            error = "el hueco declarado no coincide con los TFD en vuelo";
            break;
        }
        if (quiere_enviar) {
            unsigned idx = (unsigned)(iwl.mgmt_txq_write & (IWL_MGMT_QUEUE_SIZE - 1u));
            int hay_sitio = n_vuelo < IWL_MGMT_QUEUE_SIZE - 1u;
            int rc = iwl_trans_tx(&iwl, QID, payload, (uint16_t)sizeof(payload));

            if (hay_sitio) {
                if (rc != 0) {
                    error = "rechazó un envío con sitio de sobra";
                    break;
                }
                if (ocupado[idx]) {
                    error = "reutilizó un TFD que el firmware no ha confirmado";
                    break;
                }
                {
                    uint32_t db = g_mmio_stub[HBUS_TARG_WRPTR / 4];

                    if (db != (((uint32_t)iwl.mgmt_txq_write & 0xffu) |
                               ((uint32_t)QID << 16))) {
                        error = "el doorbell no corresponde al productor";
                        break;
                    }
                }
                ocupado[idx] = 1;
                vuelo[n_vuelo++] = idx;
                enviados++;
            } else {
                if (rc == 0) {
                    error = "aceptó un envío con la cola llena";
                    break;
                }
                rechazos++;
            }
        }
        /* Un firmware puede repetir una respuesta (reintento, duplicado en el
         * anillo). Reprocesarla no puede devolver al productor TFDs que sigue
         * teniendo en vuelo: el invariante de espacio de la vuelta siguiente lo
         * comprueba. */
        if (n_hist && (iwl_test_rand(&rnd) % 4u) == 0) {
            unsigned viejo = hist[iwl_test_rand(&rnd) % n_hist];

            if (!ocupado[viejo]) {
                unsigned n = iwl_test_tx_resp(pkt, gen3, QID, viejo, TX_STATUS_SUCCESS);

                handle_gen2_rx(&iwl, pkt, n);
                repetidas++;
            }
        }
        if (quiere_responder && n_vuelo) {
            unsigned idx = vuelo[0];
            unsigned k;
            unsigned n;

            for (k = 1; k < n_vuelo; k++)
                vuelo[k - 1] = vuelo[k];
            n_vuelo--;
            n = iwl_test_tx_resp(pkt, gen3, QID, idx, TX_STATUS_SUCCESS);
            handle_gen2_rx(&iwl, pkt, n);
            if (iwl.last_mgmt_tx_status != TX_STATUS_SUCCESS) {
                error = "la respuesta del firmware no se interpretó";
                break;
            }
            ocupado[idx] = 0;
            if (n_hist < 8u)
                n_hist++;
            memmove(&hist[1], &hist[0], (n_hist - 1u) * sizeof(hist[0]));
            hist[0] = idx;
            confirmados++;
        }
    }

    snprintf(label, sizeof(label),
             "%s: %u vueltas de cola TX sin pisar TFD en vuelo (%u enviados, %u "
             "confirmados, %u rechazos por cola llena, %u respuestas repetidas)",
             gen3 ? "AX211" : "AX200", vueltas, enviados, confirmados, rechazos,
             repetidas);
    if (error)
        printf("      motivo: %s (semilla 0x%08x)\n", error, semilla);
    /* Sin rechazos la cola nunca se llenó y el caso interesante no se ha
     * probado: eso también es un fallo del banco, no un aprobado. */
    check(!error && rechazos > 0 && confirmados > 0 && repetidas > 0, label);
}

int main(int argc, char **argv)
{
    uint32_t semilla = (argc > 1) ? (uint32_t)strtoul(argv[1], 0, 0) : 0x5e1f1a11u;
    unsigned vueltas = (argc > 2) ? (unsigned)strtoul(argv[2], 0, 0) : 4000u;

    if (!semilla)
        semilla = 1;
    printf("ring_soak: semilla 0x%08x, %u vueltas por anillo\n", semilla, vueltas);
    soak_rx(0, semilla, vueltas);
    soak_rx(1, semilla ^ 0x9e3779b9u, vueltas);
    soak_tx(0, semilla ^ 0x2545f491u, vueltas);
    soak_tx(1, semilla ^ 0x85ebca6bu, vueltas);
    puts(fails ? "anillos bajo carga: FALLOS" :
                 "anillos bajo carga OK (modelo de firmware, no silicio)");
    return fails ? 1 : 0;
}
