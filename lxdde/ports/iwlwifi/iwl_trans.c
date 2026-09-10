/* Transporte Gen2/Gen3: context-info, anillos MTR/MCR, arranque firmware. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);

static uint32_t iwl_read32(struct iwl_ax211_priv *iwl, uint32_t off)
{
    return iwl->mmio[off / 4];
}

static void iwl_write32(struct iwl_ax211_priv *iwl, uint32_t off, uint32_t val)
{
    iwl->mmio[off / 4] = val;
}

static void iwl_write8(struct iwl_ax211_priv *iwl, uint32_t off, uint8_t val)
{
    volatile uint8_t *p = (volatile uint8_t *)&iwl->mmio[off / 4];

    p[off & 3u] = val;
}

static void iwl_write64(struct iwl_ax211_priv *iwl, uint32_t off, uint64_t val)
{
    iwl_write32(iwl, off, (uint32_t)(val & 0xffffffffu));
    iwl_write32(iwl, off + 4, (uint32_t)(val >> 32));
}

static uint32_t iwl_prph_msk(struct iwl_ax211_priv *iwl)
{
    /* AX210/SO (gen3): 24 bits; 22000/AX200 (gen2): 20 bits. */
    return iwl->gen3 ? IWL_PRPH_MSK_GEN3 : IWL_PRPH_MSK_GEN2;
}

static uint32_t iwl_umac_prph(struct iwl_ax211_priv *iwl, uint32_t addr)
{
    return iwl->gen3 ? addr + IWL_UMAC_PRPH_OFFSET : addr;
}

static void iwl_write_prph_no_grab(struct iwl_ax211_priv *iwl, uint32_t addr, uint32_t val)
{
    iwl_write32(iwl, HBUS_TARG_PRPH_WADDR, (addr & iwl_prph_msk(iwl)) | (3u << 24));
    iwl_write32(iwl, HBUS_TARG_PRPH_WDATA, val);
}

static uint32_t iwl_read_prph_no_grab(struct iwl_ax211_priv *iwl, uint32_t addr)
{
    iwl_write32(iwl, HBUS_TARG_PRPH_RADDR, (addr & iwl_prph_msk(iwl)) | (3u << 24));
    return iwl_read32(iwl, HBUS_TARG_PRPH_RDAT);
}

static int iwl_poll_bit(struct iwl_ax211_priv *iwl, uint32_t off, uint32_t mask,
                        uint32_t want, int ms)
{
    while (ms-- > 0) {
        if ((iwl_read32(iwl, off) & mask) == want)
            return 0;
        lx_mdelay(1);
    }
    return -1;
}

static void iwl_set_bit(struct iwl_ax211_priv *iwl, uint32_t off, uint32_t mask)
{
    iwl_write32(iwl, off, iwl_read32(iwl, off) | mask);
}

static void iwl_clear_bit(struct iwl_ax211_priv *iwl, uint32_t off, uint32_t mask)
{
    iwl_write32(iwl, off, iwl_read32(iwl, off) & ~mask);
}

static int iwl_grab_nic_access(struct iwl_ax211_priv *iwl)
{
    uint32_t mask = CSR_GP_CNTRL_REG_FLAG_MAC_CLOCK_READY |
                    CSR_GP_CNTRL_REG_FLAG_GOING_TO_SLEEP;
    uint32_t poll = CSR_GP_CNTRL_REG_FLAG_MAC_CLOCK_READY;

    iwl_set_bit(iwl, CSR_GP_CNTRL, CSR_GP_CNTRL_REG_FLAG_MAC_ACCESS_REQ);
    lx_udelay(2);
    if (iwl_poll_bit(iwl, CSR_GP_CNTRL, mask, poll, 15) != 0) {
        lx_printk("iwl_trans: grab_nic_access timeout GP=0x%08x\n",
                  iwl_read32(iwl, CSR_GP_CNTRL));
        return -1;
    }
    return 0;
}

static void iwl_release_nic_access(struct iwl_ax211_priv *iwl)
{
    iwl_clear_bit(iwl, CSR_GP_CNTRL, CSR_GP_CNTRL_REG_FLAG_MAC_ACCESS_REQ);
}

static void iwl_write_prph(struct iwl_ax211_priv *iwl, uint32_t addr, uint32_t val)
{
    if (iwl_grab_nic_access(iwl) != 0)
        return;
    iwl_write_prph_no_grab(iwl, addr, val);
    iwl_release_nic_access(iwl);
}

static uint32_t iwl_read_prph(struct iwl_ax211_priv *iwl, uint32_t addr)
{
    uint32_t v;

    if (iwl_grab_nic_access(iwl) != 0)
        return IWL_PRPH_HW_TIMEOUT;
    v = iwl_read_prph_no_grab(iwl, addr);
    iwl_release_nic_access(iwl);
    return v;
}

static int iwl_prepare_card_hw(struct iwl_ax211_priv *iwl)
{
    if (iwl_poll_bit(iwl, CSR_HW_IF_CONFIG_REG, CSR_HW_IF_CONFIG_REG_BIT_NIC_READY,
                     CSR_HW_IF_CONFIG_REG_BIT_NIC_READY, 200) == 0)
        return 0;
    iwl_set_bit(iwl, CSR_HW_IF_CONFIG_REG, CSR_HW_IF_CONFIG_REG_BIT_NIC_READY);
    if (iwl_poll_bit(iwl, CSR_HW_IF_CONFIG_REG, CSR_HW_IF_CONFIG_REG_BIT_NIC_READY,
                     CSR_HW_IF_CONFIG_REG_BIT_NIC_READY, 200) != 0) {
        lx_printk("iwl_trans: NIC no listo\n");
        return -1;
    }
    return 0;
}

static void iwl_enable_fw_load_int_ctx_info(struct iwl_ax211_priv *iwl)
{
    uint32_t mask = CSR_INT_BIT_ALIVE | CSR_INT_BIT_FH_RX;

    iwl_write32(iwl, CSR_INT_MASK, mask);
}

static void iwl_pcie_set_ltr(struct iwl_ax211_priv *iwl)
{
    uint32_t ltr_val;

    /* iwl_trans_pcie_set_ltr: familia 22000 (AX200 PCIe), no integrada. */
    if (iwl->gen3)
        return;

    ltr_val = CSR_LTR_LONG_VAL_AD_NO_SNOOP_REQ |
              (CSR_LTR_LONG_VAL_AD_SCALE_USEC << 26) |
              (250u << 16) |
              CSR_LTR_LONG_VAL_AD_SNOOP_REQ |
              (CSR_LTR_LONG_VAL_AD_SCALE_USEC << 10) |
              250u;
    iwl_write32(iwl, CSR_LTR_LONG_VAL_AD, ltr_val);
}

static void iwl_sw_reset(struct iwl_ax211_priv *iwl)
{
    /* iwl_trans_pcie_sw_reset: pulso SW_RESET, sin escribir 0 después. */
    iwl_set_bit(iwl, CSR_RESET, CSR_RESET_REG_FLAG_SW_RESET);
    lx_udelay(5000);
}

static int iwl_finish_nic_init(struct iwl_ax211_priv *iwl)
{
    /* iwl_finish_nic_init: set_bit INIT_DONE, poll MAC_CLOCK_READY 25 s. */
    iwl_set_bit(iwl, CSR_GP_CNTRL, CSR_GP_CNTRL_REG_FLAG_INIT_DONE);
    if (iwl_poll_bit(iwl, CSR_GP_CNTRL, CSR_GP_CNTRL_REG_FLAG_MAC_CLOCK_READY,
                     CSR_GP_CNTRL_REG_FLAG_MAC_CLOCK_READY, 25000) != 0) {
        lx_printk("iwl_trans: MAC no despertó (finish_nic_init GP=0x%08x)\n",
                  iwl_read32(iwl, CSR_GP_CNTRL));
        return -1;
    }
    return 0;
}

static int iwl_gen2_apm_init(struct iwl_ax211_priv *iwl)
{
    /* iwl_pcie_gen2_apm_init + iwl_finish_nic_init (22000: sin APMG_CLK). */
    iwl_set_bit(iwl, CSR_GIO_CHICKEN_BITS, CSR_GIO_CHICKEN_BITS_REG_BIT_L1A_NO_L0S_RX);
    iwl_set_bit(iwl, CSR_DBG_HPET_MEM_REG, CSR_DBG_HPET_MEM_REG_VAL);
    iwl_set_bit(iwl, CSR_HW_IF_CONFIG_REG, CSR_HW_IF_CONFIG_REG_BIT_HAP_WAKE_L1A);
    iwl_set_bit(iwl, CSR_GIO_REG, CSR_GIO_REG_VAL_L0S_DISABLED);
    return iwl_finish_nic_init(iwl);
}

static int iwl_clear_persistence_bit(struct iwl_ax211_priv *iwl)
{
    uint32_t hpm;
    uint32_t wprot;

    /* Linux `iwl_trans_pcie_clear_persistence_bit` solo toca familias 9000 y
     * 22000. AX210/SO (gen3) sale al instante. */
    if (iwl->gen3)
        return 0;

    /* Linux: PRPH sin grab. El MAC aún no tiene reloj (INIT_DONE va en APM);
     * grab_nic_access espera MAC_CLOCK_READY y abortaba (GP=0x08040008). */
    hpm = iwl_read_prph_no_grab(iwl, HPM_DEBUG);
    if (hpm == IWL_PRPH_HW_TIMEOUT || hpm == 0xa5a5a5a2u) {
        return 0;
    }
    if (!(hpm & PERSISTENCE_BIT)) {
        return 0;
    }
    wprot = iwl_read_prph_no_grab(iwl, PREG_PRPH_WPROT_22000);
    if (wprot & PREG_WFPM_ACCESS) {
        lx_printk("iwl_trans: persistence bit bloqueado (WPROT=0x%x)\n", wprot);
        return 0;
    }
    iwl_write_prph_no_grab(iwl, HPM_DEBUG, hpm & ~PERSISTENCE_BIT);
    return 0;
}

static int iwl_pcie_check_hw_rf_kill(struct iwl_ax211_priv *iwl)
{
    uint32_t gp = iwl_read32(iwl, CSR_GP_CNTRL);

    if (!(gp & CSR_GP_CNTRL_REG_FLAG_HW_RF_KILL_SW)) {
        lx_printk("iwl_trans: RF-kill hardware activo (GP_CNTRL=0x%08x)\n", gp);
        return -1;
    }
    return 0;
}

static int iwl_alloc_queues(struct iwl_ax211_priv *iwl)
{
    uint64_t *bd;
    unsigned i;
    unsigned used_sz = iwl->gen3 ? (IWL_GEN2_RX_N * 2u) : (IWL_GEN2_RX_N * 4u);

    iwl->rx_bd_cpu = lx_dma_alloc_coherent(0, IWL_GEN2_RX_N * 8, &iwl->rx_bd_dma, GFP_KERNEL);
    iwl->used_bd_cpu = lx_dma_alloc_coherent(0, used_sz, &iwl->used_bd_dma, GFP_KERNEL);
    iwl->rb_stts = (volatile uint16_t *)lx_dma_alloc_coherent(0, 16, &iwl->rb_stts_dma, GFP_KERNEL);
    iwl->rx_page_cpu = lx_dma_alloc_coherent(0, IWL_GEN2_RX_N * IWL_GEN2_RX_SZ,
                                             &iwl->rx_page_dma, GFP_KERNEL);
    iwl->mtr_cpu = lx_dma_alloc_coherent(0, IWL_CMD_QUEUE_SIZE * IWL_TFH_TFD_SIZE,
                                         &iwl->mtr_dma, GFP_KERNEL);
    iwl->mcr_cpu = lx_dma_alloc_coherent(0, IWL_CMD_QUEUE_SIZE * IWL_CMD_SLOT_SIZE,
                                         &iwl->mcr_dma, GFP_KERNEL);
    if (!iwl->rx_bd_cpu || !iwl->used_bd_cpu || !iwl->rb_stts ||
        !iwl->rx_page_cpu || !iwl->mtr_cpu || !iwl->mcr_cpu)
        return -1;
    memset(iwl->rx_bd_cpu, 0, IWL_GEN2_RX_N * 8);
    memset(iwl->used_bd_cpu, 0, used_sz);
    memset((void *)iwl->rb_stts, 0, 16);
    memset(iwl->mtr_cpu, 0, IWL_CMD_QUEUE_SIZE * IWL_TFH_TFD_SIZE);
    memset(iwl->mcr_cpu, 0, IWL_CMD_QUEUE_SIZE * IWL_CMD_SLOT_SIZE);
    bd = (uint64_t *)iwl->rx_bd_cpu;
    if (iwl->gen3) {
        for (i = 0; i < IWL_GEN2_RX_N - 1; i++)
            bd[i] = iwl->rx_page_dma + (uint64_t)i * IWL_GEN2_RX_SZ;
        iwl->rx_write = IWL_GEN2_RX_N - 1;
    } else {
        /* iwl_pcie_restock_bd (22000): RBD = page_dma | vid, vid = i + 1.
         * WIDX empieza en N-1 como gen3 — con 0 el FW no recibe RBD y no ALIVE. */
        for (i = 0; i < IWL_GEN2_RX_N; i++)
            bd[i] = (iwl->rx_page_dma + (uint64_t)i * IWL_GEN2_RX_SZ) | (uint64_t)(i + 1u);
        iwl->rx_write = IWL_GEN2_RX_N - 1;
    }
    iwl->rx_read = 0;
    iwl->cmd_write = 0;
    return 0;
}

static void parse_scan_complete(struct iwl_ax211_priv *iwl, const uint8_t *data, int len)
{
    if (len < (int)sizeof(struct iwl_umac_scan_complete))
        return;
    const struct iwl_umac_scan_complete *n = (const struct iwl_umac_scan_complete *)data;
    iwl_mvm_on_scan_complete(iwl, n->uid, n->status);
    lx_printk("iwl_trans: SCAN_COMPLETE status=%u uid=0x%x count=%d end=%d\n",
              n->status, (unsigned)n->uid, iwl->scan_count, iwl->scan_end);
}

static void parse_offload_match(struct iwl_ax211_priv *iwl, const uint8_t *data, int len)
{
    /* Perfil match: BSSID @0, canal @8 en match v1 */
    if (len < 24)
        return;
    struct iwl_ax211_bss bss;
    memset(&bss, 0, sizeof(bss));
    memcpy(bss.bssid, data, 6);
    bss.channel = data[8];
    bss.rssi = (int8_t)(0 - (int)data[9]);
    bss.open = 1;
    bss.ssid[0] = '?';
    bss.ssid[1] = '\0';
    iwl_ax211_add_bss(&bss);
    (void)iwl;
}

static void parse_rx_phy(struct iwl_ax211_priv *iwl, const uint8_t *data, int len)
{
    if (len < 20)
        return;
    iwl->last_rx_band24 = data[0] & 1u;
    iwl->last_rx_channel = (uint8_t)(data[1] | (data[2] << 8));
    if (len >= 48) {
        uint32_t energy = (uint32_t)data[44] | ((uint32_t)data[45] << 8) |
                          ((uint32_t)data[46] << 16) | ((uint32_t)data[47] << 24);
        int a = (int)((energy >> 0) & 0xffu);
        int b = (int)((energy >> 8) & 0xffu);
        if (a)
            a = -a;
        else
            a = -100;
        if (b)
            b = -b;
        else
            b = -100;
        iwl->last_rx_rssi = (int8_t)(a > b ? a : b);
    }
}

static void parse_rx_mpdu(struct iwl_ax211_priv *iwl, const uint8_t *data, int len)
{
    unsigned desc_size = iwl->gen3 ? (unsigned)sizeof(struct iwl_rx_mpdu_res_start)
                                   : IWL_RX_DESC_SIZE_V1;
    int flen;
    const uint8_t *frame;

    if (len < (int)desc_size)
        return;
    if (iwl->gen3) {
        const struct iwl_rx_mpdu_res_start *res =
            (const struct iwl_rx_mpdu_res_start *)data;
        flen = (int)res->byte_count;
    } else {
        flen = (int)(data[0] | ((uint16_t)data[1] << 8));
    }
    frame = data + desc_size;
    if (flen <= 0 || desc_size + (size_t)flen > (size_t)len)
        return;
    iwl_mvm_rx_scan_frame(iwl, frame, flen);
}

/* --- Propiedad de slots de la cola de comandos (R4) ---------------------
 *
 * Modelo de `pcie/tx-gen2.c`: anillo FIFO estricto con productor
 * (`cmd_write`) y consumidor (`cmd_read`). Los slots [cmd_read, cmd_write)
 * están en vuelo; el FW consume TFDs en orden, así que la liberación es
 * también en orden. Un slot no vuelve a usarse hasta que se libera: nada
 * de saltar huecos, porque el hardware procesaría los TFD intermedios.
 */

static unsigned cmd_q_used(const struct iwl_ax211_priv *iwl)
{
    return (unsigned)((iwl->cmd_write - iwl->cmd_read) & (IWL_CMD_QUEUE_SIZE - 1u));
}

unsigned iwl_trans_cmd_space(struct iwl_ax211_priv *iwl)
{
    /* Se deja un hueco para distinguir vacío de lleno, como iwl_txq_space. */
    return (unsigned)(IWL_CMD_QUEUE_SIZE - 1u) - cmd_q_used(iwl);
}

int iwl_trans_needs_recover(struct iwl_ax211_priv *iwl)
{
    return iwl->cmd_needs_recover ? 1 : 0;
}

/* Marca el slot como respondido y libera desde la cabeza los que ya lo estén.
 * Solo se recicla DMA en orden: un slot en vuelo o envenenado por delante
 * detiene la liberación, aunque su respuesta llegue más tarde. */
static void cmd_slot_done(struct iwl_ax211_priv *iwl, unsigned slot)
{
    unsigned guard = 0;

    iwl->cmd_slot_state[slot] = IWL_SLOT_DONE;
    while (cmd_q_used(iwl) > 0 && guard++ < IWL_CMD_QUEUE_SIZE) {
        unsigned r = iwl->cmd_read % IWL_CMD_QUEUE_SIZE;

        if (iwl->cmd_slot_state[r] != IWL_SLOT_DONE)
            break;
        iwl->cmd_slot_state[r] = IWL_SLOT_FREE;
        iwl->cmd_read = (uint16_t)((iwl->cmd_read + 1u) & (IWL_CMD_QUEUE_SIZE - 1u));
    }
}

int iwl_trans_recover(struct iwl_ax211_priv *iwl)
{
    unsigned i;

    if (!iwl)
        return -1;
    for (i = 0; i < IWL_CMD_QUEUE_SIZE; i++) {
        iwl->cmd_slot_state[i] = IWL_SLOT_FREE;
        iwl->cmd_slot_seq[i] = 0;
        iwl->cmd_slot_group[i] = 0;
        iwl->cmd_slot_id[i] = 0;
    }
    iwl->cmd_read = 0;
    iwl->cmd_write = 0;
    iwl->cmd_poisoned = 0;
    iwl->cmd_pending = 0;
    iwl->cmd_status = 0;
    iwl->cmd_fw_err = 0;
    iwl->cmd_resp_len = 0;
    iwl->cmd_resp_wire_len = 0;
    iwl->cmd_resp_trunc = 0;
    iwl->cmd_needs_recover = 0;
    iwl->cmd_recover++;
    /* MVM abajo: sin ALIVE la próxima llamada rehace carga de FW e init.
     * Reutilizar el anillo sin reiniciar el FW desincronizaría los índices. */
    iwl->alive = 0;
    iwl->init_complete = 0;
    iwl->radio_ready = 0;
    iwl->mvm_up_done = 0;
    iwl->phy_ctxt_added = 0;
    iwl->scan_cfg_sent = 0;
    iwl->scan_active = 0;
    iwl->mcc_done = 0;
    iwl->nvm_ready = 0;
    lx_iwlwifi_set_alive(0);
    lx_printk("iwl_trans: recuperación #%u — cola liberada, MVM abajo\n",
              (unsigned)iwl->cmd_recover);
    return 0;
}

static void log_rx(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd,
                   uint16_t seq, int pay, int status)
{
    lx_printk("iwl_rx: grp=%u id=0x%02x seq=0x%04x len=%d st=%d\n",
              (unsigned)group, (unsigned)cmd, (unsigned)seq, pay, status);
    (void)iwl;
}

/* Devuelve el slot propietario de esta respuesta, o -1 si no es respuesta a
 * un comando en vuelo (notificación, opcode ajeno, secuencia rancia o slot
 * envenenado). La identidad se comprueba contra el slot, no contra un único
 * `cmd_pending`: así conviven sync y async intercalados. */
static int rx_owner_slot(struct iwl_ax211_priv *iwl, uint8_t group,
                         uint8_t cmd, uint16_t seq)
{
    unsigned slot;

    if (seq & SEQ_RX_FRAME)
        return -1;
    if (SEQ_TO_QUEUE(seq) != (unsigned)(iwl->cmd_qid & 0x1fu))
        return -1;
    slot = SEQ_TO_INDEX(seq) % IWL_CMD_QUEUE_SIZE;
    if (iwl->cmd_slot_state[slot] != IWL_SLOT_SYNC &&
        iwl->cmd_slot_state[slot] != IWL_SLOT_ASYNC)
        return -1;
    if (iwl->cmd_slot_seq[slot] != seq)
        return -1;
    if (iwl->cmd_slot_group[slot] != group || iwl->cmd_slot_id[slot] != cmd)
        return -1;
    return (int)slot;
}

/* `avail` = bytes válidos realmente recibidos en `buf` (incluida la cabecera
 * de 8 B). El RB de hardware siempre trae IWL_GEN2_RX_SZ; una inyección corta
 * (banco de pruebas, DMA parcial) no debe leerse más allá de `avail`. */
static void handle_gen2_rx(struct iwl_ax211_priv *iwl, const uint8_t *buf, unsigned avail)
{
    uint32_t len_n_flags;
    uint16_t len;
    uint8_t cmd;
    uint8_t group;
    uint16_t seq;
    const uint8_t *data;
    int pay;
    int copy;

    if (avail < 8u) {
        iwl->rx_trunc_drop++;
        return;
    }
    len_n_flags = (uint32_t)buf[0] | ((uint32_t)buf[1] << 8) |
                  ((uint32_t)buf[2] << 16) | ((uint32_t)buf[3] << 24);
    len = (uint16_t)(len_n_flags & 0x3fff);
    cmd = buf[4];
    group = buf[5];
    seq = (uint16_t)buf[6] | ((uint16_t)buf[7] << 8);
    data = buf + 8;
    pay = (int)len - 4;

    if (pay < 0)
        pay = 0;
    /* Longitud anunciada mayor que la recibida: no se recorta en silencio, se
     * descarta. Recortar dejaba pasar respuestas incompletas como válidas. */
    if ((unsigned)pay > avail - 8u) {
        iwl->rx_trunc_drop++;
        log_rx(iwl, group, cmd, seq, pay, -1);
        lx_printk("iwl_rx: truncado grp=%u id=0x%02x len=%d recibidos=%u; descartado\n",
                  (unsigned)group, (unsigned)cmd, pay, avail - 8u);
        return;
    }

    log_rx(iwl, group, cmd, seq, pay, 0);

    {
    int owner = rx_owner_slot(iwl, group, cmd, seq);

    if (owner >= 0 && iwl->cmd_slot_state[owner] == IWL_SLOT_ASYNC) {
        /* Async: nadie espera la respuesta, pero el slot solo se libera aquí
         * y no toca cmd_resp[], que pertenece al comando sincrónico. */
        cmd_slot_done(iwl, (unsigned)owner);
    } else if (owner >= 0) {
        iwl->cmd_status = 1;
        iwl->cmd_pending = 0;
        /* Gen2 iwlwifi: tamaño en bits 13:0; no hay bit FAILED en len_n_flags
         * (iwlegacy usaba hdr.flags). NVM_GET_INFO v4 = 468 B → len=472 y
         * 472&0x40≠0 si se interpretaba como rechazo — falso positivo run14. */
        iwl->cmd_fw_err = 0;
        iwl->cmd_resp_wire_len = (uint16_t)pay;
        copy = pay;
        if (copy > (int)sizeof(iwl->cmd_resp))
            copy = (int)sizeof(iwl->cmd_resp);
        iwl->cmd_resp_trunc = (copy < pay) ? 1 : 0;
        if (iwl->cmd_resp_trunc)
            lx_printk("iwl_rx: respuesta grp=%u id=0x%02x de %d B > buffer %u B\n",
                      (unsigned)group, (unsigned)cmd, pay,
                      (unsigned)sizeof(iwl->cmd_resp));
        if (copy > 0) {
            memcpy(iwl->cmd_resp, data, (size_t)copy);
            iwl->cmd_resp_len = (uint16_t)copy;
        } else {
            iwl->cmd_resp_len = 0;
        }
        cmd_slot_done(iwl, (unsigned)owner);
    }
    }

    if (group == 0 && cmd == UCODE_ALIVE_NTFY) {
        iwl->alive = 1;
        lx_iwlwifi_set_alive(1);
        lx_printk("iwl_ax211: firmware ALIVE (UCODE_ALIVE_NTFY)\n");
        return;
    }
    if (group == LEGACY_GROUP && cmd == INIT_COMPLETE_NOTIF) {
        iwl->init_complete = 1;
        lx_printk("iwl_mvm: INIT_COMPLETE_NOTIF\n");
        return;
    }
    if (cmd == SCAN_COMPLETE_UMAC &&
        (group == LEGACY_GROUP || group == LONG_GROUP))
        parse_scan_complete(iwl, data, pay);
    if (group == SCAN_GROUP && cmd == OFFLOAD_MATCH_INFO_NOTIF)
        parse_offload_match(iwl, data, pay);
    if (group == LEGACY_GROUP && cmd == REPLY_RX_PHY_CMD)
        parse_rx_phy(iwl, data, pay);
    if (group == LEGACY_GROUP && cmd == REPLY_RX_MPDU_CMD)
        parse_rx_mpdu(iwl, data, pay);
    if (group == DATA_PATH_GROUP && cmd == 0x1 && pay > 14)
        iwl_ax211_deliver_rx(data, pay);
}

static void drain_rx_gen2(struct iwl_ax211_priv *iwl)
{
    uint64_t *bd;
    uint16_t hw;
    int n = 0;

    if (!iwl->rb_stts || !iwl->used_bd_cpu || !iwl->rx_page_cpu || !iwl->rx_bd_cpu)
        return;
    bd = (uint64_t *)iwl->rx_bd_cpu;
    hw = iwl->rb_stts[0] & 0x0fff;
    while (iwl->rx_read != hw && n++ < IWL_GEN2_RX_N) {
        if (iwl->gen3) {
            uint16_t *used = (uint16_t *)iwl->used_bd_cpu;
            uint16_t idx = used[iwl->rx_read % IWL_GEN2_RX_N] & 0x0fff;

            if (idx < IWL_GEN2_RX_N) {
                handle_gen2_rx(iwl,
                               (const uint8_t *)iwl->rx_page_cpu +
                                   (size_t)idx * IWL_GEN2_RX_SZ,
                               IWL_GEN2_RX_SZ);
                bd[iwl->rx_write % IWL_GEN2_RX_N] =
                    iwl->rx_page_dma + (uint64_t)idx * IWL_GEN2_RX_SZ;
                iwl->rx_write = (uint16_t)((iwl->rx_write + 1) % IWL_GEN2_RX_N);
            }
        } else {
            uint32_t *used32 = (uint32_t *)iwl->used_bd_cpu;
            uint32_t cd = used32[iwl->rx_read % IWL_GEN2_RX_N];
            uint16_t vid = (uint16_t)(cd & 0x0fffu);
            uint16_t idx;

            if (vid == 0 || vid > IWL_GEN2_RX_N)
                goto next_slot;
            idx = (uint16_t)(vid - 1u);
            handle_gen2_rx(iwl,
                           (const uint8_t *)iwl->rx_page_cpu +
                               (size_t)idx * IWL_GEN2_RX_SZ,
                           IWL_GEN2_RX_SZ);
            bd[iwl->rx_write % IWL_GEN2_RX_N] =
                (iwl->rx_page_dma + (uint64_t)idx * IWL_GEN2_RX_SZ) | (uint64_t)vid;
            iwl->rx_write = (uint16_t)((iwl->rx_write + 1) % IWL_GEN2_RX_N);
        }
next_slot:
        iwl->rx_read = (uint16_t)((iwl->rx_read + 1) % IWL_GEN2_RX_N);
        hw = iwl->rb_stts[0] & 0x0fff;
    }
    iwl_write32(iwl, RFH_Q0_FRBDCB_WIDX_TRG, (uint32_t)(iwl->rx_write & ~7u));
}

int iwl_trans_gen2_start(struct iwl_ax211_priv *iwl)
{
    struct iwl_context_info *ctxt;

    /* iwl_trans_pcie_gen2_start_fw */
    if (iwl_prepare_card_hw(iwl) != 0)
        return -1;
    iwl_sw_reset(iwl);
    if (iwl_prepare_card_hw(iwl) != 0)
        return -1;
    if (iwl_clear_persistence_bit(iwl) != 0)
        return -1;

    iwl_write32(iwl, CSR_INT, 0xffffffffu);
    if (iwl_pcie_check_hw_rf_kill(iwl) != 0)
        return -1;

    iwl_write32(iwl, CSR_UCODE_DRV_GP1_CLR, CSR_UCODE_SW_BIT_RFKILL);
    iwl_write32(iwl, CSR_UCODE_DRV_GP1_CLR, CSR_UCODE_DRV_GP1_BIT_CMD_BLOCKED);
    iwl_write32(iwl, CSR_INT, 0xffffffffu);

    if (iwl_gen2_apm_init(iwl) != 0)
        return -1;
    lx_printk("iwl_trans: APM ok GP=0x%08x\n", iwl_read32(iwl, CSR_GP_CNTRL));
    iwl_write8(iwl, CSR_INT_COALESCING, IWL_HOST_INT_TIMEOUT_DEF);
    if (iwl_pcie_check_hw_rf_kill(iwl) != 0)
        return -1;
    if (iwl_alloc_queues(iwl) != 0)
        return -1;

    iwl_set_bit(iwl, CSR_MAC_SHADOW_REG_CTRL, 0x800fffffu);

    ctxt = lx_dma_alloc_coherent(0, sizeof(*ctxt), &iwl->ctxt_dma, GFP_KERNEL);
    if (!ctxt)
        return -1;
    memset(ctxt, 0, sizeof(*ctxt));

    iwl_write32(iwl, RFH_Q0_FRBDCB_WIDX_TRG, (uint32_t)(iwl->rx_write & ~7u));

    ctxt->version.mac_id = (uint16_t)iwl_read32(iwl, CSR_HW_REV);
    ctxt->version.version = 0;
    ctxt->version.size = (uint16_t)(sizeof(*ctxt) / 4);
    ctxt->control.control_flags = IWL_CTXT_INFO_TFD_FORMAT_LONG |
                                  IWL_CTXT_INFO_RB_CB_SIZE_32 |
                                  IWL_CTXT_INFO_RB_SIZE_4K;
    ctxt->rbd_cfg.free_rbd_addr = iwl->rx_bd_dma;
    ctxt->rbd_cfg.used_rbd_addr = iwl->used_bd_dma;
    ctxt->rbd_cfg.status_wr_ptr = iwl->rb_stts_dma;
    ctxt->hcmd_cfg.cmd_queue_addr = iwl->mtr_dma;
    ctxt->hcmd_cfg.cmd_queue_size = TFD_QUEUE_CB_SIZE_32;

    if (iwl_fw_upload_sections(iwl, &ctxt->dram) != 0)
        return -1;

    iwl_write32(iwl, CSR_INT, 0xffffffffu);
    iwl_enable_fw_load_int_ctx_info(iwl);
    iwl_write64(iwl, CSR_CTXT_INFO_BA, iwl->ctxt_dma);
    lx_printk("iwlwifi: context-info gen2 BA=0x%llx\n", (unsigned long long)iwl->ctxt_dma);

    iwl_pcie_set_ltr(iwl);
    iwl_write_prph(iwl, UREG_CPU_INIT_RUN, 1);
    lx_printk("iwlwifi: UREG_CPU_INIT_RUN=0x%x\n",
              iwl_read_prph(iwl, UREG_CPU_INIT_RUN));

    for (int t = 0; t < 500; t++) {
        uint32_t inta = iwl_read32(iwl, CSR_INT);

        if (inta & CSR_INT_BIT_ALIVE) {
            iwl_write32(iwl, RFH_Q0_FRBDCB_WIDX_TRG,
                        (uint32_t)(iwl->rx_write & ~7u));
        }
        /* ALIVE vive en el anillo RX; drenar aunque CSR_INT siga a 0. */
        drain_rx_gen2(iwl);
        if (iwl->alive)
            return 0;
        lx_mdelay(10);
    }
    {
        uint32_t inta = iwl_read32(iwl, CSR_INT);
        uint32_t gp = iwl_read32(iwl, CSR_GP_CNTRL);
        uint16_t rb_hw = iwl->rb_stts ? (iwl->rb_stts[0] & 0x0fffu) : 0;

        lx_printk("iwl_trans: timeout ALIVE (AX200 gen2) INT=0x%08x GP=0x%08x "
                  "rb_hw=0x%03x rx_read=%u rx_write=%u WIDX=0x%x%s%s\n",
                  inta, gp, rb_hw, (unsigned)iwl->rx_read,
                  (unsigned)iwl->rx_write,
                  (unsigned)(iwl->rx_write & ~7u),
                  (inta & CSR_INT_BIT_SW_ERR) ? " SW_ERR" : "",
                  !(gp & CSR_GP_CNTRL_REG_FLAG_HW_RF_KILL_SW) ? " RF_KILL" : "");
    }
    return -1;
}

int iwl_trans_gen3_start(struct iwl_ax211_priv *iwl)
{
    uint64_t dma = 0;
    uint64_t iml_dma = 0;
    struct iwl_prph_scratch *scratch;
    void *info;
    struct iwl_context_info_gen3 *ctxt;
    void *iml_cpu;
    uint32_t gp;

    /* El mismo preámbulo que gen2: `iwl_apm_init` reescribía GP_CNTRL entero
     * (sin chicken bits / HAP_WAKE) y el IML no arrancaba (INT=0, rb_hw=0). */
    if (iwl_prepare_card_hw(iwl) != 0)
        return -1;
    iwl_sw_reset(iwl);
    if (iwl_prepare_card_hw(iwl) != 0)
        return -1;
    if (iwl_clear_persistence_bit(iwl) != 0)
        return -1;
    iwl_write32(iwl, CSR_INT, 0xffffffffu);
    if (iwl_pcie_check_hw_rf_kill(iwl) != 0)
        return -1;
    iwl_write32(iwl, CSR_UCODE_DRV_GP1_CLR, CSR_UCODE_SW_BIT_RFKILL);
    iwl_write32(iwl, CSR_UCODE_DRV_GP1_CLR, CSR_UCODE_DRV_GP1_BIT_CMD_BLOCKED);
    iwl_write32(iwl, CSR_INT, 0xffffffffu);
    if (iwl_gen2_apm_init(iwl) != 0)
        return -1;
    iwl_write8(iwl, CSR_INT_COALESCING, IWL_HOST_INT_TIMEOUT_DEF);
    if (iwl_pcie_check_hw_rf_kill(iwl) != 0)
        return -1;
    if (iwl_alloc_queues(iwl) != 0)
        return -1;
    iwl_set_bit(iwl, CSR_MAC_SHADOW_REG_CTRL, 0x800fffffu);
    if (!iwl->iml || !iwl->iml_len) {
        lx_printk("iwlwifi: sin IML en el firmware\n");
        return -1;
    }

    scratch = lx_dma_alloc_coherent(0, sizeof(*scratch), &iwl->scratch_dma, GFP_KERNEL);
    info = lx_dma_alloc_coherent(0, IWL_PRPH_INFO_ALLOC, &iwl->info_dma, GFP_KERNEL);
    ctxt = lx_dma_alloc_coherent(0, sizeof(*ctxt), &iwl->ctxt_dma, GFP_KERNEL);
    iml_cpu = lx_dma_alloc_coherent(0, iwl->iml_len, &iml_dma, GFP_KERNEL);
    if (!scratch || !info || !ctxt || !iml_cpu)
        return -1;

    memset(scratch, 0, sizeof(*scratch));
    scratch->ctrl_cfg.version.mac_id = (uint16_t)iwl_read32(iwl, CSR_HW_REV);
    scratch->ctrl_cfg.version.version = 0;
    scratch->ctrl_cfg.version.size = sizeof(*scratch) / 4;
    scratch->ctrl_cfg.control.control_flags = IWL_PRPH_SCRATCH_RB_SIZE_4K |
                                              IWL_PRPH_SCRATCH_MTR_MODE |
                                              IWL_PRPH_MTR_FORMAT_256B;
    scratch->ctrl_cfg.rbd_cfg.free_rbd_addr = iwl->rx_bd_dma;

    if (iwl->pnvm_data && iwl->pnvm_len) {
        void *pnvm_cpu = lx_dma_alloc_coherent(0, iwl->pnvm_len, &dma, GFP_KERNEL);
        if (pnvm_cpu) {
            memcpy(pnvm_cpu, iwl->pnvm_data, iwl->pnvm_len);
            scratch->ctrl_cfg.pnvm_cfg.pnvm_base_addr = dma;
            scratch->ctrl_cfg.pnvm_cfg.pnvm_size = (uint32_t)iwl->pnvm_len;
        }
    }

    if (iwl_fw_upload_sections(iwl, &scratch->dram) != 0)
        return -1;

    memset(info, 0, IWL_PRPH_INFO_ALLOC);
    memset(ctxt, 0, sizeof(*ctxt));
    /* Linux deja version/size/config/idx_arr_size a 0 (dma_alloc_coherent). */
    ctxt->prph_info_base_addr = iwl->info_dma;
    ctxt->prph_scratch_base_addr = iwl->scratch_dma;
    ctxt->prph_scratch_size = sizeof(*scratch);
    ctxt->cr_head_idx_arr_base_addr = iwl->rb_stts_dma;
    ctxt->tr_tail_idx_arr_base_addr = iwl->info_dma + IWL_PRPH_INFO_ALLOC / 2;
    ctxt->cr_tail_idx_arr_base_addr = iwl->info_dma + 3 * IWL_PRPH_INFO_ALLOC / 4;
    ctxt->mtr_base_addr = iwl->mtr_dma;
    ctxt->mcr_base_addr = iwl->used_bd_dma;
    ctxt->mtr_size = TFD_QUEUE_CB_SIZE_32;
    ctxt->mcr_size = RX_QUEUE_CB_SIZE_32;

    memcpy(iml_cpu, iwl->iml, iwl->iml_len);
    iwl_write32(iwl, CSR_INT, 0xffffffffu);
    iwl_enable_fw_load_int_ctx_info(iwl);
    iwl_write64(iwl, CSR_CTXT_INFO_ADDR, iwl->ctxt_dma);
    iwl_write64(iwl, CSR_IML_DATA_ADDR, iml_dma);
    iwl_write32(iwl, CSR_IML_SIZE_ADDR, (uint32_t)iwl->iml_len);
    /* CSR_CTXT_INFO_BOOT_CTRL == CSR_HW_IF_CONFIG_REG. Linux hace set_bit;
     * un write32 de solo BOOT_ENA borra NIC_READY y HAP_WAKE_L1A. */
    iwl_set_bit(iwl, CSR_CTXT_INFO_BOOT_CTRL, CSR_AUTO_FUNC_BOOT_ENA);
    /* Linux `iwl_trans_pcie_gen2_start_fw` (familia AX210): UREG_CPU_INIT_RUN
     * vía umac_prph (+0x300000). No escribe AUTO_FUNC_INIT en GP_CNTRL. */
    iwl_write_prph(iwl, iwl_umac_prph(iwl, UREG_CPU_INIT_RUN), 1);
    gp = iwl_read32(iwl, CSR_GP_CNTRL);
    lx_printk("iwlwifi: context-info gen3 BA=0x%llx iml=%lu HW_IF=0x%08x GP=0x%08x\n",
              (unsigned long long)iwl->ctxt_dma, iwl->iml_len,
              iwl_read32(iwl, CSR_HW_IF_CONFIG_REG), gp);

    for (int i = 0; i < 500; i++) {
        drain_rx_gen2(iwl);
        if (iwl->alive)
            return 0;
        lx_mdelay(10);
    }
    {
        uint32_t inta = iwl_read32(iwl, CSR_INT);
        uint32_t gp = iwl_read32(iwl, CSR_GP_CNTRL);
        uint16_t rb_hw = iwl->rb_stts ? (iwl->rb_stts[0] & 0x0fffu) : 0;
        struct iwl_prph_info *pi = (struct iwl_prph_info *)info;

        lx_printk("iwl_trans: timeout ALIVE INT=0x%08x GP=0x%08x "
                  "rb_hw=0x%03x rx_read=%u rx_write=%u boot=0x%x ipc=0x%x%s%s\n",
                  inta, gp, rb_hw, (unsigned)iwl->rx_read,
                  (unsigned)iwl->rx_write,
                  pi ? pi->boot_stage_mirror : 0,
                  pi ? pi->ipc_status_mirror : 0,
                  (inta & CSR_INT_BIT_SW_ERR) ? " SW_ERR" : "",
                  !(gp & CSR_GP_CNTRL_REG_FLAG_HW_RF_KILL_SW) ? " RF_KILL" : "");
    }
    return -1;
}

void iwl_trans_poll(struct iwl_ax211_priv *iwl)
{
    if (!iwl->mmio)
        return;
    drain_rx_gen2(iwl);
    (void)iwl_read32(iwl, CSR_INT);
}

void iwl_trans_rx_packet(struct iwl_ax211_priv *iwl, const uint8_t *buf, unsigned len)
{
    uint8_t tmp[IWL_GEN2_RX_SZ];

    if (!iwl || !buf)
        return;
    if (len < 8) {
        iwl->rx_trunc_drop++;
        return;
    }
    if (len > IWL_GEN2_RX_SZ)
        len = IWL_GEN2_RX_SZ;
    memcpy(tmp, buf, len);
    if (len < IWL_GEN2_RX_SZ)
        memset(tmp + len, 0, IWL_GEN2_RX_SZ - len);
    /* Se pasa `len`, no el tamaño del RB: el relleno a cero no es payload. */
    handle_gen2_rx(iwl, tmp, len);
}

static int send_hcmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                     const void *payload, uint16_t pay_len, int async)
{
    uint16_t slot;
    uint8_t *buf;
    struct iwl_tfh_tfd_long *tfd;
    uint16_t total;
    uint16_t seq;

    if (!iwl->alive || !iwl->mtr_cpu || !iwl->mcr_cpu)
        return -1;
    /* Un único propietario para envío/RX/poll: una llamada anidada (desde el
     * drenaje RX o una IRQ) no reserva slot, se rechaza. */
    if (iwl->in_trans) {
        iwl->cmd_reentry_reject++;
        lx_printk("iwl_trans: envío reentrante rechazado (grp=%u id=0x%02x)\n",
                  (unsigned)group, (unsigned)id);
        return -1;
    }
    /* Tras un timeout la cola queda bloqueada a propósito: hasta recuperar el
     * transporte no se admiten comandos nuevos ni se recicla el DMA en uso. */
    if (iwl->cmd_needs_recover) {
        lx_printk("iwl_trans: cola bloqueada; hace falta iwl_trans_recover()\n");
        return -1;
    }
    /* Solo un comando sincrónico en vuelo: cmd_resp[] tiene un dueño. Los
     * async sí pueden intercalarse, cada uno con su propio slot. */
    if (!async && iwl->cmd_pending)
        return -1;
    {
        uint32_t total32 = (uint32_t)sizeof(struct iwl_cmd_header_wide) + (uint32_t)pay_len;

        if (total32 > IWL_CMD_SLOT_SIZE)
            return -1;
        total = (uint16_t)total32;
    }

    /* Reserva: comprobar espacio y tomar el slot ocurre sin ceder el control
     * (ni retardos ni RX en medio), con el propietario ya marcado. */
    iwl->in_trans = 1;
    if (iwl_trans_cmd_space(iwl) == 0) {
        iwl->cmd_backpressure++;
        iwl->in_trans = 0;
        lx_printk("iwl_trans: cola llena (%u en vuelo); backpressure\n",
                  cmd_q_used(iwl));
        return -1;
    }
    slot = iwl->cmd_write % IWL_CMD_QUEUE_SIZE;
    if (iwl->cmd_slot_state[slot] != IWL_SLOT_FREE) {
        iwl->cmd_backpressure++;
        iwl->in_trans = 0;
        lx_printk("iwl_trans: slot %u aún en uso (estado %u)\n",
                  (unsigned)slot, (unsigned)iwl->cmd_slot_state[slot]);
        return -1;
    }
    buf = (uint8_t *)iwl->mcr_cpu + (size_t)slot * IWL_CMD_SLOT_SIZE;
    tfd = (struct iwl_tfh_tfd_long *)((uint8_t *)iwl->mtr_cpu + (size_t)slot * IWL_TFH_TFD_SIZE);
    memset(buf, 0, IWL_CMD_SLOT_SIZE);
    memset(tfd, 0, sizeof(*tfd));

    seq = (uint16_t)(QUEUE_TO_SEQ(iwl->cmd_qid) | INDEX_TO_SEQ(slot));
    /* Linux `iwl_trans_send_cmd` (`iwl-trans.c`): con cabecera wide, los HCMD del
     * LEGACY_GROUP (API grp=0) salen con group_id=LONG_GROUP (DEF_ID). */
    {
        uint8_t wire_group = group;

        if (group == LEGACY_GROUP) {
            wire_group = LONG_GROUP;
        }
        /* Propiedad del slot: vale igual para async (nadie espera la
         * respuesta, pero el DMA sigue siendo suyo hasta liberarlo). */
        iwl->cmd_slot_state[slot] = async ? IWL_SLOT_ASYNC : IWL_SLOT_SYNC;
        iwl->cmd_slot_group[slot] = wire_group;
        iwl->cmd_slot_id[slot] = id;
        iwl->cmd_slot_seq[slot] = seq;
        if (!async) {
            iwl->cmd_pending = 1;
            iwl->cmd_status = 0;
            iwl->cmd_fw_err = 0;
            iwl->cmd_resp_len = 0;
            iwl->cmd_pending_group = wire_group;
            iwl->cmd_pending_id = id;
            iwl->cmd_pending_seq = seq;
        }

    /* Gen2/Gen3: siempre cabecera wide (Linux `pcie/tx-gen2.c`), también grupo 0
     * (TX_ANT, SF, PHY_CONTEXT). La ruta legacy de 4 B dejaba al FW sordo tras SF. */
        {
        struct iwl_cmd_header_wide *whdr = (struct iwl_cmd_header_wide *)buf;
        whdr->cmd = id;
        whdr->group_id = wire_group;
        whdr->sequence = seq;
        whdr->length = iwl_cpu_to_le16(pay_len);
        whdr->reserved = 0;
        whdr->version = (uint8_t)iwl_fw_cmd_ver(iwl, group, id);
        if (pay_len)
            memcpy(buf + sizeof(*whdr), payload, pay_len);
        if (id == TX_ANT_CONFIGURATION_CMD) {
            lx_printk("iwl_trans: TX_ANT wide ver=%u len=%u (hdr 8 B + payload %u B)\n",
                      (unsigned)whdr->version, (unsigned)total, (unsigned)pay_len);
        }
        }
    }

    tfd->num_tbs = 1;
    tfd->tbs[0].tb_len = total;
    tfd->tbs[0].addr = iwl->mcr_dma + (uint64_t)slot * IWL_CMD_SLOT_SIZE;
    iwl->cmd_write = (uint16_t)((iwl->cmd_write + 1u) & (IWL_CMD_QUEUE_SIZE - 1u));
    {
        uint32_t doorbell = ((uint32_t)iwl->cmd_write & 0xffu) |
                            ((uint32_t)iwl->cmd_qid << 16);
        static int cmd_doorbell_logged;

        if (!cmd_doorbell_logged) {
            lx_printk("iwl_trans: send_cmd qid=%u doorbell=0x%08x seq=0x%04x "
                      "grp=%u id=0x%02x%s\n",
                      (unsigned)iwl->cmd_qid, doorbell, (unsigned)seq,
                      (unsigned)group, (unsigned)id,
                      async ? " async" : "");
            cmd_doorbell_logged = 1;
        }
        iwl_write32(iwl, HBUS_TARG_WRPTR, doorbell);
    }
    drain_rx_gen2(iwl);
    iwl->cmd_seq++;
    iwl->in_trans = 0;
    return 0;
}

int iwl_trans_send_cmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                       const void *payload, uint16_t pay_len)
{
    return send_hcmd(iwl, group, id, payload, pay_len, 0);
}

int iwl_trans_send_cmd_async(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                             const void *payload, uint16_t pay_len)
{
    return send_hcmd(iwl, group, id, payload, pay_len, 1);
}

int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t pay_len, int wait_ms)
{
    int t;

    if (iwl_trans_send_cmd(iwl, group, id, payload, pay_len) != 0)
        return -1;

    for (t = 0; t < wait_ms; t++) {
        iwl_trans_poll(iwl);
        if (iwl->cmd_status) {
            if (iwl->cmd_fw_err)
                return -1;
            return 0;
        }
        lx_mdelay(1);
    }
    {
        unsigned slot = SEQ_TO_INDEX(iwl->cmd_pending_seq) % IWL_CMD_QUEUE_SIZE;
        uint8_t log_grp = iwl->cmd_pending_group;

        /* El slot queda envenenado: el FW puede escribir su DMA en cualquier
         * momento, así que no se recicla ni se salta. La cola se bloquea y
         * solo iwl_trans_recover() (reinicio de transporte) la libera. */
        if (iwl->cmd_slot_state[slot] == IWL_SLOT_SYNC ||
            iwl->cmd_slot_state[slot] == IWL_SLOT_ASYNC) {
            iwl->cmd_slot_state[slot] = IWL_SLOT_POISON;
            iwl->cmd_poisoned++;
        }
        iwl->cmd_pending = 0;
        iwl->cmd_needs_recover = 1;
        /* MVM parado: no se encadenan más etapas sobre una cola bloqueada. */
        iwl->mvm_up_done = 0;
        iwl->scan_active = 0;
        if (iwl->mmio)
            drain_rx_gen2(iwl);
        lx_printk("iwl_trans: timeout cmd grp=%u id=0x%02x slot=%u; MVM parado\n",
                  (unsigned)log_grp, (unsigned)id, slot);
    }
    return -1;
}
