/* Hostcheck del transporte MHI de ath11k (WCN6855, Steam Deck OLED).
 *
 * Compila el port contra un **modelo del dispositivo**: los accesos MMIO no
 * van a un BAR sino a una máquina de estados que imita lo que el WCN6855 hace
 * durante el arranque (PBL → descarga BHIe → AMSS → M0). Eso permite ejercer
 * la secuencia entera, y sus caminos de error, sin la Deck delante.
 *
 * Lo que **no** cubre: que los offsets y los tiempos del silicio real sean
 * éstos. Eso sólo lo dice la placa. Aquí se comprueba la lógica: orden de las
 * escrituras, contratos con el dispositivo, y que ningún fallo cuelgue el
 * arranque.
 *
 * Compilar y ejecutar: scripts/l6-ath11k-hostcheck.sh
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdarg.h>

#include "lx_emul.h"
#include "ath11k_internal.h"
#include "ath11k_qmi.h"

/* ---- Stubs de la capa lx_emul ---- */

static int traza_verbosa;

int lx_printk(const char *fmt, ...)
{
    if (!traza_verbosa)
        return 0;
    va_list ap;
    va_start(ap, fmt);
    fputs("    ", stdout);
    vprintf(fmt, ap);
    va_end(ap);
    return 0;
}

/* En el modelo el tiempo no avanza de verdad: los reintentos del port se
 * consumen al instante y un fallo se detecta en microsegundos. */
void lx_mdelay(unsigned int ms) { (void)ms; }
void lx_udelay(unsigned long us) { (void)us; }

static uint64_t reloj_ns = 1000;
uint64_t lx_ktime_get_ns(void) { return reloj_ns += 1234567; }

/* La dirección «DMA» es el propio puntero: así el modelo puede leer lo que el
 * port dejó en memoria, que es justo lo que hace el dispositivo real. */
void *lx_dma_alloc_coherent(struct lx_pci_dev *dev, size_t size, uint64_t *dma,
                            unsigned gfp)
{
    (void)dev; (void)gfp;
    void *p = aligned_alloc(4096, (size + 4095u) & ~(size_t)4095u);
    if (p) {
        memset(p, 0, (size + 4095u) & ~(size_t)4095u);
        if (dma)
            *dma = (uint64_t)(uintptr_t)p;
    }
    return p;
}

void *lx_dma_alloc_wb(struct lx_pci_dev *dev, size_t size, uint64_t *dma, unsigned gfp)
{
    return lx_dma_alloc_coherent(dev, size, dma, gfp);
}

void lx_dma_free_coherent(struct lx_pci_dev *dev, size_t size, void *cpu, uint64_t dma)
{
    (void)dev; (void)size; (void)dma;
    free(cpu);
}

static int flush_llamado;
void lx_dma_flush_range(const void *p, size_t len)
{
    (void)p; (void)len;
    flush_llamado++;
}

/* ---- Modelo del dispositivo ---- */

#define SIM_BAR_LEN   0x20000u
#define SIM_BHI_OFF   0x1000u
#define SIM_BHIE_OFF  0x1c00u
#define SIM_CHDB_OFF  0x0300u
#define SIM_ERDB_OFF  0x0500u

struct sim {
    /* Registros como memoria plana; basta para lo que toca el port. */
    uint32_t reg[SIM_BAR_LEN / 4];
    enum mhi_ee ee;
    enum mhi_state estado;
    int ready;
    int syserr;

    /* Comportamientos a inyectar. */
    int bus_caido;
    int fw_falla;
    int nunca_ready;
    int bhioff_malo;

    /* Rastro de lo que hizo el host. */
    int intvec_escrito;
    int intvec_tras_reset;
    int reset_pedido;
    uint32_t fw_seq;
    uint64_t vec_addr;
    uint32_t vec_size;
    unsigned long fw_bytes_vistos;
    int fw_descargado;
};

static struct sim sim;

static void sim_status_reg(void)
{
    uint32_t v = 0;
    v |= ((uint32_t)sim.estado << MHISTATUS_MHISTATE_SHIFT) & MHISTATUS_MHISTATE_MASK;
    if (sim.ready)
        v |= MHISTATUS_READY_MASK;
    if (sim.syserr)
        v |= MHISTATUS_SYSERR_MASK;
    sim.reg[MHISTATUS / 4] = v;
}

static void sim_init(void)
{
    memset(&sim, 0, sizeof(sim));
    sim.ee = MHI_EE_PBL;
    sim.estado = MHI_STATE_READY;
    sim.ready = 1;
    sim.reg[BHIOFF / 4] = SIM_BHI_OFF;
    sim.reg[BHIEOFF / 4] = SIM_BHIE_OFF;
    sim.reg[CHDBOFF / 4] = SIM_CHDB_OFF;
    sim.reg[ERDBOFF / 4] = SIM_ERDB_OFF;
    sim.reg[MHICFG / 4] = 0;
    sim.reg[(SIM_BHI_OFF + BHI_EXECENV) / 4] = MHI_EE_PBL;
    sim_status_reg();
    flush_llamado = 0;
}

/* La descarga por BHIe: el dispositivo lee la tabla de vectores y suma lo que
 * el host dice haber puesto. Comprobarlo aquí es lo que convierte el test en
 * algo más que «se escribió un registro». */
static void sim_procesa_descarga(void)
{
    uint32_t base = SIM_BHIE_OFF;
    if (sim.fw_falla) {
        sim.reg[(base + BHIE_TXVECSTATUS_OFFS) / 4] =
            ((uint32_t)BHIE_TXVECSTATUS_STATUS_ERROR << BHIE_TXVECSTATUS_STATUS_SHIFT) |
            sim.fw_seq;
        return;
    }
    const struct bhi_vec_entry *vec = (const struct bhi_vec_entry *)(uintptr_t)sim.vec_addr;
    unsigned long total = 0;
    if (vec && sim.vec_size % sizeof(struct bhi_vec_entry) == 0) {
        uint32_t n = sim.vec_size / (uint32_t)sizeof(struct bhi_vec_entry);
        for (uint32_t i = 0; i < n; i++)
            total += (unsigned long)vec[i].size;
    }
    sim.fw_bytes_vistos = total;
    sim.fw_descargado = 1;
    sim.reg[(base + BHIE_TXVECSTATUS_OFFS) / 4] =
        ((uint32_t)BHIE_TXVECSTATUS_STATUS_XFER_COMPL << BHIE_TXVECSTATUS_STATUS_SHIFT) |
        sim.fw_seq;
    /* Tras cargar amss.bin el dispositivo cambia de entorno. */
    sim.ee = MHI_EE_AMSS;
    sim.reg[(SIM_BHI_OFF + BHI_EXECENV) / 4] = MHI_EE_AMSS;
}

uint32_t ath11k_read32(struct ath11k_base *ab, uint32_t off)
{
    (void)ab;
    if (sim.bus_caido)
        return 0xffffffffu;
    if (off >= SIM_BAR_LEN)
        return 0xffffffffu;
    if (off == MHISTATUS)
        sim_status_reg();
    if (off == BHIOFF && sim.bhioff_malo)
        return SIM_BAR_LEN + 0x1000u;
    return sim.reg[off / 4];
}

void ath11k_write32(struct ath11k_base *ab, uint32_t off, uint32_t val)
{
    (void)ab;
    if (sim.bus_caido || off >= SIM_BAR_LEN)
        return;
    sim.reg[off / 4] = val;

    if (off == MHICTRL) {
        if (val & MHICTRL_RESET_MASK) {
            sim.reset_pedido++;
            /* El dispositivo limpia RESET y borra INTVEC al procesarlo. */
            sim.reg[MHICTRL / 4] = val & ~MHICTRL_RESET_MASK;
            sim.reg[(SIM_BHI_OFF + BHI_INTVEC) / 4] = 0xdeadbeef;
            sim.intvec_tras_reset = 0;
            sim.syserr = 0;
            sim.estado = MHI_STATE_READY;
            sim.ready = sim.nunca_ready ? 0 : 1;
        } else {
            uint32_t st = (val & MHICTRL_MHISTATE_MASK) >> MHICTRL_MHISTATE_SHIFT;
            if (st == MHI_STATE_M0)
                sim.estado = MHI_STATE_M0;
        }
        sim_status_reg();
        return;
    }
    if (off == SIM_BHI_OFF + BHI_INTVEC) {
        sim.intvec_escrito++;
        if (sim.reset_pedido)
            sim.intvec_tras_reset = 1;
        return;
    }
    if (off == SIM_BHIE_OFF + BHIE_TXVECADDR_LOW_OFFS) {
        sim.vec_addr = (sim.vec_addr & 0xffffffff00000000ull) | val;
        return;
    }
    if (off == SIM_BHIE_OFF + BHIE_TXVECADDR_HIGH_OFFS) {
        sim.vec_addr = (sim.vec_addr & 0xffffffffull) | ((uint64_t)val << 32);
        return;
    }
    if (off == SIM_BHIE_OFF + BHIE_TXVECSIZE_OFFS) {
        sim.vec_size = val;
        return;
    }
    if (off == SIM_BHIE_OFF + BHIE_TXVECDB_OFFS) {
        sim.fw_seq = val;
        sim_procesa_descarga();
        return;
    }
}

/* ---- Comprobaciones ---- */

static int fallos;
static int comprobaciones;

static void ok(int cond, const char *que)
{
    comprobaciones++;
    if (cond) {
        printf("  ok   %s\n", que);
    } else {
        printf("  FALLO %s\n", que);
        fallos++;
    }
}

static unsigned char firmware[1300 * 1024];

static struct ath11k_base *nuevo_ab(void)
{
    static struct ath11k_base ab;
    memset(&ab, 0, sizeof(ab));
    ab.mmio_len = SIM_BAR_LEN;
    ab.device_id = WCN6855_DEVICE_ID;
    return &ab;
}

static void caso_layout(void)
{
    printf("layout de estructuras (contrato con el dispositivo)\n");
    /* Los tres contextos son de 44 bytes: tres dwords seguidos de cuatro u64
     * alineados a 4 (`__packed __aligned(4)` en common.h). Si el compilador
     * alineara los u64 a 8 metería relleno y el dispositivo leería basura,
     * así que el tamaño y los offsets son parte del contrato, no un detalle. */
    ok(sizeof(struct mhi_chan_ctxt) == 44, "mhi_chan_ctxt son 44 bytes");
    ok(sizeof(struct mhi_event_ctxt) == 44, "mhi_event_ctxt son 44 bytes");
    ok(sizeof(struct mhi_cmd_ctxt) == 44, "mhi_cmd_ctxt son 44 bytes");
    ok(sizeof(struct mhi_tre) == 16, "el TRE son 16 bytes");
    ok(sizeof(struct bhi_vec_entry) == 16, "bhi_vec_entry son 16 bytes");
    ok(offsetof(struct mhi_chan_ctxt, rbase) == 12 &&
       offsetof(struct mhi_chan_ctxt, rlen) == 20 &&
       offsetof(struct mhi_chan_ctxt, rp) == 28 &&
       offsetof(struct mhi_chan_ctxt, wp) == 36,
       "chan_ctxt: rbase/rlen/rp/wp en 12/20/28/36");
    ok(offsetof(struct mhi_event_ctxt, rbase) == 12 &&
       offsetof(struct mhi_event_ctxt, wp) == 36,
       "event_ctxt: rbase en 12 y wp en 36");
    ok(offsetof(struct mhi_cmd_ctxt, rbase) == 12,
       "cmd_ctxt.rbase en el offset 12");
}

static void caso_arranque_normal(void)
{
    printf("arranque desde PBL hasta mission mode\n");
    sim_init();
    struct ath11k_base *ab = nuevo_ab();
    int rc = ath11k_mhi_power_up(ab, firmware, sizeof(firmware));

    ok(rc == 0, "power_up termina bien");
    ok(ab->state == MHI_STATE_M0, "el transporte queda en M0");
    ok(ab->ee == MHI_EE_AMSS, "el dispositivo llegó a AMSS");
    ok(strcmp(ab->phase, "mission_mode") == 0, "fase final mission_mode");
    ok(sim.fw_descargado, "el dispositivo aceptó la descarga");
    ok(sim.fw_bytes_vistos == sizeof(firmware),
       "los vectores describen el firmware entero");
    ok(flush_llamado > 0, "se bajó el firmware de la caché antes de publicarlo");
    ok(sim.intvec_escrito > 0, "INTVEC programado antes de arrancar");

    /* Contextos publicados: sin esto el dispositivo no sabe dónde están los
     * anillos y el primer comando se pierde en silencio. */
    uint64_t cca = ((uint64_t)sim.reg[CCABAP_HIGHER / 4] << 32) | sim.reg[CCABAP_LOWER / 4];
    uint64_t eca = ((uint64_t)sim.reg[ECABAP_HIGHER / 4] << 32) | sim.reg[ECABAP_LOWER / 4];
    uint64_t crc = ((uint64_t)sim.reg[CRCBAP_HIGHER / 4] << 32) | sim.reg[CRCBAP_LOWER / 4];
    ok(cca == ab->chan_ctxt_dma && cca != 0, "CCABAP apunta al contexto de canales");
    ok(eca == ab->er_ctxt_dma && eca != 0, "ECABAP apunta al contexto de eventos");
    ok(crc == ab->cmd_ctxt_dma && crc != 0, "CRCBAP apunta al contexto de comandos");

    uint32_t ner = (sim.reg[MHICFG / 4] & MHICFG_NER_MASK) >> MHICFG_NER_SHIFT;
    ok(ner == ATH11K_MHI_NUM_EVENT_RINGS, "MHICFG declara los anillos de evento");

    /* El canal IPCR (20/21) es por donde irá QMI: debe existir en su índice. */
    ok(ab->chan_ctxt[ATH11K_MHI_CH_IPCR_OUT].chtype == 1,
       "IPCR de salida en el índice 20");
    ok(ab->chan_ctxt[ATH11K_MHI_CH_IPCR_IN].chtype == 2,
       "IPCR de entrada en el índice 21");
    ok(ab->chan_ctxt[ATH11K_MHI_CH_IPCR_IN].erindex == 0,
       "IPCR usa el anillo de evento de control");
    ok(ab->er_ctxt[0].rbase == ab->ev_ring_dma[0] && ab->er_ctxt[0].rlen != 0,
       "el anillo de evento 0 queda descrito");
    ok(ab->cmd_ctxt->rbase == ab->cmd_ring_dma && ab->cmd_ctxt->rp == ab->cmd_ring_dma,
       "el anillo de comandos empieza vacío en su base");
}

static void caso_syserr(void)
{
    printf("arranque con SYS_ERR pendiente (caso QCA6390 tras reset global)\n");
    sim_init();
    sim.syserr = 1;
    sim.estado = MHI_STATE_SYS_ERR;
    struct ath11k_base *ab = nuevo_ab();
    int rc = ath11k_mhi_power_up(ab, firmware, sizeof(firmware));
    ok(rc == 0, "power_up se recupera del SYS_ERR");
    ok(sim.reset_pedido == 1, "se pidió exactamente un reset de MHI");
    ok(sim.intvec_tras_reset, "INTVEC se reprograma después del reset");
}

static void caso_ya_en_amss(void)
{
    printf("dispositivo ya en AMSS (rearranque en caliente)\n");
    sim_init();
    sim.ee = MHI_EE_AMSS;
    sim.reg[(SIM_BHI_OFF + BHI_EXECENV) / 4] = MHI_EE_AMSS;
    struct ath11k_base *ab = nuevo_ab();
    int rc = ath11k_mhi_power_up(ab, firmware, sizeof(firmware));
    ok(rc == 0, "power_up termina bien");
    ok(!sim.fw_descargado, "no se vuelve a descargar el firmware");
    ok(ab->state == MHI_STATE_M0, "queda en M0 igualmente");
}

static void caso_bus_caido(void)
{
    printf("el bus no responde (todo 0xffffffff)\n");
    sim_init();
    sim.bus_caido = 1;
    struct ath11k_base *ab = nuevo_ab();
    int rc = ath11k_mhi_power_up(ab, firmware, sizeof(firmware));
    ok(rc != 0, "power_up falla en vez de creerse los 0xffffffff");
    ok(strcmp(ab->phase, "bhi_off") == 0, "falla ya en la lectura de BHIOFF");
}

static void caso_bhioff_fuera_del_bar(void)
{
    printf("BHIOFF fuera del BAR\n");
    sim_init();
    sim.bhioff_malo = 1;
    struct ath11k_base *ab = nuevo_ab();
    int rc = ath11k_mhi_power_up(ab, firmware, sizeof(firmware));
    ok(rc != 0, "se rechaza en vez de escribir fuera del mapeo");
}

static void caso_firmware_rechazado(void)
{
    printf("el dispositivo rechaza el firmware\n");
    sim_init();
    sim.fw_falla = 1;
    struct ath11k_base *ab = nuevo_ab();
    int rc = ath11k_mhi_power_up(ab, firmware, sizeof(firmware));
    ok(rc != 0, "power_up devuelve error");
    ok(strcmp(ab->phase, "fw_download") == 0, "la fase señala dónde murió");
}

static void caso_nunca_ready(void)
{
    printf("el dispositivo nunca llega a READY\n");
    sim_init();
    sim.ready = 0;
    sim.nunca_ready = 1;
    struct ath11k_base *ab = nuevo_ab();
    int rc = ath11k_mhi_power_up(ab, firmware, sizeof(firmware));
    ok(rc != 0, "power_up se rinde en vez de girar para siempre");
    ok(strcmp(ab->phase, "ready") == 0, "la fase señala la espera de READY");
}

/* ---- QMI: formato del cable ---- */

static int bytes_iguales(const unsigned char *a, const unsigned char *b, unsigned int n)
{
    for (unsigned int i = 0; i < n; i++)
        if (a[i] != b[i])
            return 0;
    return 1;
}

static void caso_qmi_cabecera(void)
{
    printf("QMI: cabecera y TLV\n");
    unsigned char buf[64];
    struct qmi_writer w;
    ath11k_qmi_init(&w, buf, sizeof(buf));
    ath11k_qmi_put_u8(&w, 0x10, 0xab);
    ath11k_qmi_put_u32(&w, 0x11, 0x11223344);
    int n = ath11k_qmi_finish(&w, QMI_REQUEST, 0x0102, 0x0034);

    /* cabecera: type=0, txn=0x0102 LE, msg_id=0x0034 LE, msg_len LE.
     * La carga son 4 bytes del primer TLV (1+2+1) más 7 del segundo
     * (1+2+4) = 11; la cabecera no se cuenta en msg_len. */
    const unsigned char esperado[] = {
        0x00, 0x02, 0x01, 0x34, 0x00, 0x0b, 0x00,
        0x10, 0x01, 0x00, 0xab,
        0x11, 0x04, 0x00, 0x44, 0x33, 0x22, 0x11,
    };
    ok(n == (int)sizeof(esperado), "longitud total del mensaje");
    ok(bytes_iguales(buf, esperado, sizeof(esperado)), "bytes exactos del cable");

    struct qmi_msg_hdr h;
    ok(ath11k_qmi_parse_hdr(buf, (unsigned int)n, &h) == 0 &&
       h.type == QMI_REQUEST && h.txn == 0x0102 && h.msg_id == 0x0034 &&
       h.msg_len == 11,
       "la cabecera se relee igual que se escribió");

    unsigned int l = 0;
    const unsigned char *v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x11, &l);
    ok(v && l == 4 && v[0] == 0x44 && v[3] == 0x11, "find_tlv localiza el u32");
    ok(ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x99, &l) == 0,
       "un TLV ausente no devuelve basura");
}

static void caso_qmi_desbordes(void)
{
    printf("QMI: un búfer corto no produce un mensaje a medias\n");
    unsigned char buf[10];
    struct qmi_writer w;
    ath11k_qmi_init(&w, buf, sizeof(buf));
    ath11k_qmi_put_u32(&w, 0x10, 1); /* 7 + 7 = 14 > 10 */
    ok(ath11k_qmi_finish(&w, QMI_REQUEST, 1, 1) < 0,
       "finish falla si algún TLV no cupo");

    unsigned char mini[3];
    ath11k_qmi_init(&w, mini, sizeof(mini));
    ok(ath11k_qmi_finish(&w, QMI_REQUEST, 1, 1) < 0,
       "un búfer menor que la cabecera se rechaza");

    /* Un mensaje que declara más carga de la recibida no debe leerse. */
    unsigned char roto[] = { 0x02, 0x01, 0x00, 0x20, 0x00, 0xff, 0x00, 0x02 };
    struct qmi_msg_hdr h;
    ok(ath11k_qmi_parse_hdr(roto, sizeof(roto), &h) != 0,
       "msg_len mayor que el búfer se rechaza");

    /* Y un TLV cuya longitud se sale del mensaje tampoco. */
    unsigned char tlv_roto[] = { 0x02, 0x01, 0x00, 0x20, 0x00, 0x04, 0x00,
                                 0x02, 0xff, 0x00, 0x00 };
    unsigned int l = 0;
    ok(ath11k_qmi_find_tlv(tlv_roto, sizeof(tlv_roto), 0x02, &l) == 0,
       "un TLV que se sale del mensaje se rechaza");
}

static void caso_qmi_respuesta(void)
{
    printf("QMI: resultado de una respuesta\n");
    /* Respuesta con resultado SUCCESS. */
    unsigned char ok_resp[] = { QMI_RESPONSE, 0x01, 0x00, 0x20, 0x00, 0x07, 0x00,
                                QMI_TLV_RESULT, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00 };
    uint16_t res = 0xffff, err = 0xffff;
    ok(ath11k_qmi_parse_result(ok_resp, sizeof(ok_resp), &res, &err) == 0 &&
       res == QMI_RESULT_SUCCESS && err == 0,
       "una respuesta correcta se lee como SUCCESS");

    /* Y una de fallo, con su código de error. */
    unsigned char ko_resp[] = { QMI_RESPONSE, 0x01, 0x00, 0x20, 0x00, 0x07, 0x00,
                                QMI_TLV_RESULT, 0x04, 0x00, 0x01, 0x00, 0x2a, 0x00 };
    ok(ath11k_qmi_parse_result(ko_resp, sizeof(ko_resp), &res, &err) == 0 &&
       res == QMI_RESULT_FAILURE && err == 0x2a,
       "un fallo se lee con su código de error");

    /* Sin TLV de resultado no hay veredicto: no se puede dar por buena. */
    unsigned char sin_result[] = { QMI_RESPONSE, 0x01, 0x00, 0x20, 0x00, 0x04, 0x00,
                                   0x10, 0x01, 0x00, 0x01 };
    ok(ath11k_qmi_parse_result(sin_result, sizeof(sin_result), &res, &err) != 0,
       "una respuesta sin TLV de resultado se rechaza");
}

static void caso_qmi_mensajes(void)
{
    printf("QMI: mensajes de la secuencia de arranque\n");
    unsigned char buf[512];

    int n = ath11k_qmi_build_ind_register(buf, sizeof(buf), 1);
    struct qmi_msg_hdr h;
    ok(n > 0 && ath11k_qmi_parse_hdr(buf, (unsigned int)n, &h) == 0 &&
       h.msg_id == QMI_WLANFW_IND_REGISTER_REQ_V01 && h.type == QMI_REQUEST,
       "IND_REGISTER lleva su msg_id");
    unsigned int l = 0;
    const unsigned char *v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x15, &l);
    ok(v && l == 4 &&
       (uint32_t)(v[0] | (v[1] << 8) | (v[2] << 16) | ((uint32_t)v[3] << 24)) ==
           QMI_WLANFW_CLIENT_ID,
       "IND_REGISTER manda el client_id en el TLV 0x15");
    v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x18, &l);
    ok(v && l == 1 && v[0] == 1, "pide la indicación fw_init_done (0x18)");
    v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x16, &l);
    ok(v && l == 1 && v[0] == 1, "pide la indicación request_mem (0x16)");

    n = ath11k_qmi_build_host_cap(buf, sizeof(buf), 2, 0, 0);
    ok(n > 0 && ath11k_qmi_parse_hdr(buf, (unsigned int)n, &h) == 0 &&
       h.msg_id == QMI_WLANFW_HOST_CAP_REQ_V01, "HOST_CAP lleva su msg_id");
    v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x13, &l);
    ok(v && l == 1 &&
       v[0] == (HOST_CSTATE_BIT | SLEEP_CLOCK_SELECT_INTERNAL_BIT |
                PLATFORM_CAP_PCIE_GLOBAL_RESET | PLATFORM_CAP_PCIE_PME_D3COLD),
       "nm_modem con los cuatro bits que pone ath11k en WCN6855");
    v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x16, &l);
    ok(v && l == 1 && v[0] == 1, "declara soporte de m3 (0x16)");
    v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x14, &l);
    ok(v && l == 1 && v[0] == 1, "declara soporte de BDF (0x14)");

    n = ath11k_qmi_build_cap_req(buf, sizeof(buf), 3);
    ok(n == QMI_HDR_LEN && ath11k_qmi_parse_hdr(buf, (unsigned int)n, &h) == 0 &&
       h.msg_id == QMI_WLANFW_CAP_REQ_V01 && h.msg_len == 0,
       "CAP es una petición sin cuerpo");

    n = ath11k_qmi_build_wlan_mode(buf, sizeof(buf), 4, ATH11K_FIRMWARE_MODE_NORMAL, 0);
    ok(n > 0 && ath11k_qmi_parse_hdr(buf, (unsigned int)n, &h) == 0 &&
       h.msg_id == QMI_WLANFW_WLAN_MODE_REQ_V01, "WLAN_MODE lleva su msg_id");
    v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x01, &l);
    ok(v && l == 4 && v[0] == ATH11K_FIRMWARE_MODE_NORMAL,
       "el modo va en el TLV obligatorio 0x01");
    ok(ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x10, &l) == 0,
       "sin hw_debug no se emite su TLV opcional");
}

static void caso_qmi_bdf_y_m3(void)
{
    printf("QMI: BDF_DOWNLOAD y M3_INFO\n");
    unsigned char buf[8192];
    unsigned char trozo[100];
    for (unsigned int i = 0; i < sizeof(trozo); i++)
        trozo[i] = (unsigned char)(i + 1);

    int n = ath11k_qmi_build_bdf_download(buf, sizeof(buf), 7, 12345, 0, trozo,
                                          sizeof(trozo), 0, ATH11K_QMI_BDF_TYPE_ELF);
    struct qmi_msg_hdr h;
    ok(n > 0 && ath11k_qmi_parse_hdr(buf, (unsigned int)n, &h) == 0 &&
       h.msg_id == QMI_WLANFW_BDF_DOWNLOAD_REQ_V01, "BDF_DOWNLOAD lleva su msg_id");

    unsigned int l = 0;
    const unsigned char *v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x13, &l);
    /* El TLV de datos lleva DENTRO un prefijo de longitud de 2 bytes: si se
     * omitiera, el firmware leería el blob desplazado dos posiciones. */
    ok(v && l == sizeof(trozo) + 2, "el TLV de datos incluye su prefijo de 2 bytes");
    ok(v && (unsigned int)(v[0] | (v[1] << 8)) == sizeof(trozo),
       "el prefijo declara los bytes del segmento");
    ok(v && v[2] == 1 && v[2 + sizeof(trozo) - 1] == (unsigned char)sizeof(trozo),
       "el contenido del segmento va detrás del prefijo, intacto");

    v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x11, &l);
    ok(v && l == 4 &&
       (uint32_t)(v[0] | (v[1] << 8) | (v[2] << 16) | ((uint32_t)v[3] << 24)) == 12345,
       "total_size viaja en el TLV 0x11");
    v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x14, &l);
    ok(v && l == 1 && v[0] == 0, "end=0 mientras quedan segmentos");
    v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x15, &l);
    ok(v && l == 1 && v[0] == ATH11K_QMI_BDF_TYPE_ELF, "bdf_type en el TLV 0x15");

    /* El último segmento marca end=1. */
    n = ath11k_qmi_build_bdf_download(buf, sizeof(buf), 8, 12345, 1, trozo, 10, 1,
                                      ATH11K_QMI_BDF_TYPE_ELF);
    v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x14, &l);
    ok(v && l == 1 && v[0] == 1, "el último segmento marca end=1");

    /* Un segmento mayor que el máximo del protocolo se rechaza. */
    static unsigned char grande[QMI_WLANFW_MAX_DATA_SIZE_V01 + 1];
    ok(ath11k_qmi_build_bdf_download(buf, sizeof(buf), 9, 1, 0, grande,
                                     sizeof(grande), 1,
                                     ATH11K_QMI_BDF_TYPE_ELF) < 0,
       "un segmento mayor de 6144 B se rechaza");

    n = ath11k_qmi_build_m3_info(buf, sizeof(buf), 10, 0x1234abcd5678ull, 4096);
    ok(n > 0 && ath11k_qmi_parse_hdr(buf, (unsigned int)n, &h) == 0 &&
       h.msg_id == QMI_WLANFW_M3_INFO_REQ_V01, "M3_INFO lleva su msg_id");
    v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x01, &l);
    ok(v && l == 8 && v[0] == 0x78 && v[5] == 0x12,
       "la dirección de m3 va como u64 little endian");
    v = ath11k_qmi_find_tlv(buf, (unsigned int)n, 0x02, &l);
    ok(v && l == 4 && (uint32_t)(v[0] | (v[1] << 8)) == 4096, "y su tamaño como u32");
}

int main(int argc, char **argv)
{
    if (argc > 1 && strcmp(argv[1], "-v") == 0)
        traza_verbosa = 1;
    for (size_t i = 0; i < sizeof(firmware); i++)
        firmware[i] = (unsigned char)(i * 31u);

    printf("ath11k-hostcheck — transporte MHI (WCN6855)\n\n");
    caso_layout();
    caso_arranque_normal();
    caso_syserr();
    caso_ya_en_amss();
    caso_bus_caido();
    caso_bhioff_fuera_del_bar();
    caso_firmware_rechazado();
    caso_nunca_ready();
    caso_qmi_cabecera();
    caso_qmi_desbordes();
    caso_qmi_respuesta();
    caso_qmi_mensajes();
    caso_qmi_bdf_y_m3();

    printf("\n%d comprobaciones, %d fallos\n", comprobaciones, fallos);
    return fallos ? 1 : 0;
}
