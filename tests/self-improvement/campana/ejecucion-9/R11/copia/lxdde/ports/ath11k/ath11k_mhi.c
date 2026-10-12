/* Transporte MHI de ath11k (WCN6855) — fase W1 del plan Steam Deck.
 *
 * Reimplementa, acotado, lo que en Linux hacen `drivers/bus/mhi/host/{pm,init,
 * boot}.c` para el camino que necesita ath11k: encender el dispositivo desde
 * PBL, dejarlo READY, publicar los contextos, descargar el firmware y llegar a
 * M0. No hay soporte de suspensión, RDDM ni canales de datos: eso es W3/W4.
 *
 * Toda la temporización usa reintentos con espera fija en vez de esperas por
 * evento: en soso el bring-up ocurre antes de que haya nada a lo que dormir.
 */
#include "lx_emul.h"
#include "ath11k_internal.h"

/* Linux sondea cada 25 ms (`interval_us` en pm.c) y ath11k da 2 s de plazo
 * para QCA6390/WCN6855 (`timeout_ms` en su mhi_controller_config). */
#define POLL_INTERVAL_MS 25
#define POLL_TRIES       80   /* 2 s */
/* La descarga del firmware es lo más lento del arranque. */
#define FW_TRIES         400  /* 10 s */

const struct ath11k_mhi_chan_cfg ath11k_mhi_channels[ATH11K_MHI_NUM_CHANNELS] = {
    { ATH11K_MHI_CH_LOOPBACK_OUT, "LOOPBACK", 0, 1 },
    { ATH11K_MHI_CH_LOOPBACK_IN, "LOOPBACK", 0, 0 },
    { ATH11K_MHI_CH_IPCR_OUT, "IPCR", 0, 1 },
    { ATH11K_MHI_CH_IPCR_IN, "IPCR", 0, 0 },
};

const char *ath11k_ee_str(enum mhi_ee ee)
{
    switch (ee) {
    case MHI_EE_PBL: return "PBL";
    case MHI_EE_SBL: return "SBL";
    case MHI_EE_AMSS: return "AMSS";
    case MHI_EE_RDDM: return "RDDM";
    case MHI_EE_WFW: return "WFW";
    case MHI_EE_PTHRU: return "PTHRU";
    case MHI_EE_EDL: return "EDL";
    case MHI_EE_FP: return "FP";
    default: return "?";
    }
}

const char *ath11k_state_str(enum mhi_state st)
{
    switch (st) {
    case MHI_STATE_RESET: return "RESET";
    case MHI_STATE_READY: return "READY";
    case MHI_STATE_M0: return "M0";
    case MHI_STATE_M1: return "M1";
    case MHI_STATE_M2: return "M2";
    case MHI_STATE_M3: return "M3";
    case MHI_STATE_M3_FAST: return "M3_FAST";
    case MHI_STATE_BHI: return "BHI";
    case MHI_STATE_SYS_ERR: return "SYS_ERR";
    default: return "?";
    }
}

enum mhi_ee ath11k_mhi_get_ee(struct ath11k_base *ab)
{
    uint32_t v = ath11k_read32(ab, ab->bhi_off + BHI_EXECENV);
    if (v >= MHI_EE_MAX)
        return MHI_EE_MAX;
    return (enum mhi_ee)v;
}

enum mhi_state ath11k_mhi_get_state(struct ath11k_base *ab)
{
    uint32_t v = ath11k_read32(ab, MHISTATUS);
    /* Un bus caído devuelve 0xffffffff; leerlo como un estado válido haría
     * creer que el chip contesta. */
    if (v == 0xffffffffu)
        return MHI_STATE_SYS_ERR;
    return (enum mhi_state)((v & MHISTATUS_MHISTATE_MASK) >> MHISTATUS_MHISTATE_SHIFT);
}

int ath11k_mhi_poll_field(struct ath11k_base *ab, uint32_t off, uint32_t mask,
                          uint32_t shift, uint32_t val, unsigned int intentos)
{
    for (unsigned int i = 0; i < intentos; i++) {
        uint32_t v = ath11k_read32(ab, off);
        if (v != 0xffffffffu && ((v & mask) >> shift) == val)
            return 0;
        lx_mdelay(POLL_INTERVAL_MS);
    }
    return -1;
}

/* MHICTRL con el estado pedido (mhi_set_mhi_state). */
int ath11k_mhi_set_state(struct ath11k_base *ab, enum mhi_state st)
{
    uint32_t v = ath11k_read32(ab, MHICTRL);
    if (v == 0xffffffffu)
        return -1;
    v &= ~MHICTRL_MHISTATE_MASK;
    v |= ((uint32_t)st << MHICTRL_MHISTATE_SHIFT) & MHICTRL_MHISTATE_MASK;
    ath11k_write32(ab, MHICTRL, v);
    return 0;
}

/* Reset del MHI: el dispositivo limpia RESET cuando termina.
 *
 * En QCA6390 se ha observado que tras un SOC_GLOBAL_RESET queda SYSERR puesto
 * y hace falta este reset para limpiarlo (ath11k_mhi_set_mhictrl_reset,
 * ath11k/mhi.c:213).
 */
int ath11k_mhi_reset(struct ath11k_base *ab)
{
    ath11k_write32(ab, MHICTRL, MHICTRL_RESET_MASK);
    if (ath11k_mhi_poll_field(ab, MHICTRL, MHICTRL_RESET_MASK, 1, 0, POLL_TRIES)) {
        lx_printk("ath11k: MHI no limpió RESET\n");
        return -1;
    }
    /* El dispositivo borra INTVEC al procesar el reset; hay que reprogramarlo
     * (pm.c:1138). */
    ath11k_write32(ab, ab->bhi_off + BHI_INTVEC, 0);
    return 0;
}

int ath11k_mhi_wait_ready(struct ath11k_base *ab)
{
    if (ath11k_mhi_poll_field(ab, MHICTRL, MHICTRL_RESET_MASK, 1, 0, POLL_TRIES)) {
        lx_printk("ath11k: el dispositivo no limpió MHI Reset\n");
        return -1;
    }
    if (ath11k_mhi_poll_field(ab, MHISTATUS, MHISTATUS_READY_MASK, 0, 1, POLL_TRIES)) {
        lx_printk("ath11k: el dispositivo no llegó a MHI READY\n");
        return -1;
    }
    ab->state = MHI_STATE_READY;
    return 0;
}

/* ---- Contextos y anillos ---- */

static void *dma_zalloc(struct ath11k_base *ab, size_t size, uint64_t *dma)
{
    void *p = lx_dma_alloc_coherent(ab->pdev, size, dma, GFP_KERNEL);
    if (!p)
        return NULL;
    for (size_t i = 0; i < size; i++)
        ((volatile unsigned char *)p)[i] = 0;
    return p;
}

int ath11k_mhi_alloc_ctxt(struct ath11k_base *ab)
{
    /* Los contextos de canal se indexan por número de canal, no por posición
     * en nuestra tabla: IPCR es el 20, así que el array llega hasta el máximo
     * que declara el chip. */
    size_t chan_sz = sizeof(struct mhi_chan_ctxt) * ATH11K_MHI_MAX_CHANNELS;
    ab->chan_ctxt = dma_zalloc(ab, chan_sz, &ab->chan_ctxt_dma);
    if (!ab->chan_ctxt)
        return -1;

    size_t er_sz = sizeof(struct mhi_event_ctxt) * ATH11K_MHI_NUM_EVENT_RINGS;
    ab->er_ctxt = dma_zalloc(ab, er_sz, &ab->er_ctxt_dma);
    if (!ab->er_ctxt)
        return -1;

    ab->cmd_ctxt = dma_zalloc(ab, sizeof(struct mhi_cmd_ctxt), &ab->cmd_ctxt_dma);
    if (!ab->cmd_ctxt)
        return -1;

    size_t cmd_sz = sizeof(struct mhi_tre) * ATH11K_MHI_CMD_ELEMENTS;
    ab->cmd_ring = dma_zalloc(ab, cmd_sz, &ab->cmd_ring_dma);
    if (!ab->cmd_ring)
        return -1;
    ab->cmd_ctxt->rbase = ab->cmd_ring_dma;
    ab->cmd_ctxt->rlen = cmd_sz;
    ab->cmd_ctxt->rp = ab->cmd_ring_dma;
    ab->cmd_ctxt->wp = ab->cmd_ring_dma;

    for (int i = 0; i < ATH11K_MHI_NUM_EVENT_RINGS; i++) {
        size_t ev_sz = sizeof(struct mhi_tre) * ATH11K_MHI_EV_ELEMENTS;
        ab->ev_ring[i] = dma_zalloc(ab, ev_sz, &ab->ev_ring_dma[i]);
        if (!ab->ev_ring[i])
            return -1;
        struct mhi_event_ctxt *er = &ab->er_ctxt[i];
        /* intmod: moderación en los 16 bits altos, contador en 15..8. El
         * anillo de control de ath11k va sin moderación y el de datos con
         * 1 ms (ath11k_mhi_events_qca6390). */
        er->intmod = (i == 0) ? 0 : (1u << 16);
        er->ertype = 1; /* válido */
        er->msivec = i;
        er->rbase = ab->ev_ring_dma[i];
        er->rlen = ev_sz;
        er->rp = ab->ev_ring_dma[i];
        er->wp = ab->ev_ring_dma[i];
    }

    for (int i = 0; i < ATH11K_MHI_NUM_CHANNELS; i++) {
        const struct ath11k_mhi_chan_cfg *c = &ath11k_mhi_channels[i];
        struct mhi_chan_ctxt *ch = &ab->chan_ctxt[c->num];
        ch->chcfg = (MHI_CH_STATE_DISABLED & CHAN_CTX_CHSTATE_MASK) |
                    ((uint32_t)MHI_DB_BRST_DISABLE << CHAN_CTX_BRSTMODE_SHIFT);
        /* chtype: 1 = salida (host→dispositivo), 2 = entrada. */
        ch->chtype = c->to_device ? 1 : 2;
        ch->erindex = c->event_ring;
    }
    return 0;
}

void ath11k_mhi_free_ctxt(struct ath11k_base *ab)
{
    /* El asignador DMA de soso no libera; lo que sí hace falta es que nadie
     * siga usando los punteros si el bring-up falló a medias. */
    ab->chan_ctxt = NULL;
    ab->er_ctxt = NULL;
    ab->cmd_ctxt = NULL;
    ab->cmd_ring = NULL;
    for (int i = 0; i < ATH11K_MHI_NUM_EVENT_RINGS; i++)
        ab->ev_ring[i] = NULL;
}

/* Publica las direcciones de los contextos (mhi_init_mmio, init.c:440). */
int ath11k_mhi_init_mmio(struct ath11k_base *ab)
{
    if (!ab->chan_ctxt || !ab->er_ctxt || !ab->cmd_ctxt)
        return -1;

    ab->chdb_off = ath11k_read32(ab, CHDBOFF);
    ab->erdb_off = ath11k_read32(ab, ERDBOFF);
    if (ab->chdb_off == 0xffffffffu || ab->erdb_off == 0xffffffffu) {
        lx_printk("ath11k: CHDBOFF/ERDBOFF ilegibles\n");
        return -1;
    }
    if (ab->mmio_len && (ab->chdb_off >= ab->mmio_len || ab->erdb_off >= ab->mmio_len)) {
        lx_printk("ath11k: doorbells fuera del BAR (chdb %#x erdb %#x len %#x)\n",
                  ab->chdb_off, ab->erdb_off, ab->mmio_len);
        return -1;
    }

    ath11k_write32(ab, CCABAP_HIGHER, (uint32_t)(ab->chan_ctxt_dma >> 32));
    ath11k_write32(ab, CCABAP_LOWER, (uint32_t)ab->chan_ctxt_dma);
    ath11k_write32(ab, ECABAP_HIGHER, (uint32_t)(ab->er_ctxt_dma >> 32));
    ath11k_write32(ab, ECABAP_LOWER, (uint32_t)ab->er_ctxt_dma);
    ath11k_write32(ab, CRCBAP_HIGHER, (uint32_t)(ab->cmd_ctxt_dma >> 32));
    ath11k_write32(ab, CRCBAP_LOWER, (uint32_t)ab->cmd_ctxt_dma);

    /* Sin IOMMU el dispositivo puede direccionar toda la memoria: los límites
     * son el rango completo, igual que hace ath11k con iova_start/iova_stop
     * cuando no hay SMMU. */
    ath11k_write32(ab, MHICTRLBASE_HIGHER, 0);
    ath11k_write32(ab, MHICTRLBASE_LOWER, 0);
    ath11k_write32(ab, MHIDATABASE_HIGHER, 0);
    ath11k_write32(ab, MHIDATABASE_LOWER, 0);
    ath11k_write32(ab, MHICTRLLIMIT_HIGHER, 0xffffffffu);
    ath11k_write32(ab, MHICTRLLIMIT_LOWER, 0xffffffffu);
    ath11k_write32(ab, MHIDATALIMIT_HIGHER, 0xffffffffu);
    ath11k_write32(ab, MHIDATALIMIT_LOWER, 0xffffffffu);

    /* Número de anillos de evento que usa el host, en MHICFG. */
    uint32_t cfg = ath11k_read32(ab, MHICFG);
    cfg &= ~(MHICFG_NER_MASK | MHICFG_NHWER_MASK);
    cfg |= ((uint32_t)ATH11K_MHI_NUM_EVENT_RINGS << MHICFG_NER_SHIFT) & MHICFG_NER_MASK;
    ath11k_write32(ab, MHICFG, cfg);
    return 0;
}

/* ---- Descarga de firmware ----
 *
 * amss.bin no cabe en un buffer contiguo cualquiera, así que se envía por
 * BHIe con una tabla de vectores (`mhi_fw_load_bhie`, boot.c:180). Aquí el
 * buffer sí es contiguo, pero se usa el mismo camino porque es el que el
 * dispositivo espera en este punto del arranque.
 */
#define BHIE_VEC_CHUNK (512u * 1024u)

/* Un identificador de sesión distinto de cero; el dispositivo lo devuelve en
 * el estado para que el host sepa a qué descarga corresponde. */
static uint32_t seq_no_cero(struct ath11k_base *ab)
{
    static uint32_t contador;
    contador++;
    uint32_t s = (uint32_t)(lx_ktime_get_ns() >> 8) + contador;
    s &= BHIE_TXVECSTATUS_SEQNUM_MASK;
    if (s == 0)
        s = 1;
    (void)ab;
    return s;
}

int ath11k_mhi_fw_download(struct ath11k_base *ab, const void *fw, unsigned long len)
{
    if (!fw || len == 0)
        return -1;
    if (!ab->bhie_off) {
        lx_printk("ath11k: sin ventana BHIe; no se puede descargar el firmware\n");
        return -1;
    }

    /* El firmware lo escribe la CPU en volumen y lo lee el dispositivo: buffer
     * cacheado más flush explícito, no memoria UC (ver lx_dma_alloc_wb). */
    ab->fw_buf = lx_dma_alloc_wb(ab->pdev, len, &ab->fw_buf_dma, GFP_KERNEL);
    if (!ab->fw_buf) {
        lx_printk("ath11k: sin memoria para %lu B de firmware\n", len);
        return -1;
    }
    for (unsigned long i = 0; i < len; i++)
        ((unsigned char *)ab->fw_buf)[i] = ((const unsigned char *)fw)[i];
    lx_dma_flush_range(ab->fw_buf, len);
    ab->fw_len = len;

    uint32_t entradas = (uint32_t)((len + BHIE_VEC_CHUNK - 1) / BHIE_VEC_CHUNK);
    size_t vec_sz = sizeof(struct bhi_vec_entry) * entradas;
    ab->fw_vec = lx_dma_alloc_coherent(ab->pdev, vec_sz, &ab->fw_vec_dma, GFP_KERNEL);
    if (!ab->fw_vec) {
        lx_printk("ath11k: sin memoria para la tabla de vectores\n");
        return -1;
    }
    ab->fw_vec_entries = entradas;
    unsigned long restante = len;
    for (uint32_t i = 0; i < entradas; i++) {
        unsigned long trozo = restante < BHIE_VEC_CHUNK ? restante : BHIE_VEC_CHUNK;
        ab->fw_vec[i].dma_addr = ab->fw_buf_dma + (uint64_t)i * BHIE_VEC_CHUNK;
        ab->fw_vec[i].size = trozo;
        restante -= trozo;
    }

    uint32_t seq = seq_no_cero(ab);
    uint32_t base = ab->bhie_off;
    ath11k_write32(ab, base + BHIE_TXVECADDR_HIGH_OFFS, (uint32_t)(ab->fw_vec_dma >> 32));
    ath11k_write32(ab, base + BHIE_TXVECADDR_LOW_OFFS, (uint32_t)ab->fw_vec_dma);
    ath11k_write32(ab, base + BHIE_TXVECSIZE_OFFS, (uint32_t)vec_sz);
    ath11k_write32(ab, base + BHIE_TXVECDB_OFFS, seq);

    lx_printk("ath11k: descarga BHIe seq=%u %lu B en %u vectores\n",
              seq, len, entradas);

    for (unsigned int i = 0; i < FW_TRIES; i++) {
        uint32_t st = ath11k_read32(ab, base + BHIE_TXVECSTATUS_OFFS);
        if (st != 0xffffffffu) {
            uint32_t code = (st & BHIE_TXVECSTATUS_STATUS_MASK) >>
                            BHIE_TXVECSTATUS_STATUS_SHIFT;
            if (code == BHIE_TXVECSTATUS_STATUS_XFER_COMPL)
                return 0;
            if (code == BHIE_TXVECSTATUS_STATUS_ERROR) {
                lx_printk("ath11k: el dispositivo rechazó el firmware (status %#x)\n", st);
                return -1;
            }
        }
        lx_mdelay(POLL_INTERVAL_MS);
    }
    lx_printk("ath11k: la descarga de firmware no terminó a tiempo\n");
    return -1;
}

/* ---- Encendido ---- */

int ath11k_mhi_power_up(struct ath11k_base *ab, const void *fw, unsigned long fw_len)
{
    ab->phase = "bhi_off";
    ab->bhi_off = ath11k_read32(ab, BHIOFF);
    ab->bhie_off = ath11k_read32(ab, BHIEOFF);
    if (ab->bhi_off == 0xffffffffu || ab->bhi_off == 0) {
        lx_printk("ath11k: BHIOFF ilegible (%#x): ¿el bus responde?\n", ab->bhi_off);
        return -1;
    }
    if (ab->mmio_len && ab->bhi_off >= ab->mmio_len) {
        lx_printk("ath11k: BHIOFF %#x fuera del BAR (%#x)\n", ab->bhi_off, ab->mmio_len);
        return -1;
    }
    lx_printk("ath11k: BHIOFF %#x BHIEOFF %#x\n", ab->bhi_off, ab->bhie_off);

    /* INTVEC antes de nada (pm.c:1107). */
    ath11k_write32(ab, ab->bhi_off + BHI_INTVEC, 0);

    ab->phase = "execenv";
    ab->ee = ath11k_mhi_get_ee(ab);
    ab->state = ath11k_mhi_get_state(ab);
    lx_printk("ath11k: EE=%s estado=%s\n", ath11k_ee_str(ab->ee),
              ath11k_state_str(ab->state));
    if (!MHI_POWER_UP_CAPABLE(ab->ee)) {
        lx_printk("ath11k: EE %s no permite encender\n", ath11k_ee_str(ab->ee));
        return -1;
    }

    if (ab->state == MHI_STATE_SYS_ERR) {
        ab->phase = "syserr_reset";
        lx_printk("ath11k: SYS_ERR al arrancar; reset del MHI\n");
        if (ath11k_mhi_reset(ab))
            return -1;
    }

    ab->phase = "ready";
    if (ath11k_mhi_wait_ready(ab))
        return -1;

    ab->phase = "ctxt";
    if (ath11k_mhi_alloc_ctxt(ab)) {
        lx_printk("ath11k: sin memoria para los contextos MHI\n");
        return -1;
    }
    if (ath11k_mhi_init_mmio(ab))
        return -1;

    /* Con el dispositivo en PBL toca descargar el firmware; si ya está en
     * AMSS (rearranque en caliente) se salta ese paso. */
    if (MHI_IN_PBL(ab->ee)) {
        ab->phase = "fw_download";
        if (ath11k_mhi_fw_download(ab, fw, fw_len))
            return -1;
        ab->phase = "espera_amss";
        for (unsigned int i = 0; i < FW_TRIES; i++) {
            enum mhi_ee ee = ath11k_mhi_get_ee(ab);
            if (ee == MHI_EE_AMSS || ee == MHI_EE_WFW) {
                ab->ee = ee;
                break;
            }
            lx_mdelay(POLL_INTERVAL_MS);
        }
        if (ab->ee != MHI_EE_AMSS && ab->ee != MHI_EE_WFW) {
            lx_printk("ath11k: el firmware no llegó a AMSS (EE=%s)\n",
                      ath11k_ee_str(ath11k_mhi_get_ee(ab)));
            return -1;
        }
        lx_printk("ath11k: EE=%s tras la descarga\n", ath11k_ee_str(ab->ee));
    }

    ab->phase = "m0";
    if (ath11k_mhi_set_state(ab, MHI_STATE_M0))
        return -1;
    if (ath11k_mhi_poll_field(ab, MHISTATUS, MHISTATUS_MHISTATE_MASK,
                              MHISTATUS_MHISTATE_SHIFT, MHI_STATE_M0, POLL_TRIES)) {
        lx_printk("ath11k: el dispositivo no llegó a M0\n");
        return -1;
    }
    ab->state = MHI_STATE_M0;
    ab->phase = "mission_mode";
    lx_printk("ath11k: MHI en M0, EE=%s — mission mode\n", ath11k_ee_str(ab->ee));
    return 0;
}
