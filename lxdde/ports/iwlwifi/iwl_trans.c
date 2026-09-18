/* Transporte Gen2/Gen3: context-info, anillos MTR/MCR, arranque firmware. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);

static void iwl_trans_pnvm_free_dma(struct iwl_ax211_priv *iwl);

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

/* Tamaño de cada entrada del anillo RX, por generación (ver iwl_internal.h). */
static unsigned iwl_rx_bd_size(const struct iwl_ax211_priv *iwl)
{
    return iwl->gen3 ? IWL_RX_BD_SIZE_GEN3 : IWL_RX_BD_SIZE_GEN2;
}

static unsigned iwl_rx_cd_size(const struct iwl_ax211_priv *iwl)
{
    return iwl->gen3 ? IWL_RX_CD_SIZE_GEN3 : IWL_RX_CD_SIZE_GEN2;
}

/* Publica el buffer `vid - 1` en la ranura `slot` del anillo de BD libres. */
static void iwl_rx_post_bd(struct iwl_ax211_priv *iwl, unsigned slot, uint16_t vid)
{
    uint64_t addr;

    if (vid == 0 || vid > IWL_GEN2_RX_N)
        return;
    addr = iwl->rx_page_dma + (uint64_t)(vid - 1u) * IWL_GEN2_RX_SZ;
    slot %= IWL_GEN2_RX_N;
    if (iwl->gen3) {
        struct iwl_rx_transfer_desc *bd =
            (struct iwl_rx_transfer_desc *)iwl->rx_bd_cpu;

        memset(&bd[slot], 0, sizeof(bd[slot]));
        bd[slot].rbid = vid;
        bd[slot].addr = addr;
    } else {
        uint64_t *bd = (uint64_t *)iwl->rx_bd_cpu;

        bd[slot] = addr | (uint64_t)vid;
    }
}

/* VID del descriptor completado `slot`, o 0 si la ranura no designa buffer. */
static uint16_t iwl_rx_completed_vid(const struct iwl_ax211_priv *iwl, unsigned slot)
{
    slot %= IWL_GEN2_RX_N;
    if (iwl->gen3) {
        const struct iwl_rx_completion_desc *cd =
            (const struct iwl_rx_completion_desc *)iwl->used_bd_cpu;

        return cd[slot].rbid;
    }
    {
        const uint32_t *cd = (const uint32_t *)iwl->used_bd_cpu;

        return (uint16_t)(cd[slot] & 0x0fffu);
    }
}

/* Anillos RX/comandos del transporte.
 *
 * Idempotente: en un reinicio de transporte (R4) se reutilizan los mismos
 * buffers en vez de pedir otros. Pedirlos de nuevo perdía ~400 KiB de DMA
 * coherente por recuperación y dejaba los antiguos aún mapeados para el
 * dispositivo. */
static int iwl_alloc_queues(struct iwl_ax211_priv *iwl)
{
    unsigned i;
    unsigned bd_sz = IWL_GEN2_RX_N * iwl_rx_bd_size(iwl);
    unsigned used_sz = IWL_GEN2_RX_N * iwl_rx_cd_size(iwl);

    if (iwl->rx_bd_cpu && iwl->used_bd_cpu && iwl->rb_stts && iwl->rx_page_cpu &&
        iwl->mtr_cpu && iwl->mcr_cpu && iwl->hcmd_first_tb_cpu) {
        lx_printk("iwlwifi: anillos ya reservados; se reinician en su sitio\n");
        goto reiniciar;
    }
    iwl->rx_bd_cpu = lx_dma_alloc_coherent(0, bd_sz, &iwl->rx_bd_dma, GFP_KERNEL);
    iwl->used_bd_cpu = lx_dma_alloc_coherent(0, used_sz, &iwl->used_bd_dma, GFP_KERNEL);
    iwl->rb_stts = (volatile uint16_t *)lx_dma_alloc_coherent(0, 16, &iwl->rb_stts_dma, GFP_KERNEL);
    iwl->rx_page_cpu = lx_dma_alloc_coherent(0, IWL_GEN2_RX_N * IWL_GEN2_RX_SZ,
                                             &iwl->rx_page_dma, GFP_KERNEL);
    iwl->mtr_cpu = lx_dma_alloc_coherent(0, IWL_CMD_QUEUE_SIZE * IWL_TFH_TFD_SIZE,
                                         &iwl->mtr_dma, GFP_KERNEL);
    iwl->mcr_cpu = lx_dma_alloc_coherent(0, IWL_CMD_QUEUE_SIZE * IWL_CMD_SLOT_SIZE,
                                         &iwl->mcr_dma, GFP_KERNEL);
    iwl->hcmd_first_tb_cpu = lx_dma_alloc_coherent(
        0, IWL_CMD_QUEUE_SIZE * IWL_FIRST_TB_SIZE_ALIGN, &iwl->hcmd_first_tb_dma,
        GFP_KERNEL);
    if (!iwl->rx_bd_cpu || !iwl->used_bd_cpu || !iwl->rb_stts ||
        !iwl->rx_page_cpu || !iwl->mtr_cpu || !iwl->mcr_cpu ||
        !iwl->hcmd_first_tb_cpu)
        return -1;
reiniciar:
    memset(iwl->rx_bd_cpu, 0, bd_sz);
    memset(iwl->used_bd_cpu, 0, used_sz);
    memset((void *)iwl->rb_stts, 0, 16);
    memset(iwl->mtr_cpu, 0, IWL_CMD_QUEUE_SIZE * IWL_TFH_TFD_SIZE);
    memset(iwl->mcr_cpu, 0, IWL_CMD_QUEUE_SIZE * IWL_CMD_SLOT_SIZE);
    memset(iwl->hcmd_first_tb_cpu, 0,
           IWL_CMD_QUEUE_SIZE * IWL_FIRST_TB_SIZE_ALIGN);
    /* `iwl_pcie_restock_bd`: el VID identifica el buffer y vale i + 1 en las dos
     * generaciones; sólo cambia dónde se escribe. WIDX empieza en N-1 — con 0 el
     * FW no recibe ningún RBD y no manda ALIVE. */
    for (i = 0; i < IWL_GEN2_RX_N; i++)
        iwl_rx_post_bd(iwl, i, (uint16_t)(i + 1u));
    iwl->rx_write = IWL_GEN2_RX_N - 1;
    iwl->rx_read = 0;
    iwl->cmd_write = 0;
    iwl->cmd_read = 0;
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

static void apply_rx_rssi(struct iwl_ax211_priv *iwl, uint8_t a, uint8_t b)
{
    int ea = a ? -(int)a : -100;
    int eb = b ? -(int)b : -100;

    iwl->last_rx_rssi = (int8_t)(ea > eb ? ea : eb);
}

static void parse_rx_phy(struct iwl_ax211_priv *iwl, const uint8_t *data, int len)
{
    uint8_t ch;
    uint8_t a = 0;
    uint8_t b = 0;

    ch = iwl_rx_phy_info_channel(data, len);
    if (ch)
        iwl->last_rx_channel = ch;
    if (len >= (int)(offsetof(struct iwl_rx_phy_info, phy_flags) + 2))
        iwl->last_rx_band24 = (iwl_rx_phy_info_flags(data, len) & RX_RES_PHY_FLAGS_BAND_24)
                                  ? 1u
                                  : 0u;
    if (iwl_rx_phy_info_energy(data, len, &a, &b) == 0 && (a || b))
        apply_rx_rssi(iwl, a, b);
}

/* Reparte una trama Ethernet ya convertida entre el supplicant y la pila IP. */
static void deliver_eth(struct iwl_ax211_priv *iwl, const uint8_t *eth, int len)
{
    uint16_t ethertype;

    if (!eth || len < 14)
        return;
    ethertype = (uint16_t)(((uint16_t)eth[12] << 8) | eth[13]);
    iwl->rx_data_ok++;
    if (ethertype == ETH_P_EAPOL)
        iwl_ax211_deliver_eapol(eth, len);
    else
        iwl_ax211_deliver_rx(eth, len);
}

static void parse_rx_mpdu(struct iwl_ax211_priv *iwl, const uint8_t *data, int len)
{
    /* Linux mvm/rxmq.c `iwl_mvm_rx_mpdu_mq`: el descriptor que precede a la
     * MPDU es `sizeof(struct iwl_rx_mpdu_desc)` en AX210+ y IWL_RX_DESC_SIZE_V1
     * antes. No son 4 bytes en ninguna de las dos. */
    unsigned desc_size = iwl->gen3 ? IWL_RX_DESC_SIZE_V3 : IWL_RX_DESC_SIZE_V1;
    uint32_t status;
    int flen;
    const uint8_t *frame;
    uint8_t ch;
    uint8_t a = 0;
    uint8_t b = 0;

    if (len < (int)desc_size)
        return;
    flen = (int)(data[0] | ((uint16_t)data[1] << 8));
    status = iwl_rx_mpdu_status(data, len);
    if (iwl->gen3) {
        ch = iwl_rx_mpdu_v3_channel(data, len);
        iwl_rx_mpdu_v3_energy(data, len, &a, &b);
    } else {
        ch = iwl_rx_mpdu_v1_channel(data, len);
        iwl_rx_mpdu_v1_energy(data, len, &a, &b);
    }
    if (ch) {
        iwl->last_rx_channel = ch;
        iwl->last_rx_band24 = (ch > 14u) ? 0u : 1u;
    }
    if (a || b)
        apply_rx_rssi(iwl, a, b);

    frame = data + desc_size;
    if (flen <= 0 || desc_size + (size_t)flen > (size_t)len)
        return;
    if (flen >= 24 && (iwl->bssid[0] || iwl->bssid[1] || iwl->bssid[2] ||
                       iwl->bssid[3] || iwl->bssid[4] || iwl->bssid[5])) {
        uint16_t fc = (uint16_t)frame[0] | ((uint16_t)frame[1] << 8);
        uint16_t stype = fc & IEEE80211_FC_STYPE_MASK;
        int bi;

        if (stype == IEEE80211_STYPE_BEACON) {
            for (bi = 0; bi < 6; bi++) {
                if (frame[16 + bi] != iwl->bssid[bi])
                    break;
            }
            if (bi == 6) {
                uint64_t tsf = 0;
                uint32_t gp2 = 0;

                iwl_rx_mpdu_beacon_sync(data, len, iwl->gen3, frame, flen,
                                        &tsf, &gp2);
                if (tsf)
                    iwl->sync_tsf = tsf;
                if (gp2)
                    iwl->sync_device_ts = gp2;
            }
        }
    }
    iwl_mvm_rx_scan_frame(iwl, frame, flen);
    iwl_mvm_rx_mlme_frame(iwl, frame, flen);

    /* Camino de datos: hasta ahora la MPDU sólo llegaba al scan y a MLME, así
     * que ni EAPOL ni IP salían nunca del driver. */
    if (iwl->associated) {
        uint8_t eth[IWL_MAX_ETH_FRAME];
        int n = iwl_mvm_rx_to_eth(iwl, frame, flen, status, eth, (int)sizeof(eth));

        if (n > 0)
            deliver_eth(iwl, eth, n);
        else if (n == -2 || n == -3 || n == -4 || n == -5)
            iwl->rx_data_drop++;
    }
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
    iwl->pnvm_complete = 0;
    iwl_trans_pnvm_free_dma(iwl);
    if (iwl->scratch_cpu) {
        struct iwl_prph_scratch *scratch = (struct iwl_prph_scratch *)iwl->scratch_cpu;

        scratch->ctrl_cfg.pnvm_cfg.pnvm_base_addr = 0;
        scratch->ctrl_cfg.pnvm_cfg.pnvm_size = 0;
    }
    iwl->sku_id[0] = 0;
    iwl->sku_id[1] = 0;
    iwl->sku_id[2] = 0;
    iwl->radio_ready = 0;
    iwl->mvm_up_done = 0;
    iwl->phy_ctxt_added = 0;
    iwl->mac_ctxt_added = 0;
    iwl->binding_added = 0;
    iwl->fw_link_id = IWL_MVM_FW_LINK_ID_INVALID;
    iwl->link_active = 0;
    iwl->phy_channel = 0;
    iwl->phy_band = 0;
    iwl->mlme_auth_ok = 0;
    iwl->mlme_assoc_ok = 0;
    iwl->dtim_period = 0;
    iwl->beacon_int = 0;
    iwl->assoc_id = 0;
    iwl->lar_regdom_set = 0;
    iwl->scan_cfg_sent = 0;
    iwl->scan_active = 0;
    iwl->mcc_done = 0;
    iwl->nvm_ready = 0;
    iwl->mgmt_txq_ready = 0;
    iwl->mgmt_txq_id = 0;
    iwl->mgmt_txq_write = 0;
    iwl->mgmt_txq_read = 0;
    iwl->data_txq_ready = 0;
    iwl->data_txq_id = 0;
    iwl->data_txq_write = 0;
    iwl->data_txq_read = 0;
    iwl->keys_installed = 0;
    iwl->authorized = 0;
    iwl->scan_count = 0;
    iwl->scan_complete = 0;
    iwl->scan_end = IWL_SCAN_END_NONE;
    iwl->scan_active = 0;
    lx_iwlwifi_set_alive(0);
    lx_printk("iwl_trans: recuperación #%u — cola liberada, MVM abajo\n",
              (unsigned)iwl->cmd_recover);
    return 0;
}

static int hcmd_scd_pending(const struct iwl_ax211_priv *iwl)
{
    return iwl->cmd_pending &&
           iwl->cmd_pending_group == DATA_PATH_GROUP &&
           iwl->cmd_pending_id == SCD_QUEUE_CONFIG_CMD;
}

static void log_rx(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd,
                   uint16_t seq, int pay, int status)
{
    /* Linux IWL_DEBUG_RX: sin CONFIG_IWLWIFI_DEBUG no hay traza por paquete. */
    if (status == 0 && !hcmd_scd_pending(iwl))
        return;
    if (hcmd_scd_pending(iwl)) {
        static unsigned scd_wait_rx_logged;

        if (scd_wait_rx_logged++)
            return;
        lx_printk("iwl_rx: [SCD-wait] grp=%u id=0x%02x seq=0x%04x len=%d st=%d "
                  "(esperando seq=0x%04x)\n",
                  (unsigned)group, (unsigned)cmd, (unsigned)seq, pay, status,
                  (unsigned)iwl->cmd_pending_seq);
    } else {
        lx_printk("iwl_rx: grp=%u id=0x%02x seq=0x%04x len=%d st=%d\n",
                  (unsigned)group, (unsigned)cmd, (unsigned)seq, pay, status);
    }
}

static int iwl_mvm_has_new_tx_api(struct iwl_ax211_priv *iwl)
{
    if (!iwl)
        return 0;
    /* SCD ver 3 (TVQM): struct iwl_mvm_tx_resp status @40, no v3 @36. */
    return iwl_fw_cmd_ver(iwl, DATA_PATH_GROUP, SCD_QUEUE_CONFIG_CMD) == 3;
}

static unsigned tx_resp_status_off(struct iwl_ax211_priv *iwl)
{
    if (iwl_mvm_has_new_tx_api(iwl) || iwl->gen3)
        return IWL_MVM_TX_RESP_STATUS_OFF;
    return IWL_MVM_TX_RESP_V3_STATUS_OFF;
}

static uint32_t tx_resp_status_word(struct iwl_ax211_priv *iwl,
                                    const uint8_t *data, int pay)
{
    unsigned off;

    if (!data || pay < (int)IWL_MVM_TX_RESP_MIN_PAY)
        return 0xffffffffu;
    off = tx_resp_status_off(iwl);
    if (pay < (int)(off + (int)sizeof(struct agg_tx_status)))
        return 0xffffffffu;
    return (uint32_t)(data[off] | ((uint32_t)data[off + 1] << 8));
}

static uint16_t *txq_read_ptr(struct iwl_ax211_priv *iwl, uint16_t qid)
{
    if (iwl->data_txq_ready && qid == iwl->data_txq_id)
        return &iwl->data_txq_read;
    if (iwl->mgmt_txq_ready && qid == iwl->mgmt_txq_id)
        return &iwl->mgmt_txq_read;
    return 0;
}

static uint16_t *txq_write_ptr(struct iwl_ax211_priv *iwl, uint16_t qid)
{
    if (iwl->data_txq_ready && qid == iwl->data_txq_id)
        return &iwl->data_txq_write;
    if (iwl->mgmt_txq_ready && qid == iwl->mgmt_txq_id)
        return &iwl->mgmt_txq_write;
    return 0;
}

static unsigned txq_space_one(uint16_t write, uint16_t read)
{
    unsigned used = (unsigned)((write - read) & (IWL_MGMT_QUEUE_SIZE - 1u));

    return (IWL_MGMT_QUEUE_SIZE - 1u) - used;
}

static unsigned txq_space_id(struct iwl_ax211_priv *iwl, uint16_t qid)
{
    uint16_t *readp = txq_read_ptr(iwl, qid);
    uint16_t *writep = txq_write_ptr(iwl, qid);

    if (!readp || !writep)
        return 0;
    return txq_space_one(*writep, *readp);
}

/* Espacio libre del anillo TX mgmt (compat hostcheck). */
unsigned iwl_trans_tx_space(const struct iwl_ax211_priv *iwl)
{
    if (!iwl || !iwl->mgmt_txq_ready)
        return 0;
    return txq_space_one(iwl->mgmt_txq_write, iwl->mgmt_txq_read);
}

/* Cola de datos (SSH, TCP). `iwl_trans_tx_space` es solo mgmt. */
unsigned iwl_trans_data_tx_space(const struct iwl_ax211_priv *iwl)
{
    if (!iwl || !iwl->data_txq_ready)
        return 0;
    return txq_space_one(iwl->data_txq_write, iwl->data_txq_read);
}

/* Libera hasta el TFD que el firmware acaba de reconocer. La cola es FIFO y el
 * FW responde en orden, así que el índice de la secuencia marca la cabeza. */
void iwl_trans_tx_reclaim(struct iwl_ax211_priv *iwl, uint16_t seq)
{
    uint16_t qid = SEQ_TO_QUEUE(seq);
    uint16_t *readp = txq_read_ptr(iwl, qid);
    uint16_t *writep = txq_write_ptr(iwl, qid);
    unsigned idx;
    unsigned next;

    if (!readp || !writep)
        return;
    idx = (unsigned)SEQ_TO_INDEX(seq) & (IWL_MGMT_QUEUE_SIZE - 1u);
    next = (idx + 1u) & (IWL_MGMT_QUEUE_SIZE - 1u);
    if (((next - *readp) & (IWL_MGMT_QUEUE_SIZE - 1u)) >
        ((*writep - *readp) & (IWL_MGMT_QUEUE_SIZE - 1u)))
        return;
    *readp = (uint16_t)next;
}

static const char *tx_status_name(uint32_t st)
{
    switch (st) {
    case TX_STATUS_SUCCESS:
        return "SUCCESS";
    case TX_STATUS_FAIL_LONG_LIMIT:
        return "FAIL_LONG_LIMIT";
    default:
        return "";
    }
}

static void parse_tx_resp(struct iwl_ax211_priv *iwl, uint8_t group,
                          const uint8_t *data, int pay, uint16_t seq)
{
    uint32_t raw;
    uint32_t st;
    uint16_t resp_qid = 0;
    unsigned off;

    if (!iwl || !data || pay < (int)IWL_MVM_TX_RESP_MIN_PAY) {
        lx_printk("iwl_trans: TX resp corta grp=%u len=%d\n",
                  (unsigned)group, pay);
        return;
    }
    if (iwl_mvm_has_new_tx_api(iwl) && pay >= 38)
        resp_qid = (uint16_t)(data[36] | ((uint16_t)data[37] << 8));
    if (resp_qid)
        seq = (uint16_t)(QUEUE_TO_SEQ(resp_qid) | INDEX_TO_SEQ(seq));
    iwl_trans_tx_reclaim(iwl, seq);
    raw = tx_resp_status_word(iwl, data, pay);
    if (raw == 0xffffffffu) {
        off = tx_resp_status_off(iwl);
        lx_printk("iwl_trans: TX resp status inválido off=%u len=%d\n",
                  off, pay);
        return;
    }
    st = raw & TX_STATUS_MSK;
    iwl->last_mgmt_tx_status = (uint8_t)st;
    /* Linux IWL_DEBUG_TX_REPLY: SUCCESS no va a info. 0x83 y el resto sí. */
    if (st != TX_STATUS_SUCCESS) {
        const char *name = tx_status_name(st);
        uint16_t qid = resp_qid ? resp_qid : SEQ_TO_QUEUE(seq);
        uint16_t *rd = txq_read_ptr(iwl, qid);
        uint16_t *wr = txq_write_ptr(iwl, qid);

        if (name[0])
            lx_printk("iwl_trans: TX resp grp=%u qid=%u frame_count=%u "
                      "status=0x%02x (%s) rd=%u wr=%u\n",
                      (unsigned)group, (unsigned)qid, (unsigned)data[0],
                      (unsigned)st, name,
                      rd ? (unsigned)*rd : 0u, wr ? (unsigned)*wr : 0u);
        else
            lx_printk("iwl_trans: TX resp grp=%u qid=%u frame_count=%u "
                      "status=0x%02x (raw=0x%04x off=%u) rd=%u wr=%u\n",
                      (unsigned)group, (unsigned)qid, (unsigned)data[0],
                      (unsigned)st, (unsigned)raw, tx_resp_status_off(iwl),
                      rd ? (unsigned)*rd : 0u, wr ? (unsigned)*wr : 0u);
    }
}

static void parse_session_prot_notif(struct iwl_ax211_priv *iwl,
                                     const uint8_t *data, int pay)
{
    const struct iwl_mvm_session_prot_notif *n;

    if (!iwl || !data || pay < (int)sizeof(*n))
        return;
    n = (const struct iwl_mvm_session_prot_notif *)(const void *)data;
    lx_printk("iwl_mvm: SESSION_PROTECTION_NOTIF mac_id=0x%08x status=%u start=%u conf_id=%u\n",
              (unsigned)n->mac_id, (unsigned)n->status, (unsigned)n->start,
              (unsigned)n->conf_id);
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
/* Cierra el comando sincrónico con su respuesta. */
static void rx_complete_sync(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd,
                             const uint8_t *data, int pay)
{
    int copy = pay;

    iwl->cmd_status = 1;
    iwl->cmd_pending = 0;
    /* Gen2 iwlwifi: tamaño en bits 13:0; no hay bit FAILED en len_n_flags
     * (iwlegacy usaba hdr.flags). NVM_GET_INFO v4 = 468 B → len=472 y
     * 472&0x40≠0 si se interpretaba como rechazo — falso positivo run14. */
    iwl->cmd_fw_err = 0;
    iwl->cmd_resp_wire_len = (uint16_t)pay;
    if (copy > (int)sizeof(iwl->cmd_resp)) {
        copy = (int)sizeof(iwl->cmd_resp);
    }
    iwl->cmd_resp_trunc = (copy < pay) ? 1 : 0;
    if (iwl->cmd_resp_trunc) {
        lx_printk("iwl_rx: respuesta grp=%u id=0x%02x de %d B > buffer %u B\n",
                  (unsigned)group, (unsigned)cmd, pay,
                  (unsigned)sizeof(iwl->cmd_resp));
    }
    if (copy > 0) {
        memcpy(iwl->cmd_resp, data, (size_t)copy);
        iwl->cmd_resp_len = (uint16_t)copy;
    } else {
        iwl->cmd_resp_len = 0;
    }
}

/* Gen2: el FH escribe en first_tb (DMA bidireccional). Si el anillo RX no
 * entrega la respuesta, completar cuando sequence lleva SEQ_RX_FRAME con el
 * mismo índice/cola que el HCMD sincrónico en vuelo. */
static void poll_hcmd_first_tb(struct iwl_ax211_priv *iwl)
{
    unsigned slot;
    const struct iwl_cmd_header_wide *whdr;
    uint16_t resp_seq;
    uint16_t sent_seq;
    int pay;
    const uint8_t *data;

    if (!iwl->cmd_pending)
        return;

    slot = (unsigned)SEQ_TO_INDEX(iwl->cmd_pending_seq) % IWL_CMD_QUEUE_SIZE;
    sent_seq = iwl->cmd_pending_seq;
    if (iwl->cmd_slot_state[slot] != IWL_SLOT_SYNC)
        return;

    whdr = NULL;
    if (iwl->hcmd_first_tb_cpu) {
        whdr = (const struct iwl_cmd_header_wide *)
            ((const uint8_t *)iwl->hcmd_first_tb_cpu +
             (size_t)slot * IWL_FIRST_TB_SIZE_ALIGN);
    }
    if (!whdr && iwl->mcr_cpu) {
        whdr = (const struct iwl_cmd_header_wide *)
            ((const uint8_t *)iwl->mcr_cpu + (size_t)slot * IWL_CMD_SLOT_SIZE);
    }
    if (!whdr)
        return;

    resp_seq = whdr->sequence;
    if (!(resp_seq & SEQ_RX_FRAME))
        return;
    if ((resp_seq & 0x7fffu) != (sent_seq & 0x7fffu))
        return;
    if (whdr->cmd != iwl->cmd_pending_id ||
        whdr->group_id != iwl->cmd_pending_group)
        return;

    pay = (int)whdr->length;
    if (pay < 0)
        pay = 0;
    data = (const uint8_t *)whdr + sizeof(*whdr);
    if (hcmd_scd_pending(iwl)) {
        lx_printk("iwl_trans: SCD first_tb complete seq=0x%04x len=%d\n",
                  (unsigned)resp_seq, pay);
    }
    rx_complete_sync(iwl, whdr->group_id, whdr->cmd, data, pay);
    cmd_slot_done(iwl, slot);
}

static void handle_gen2_rx(struct iwl_ax211_priv *iwl, const uint8_t *buf, unsigned avail)
{
    uint32_t len_n_flags;
    uint16_t len;
    uint8_t cmd;
    uint8_t group;
    uint16_t seq;
    const uint8_t *data;
    int pay;
    int owner;

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
        lx_printk("iwl_rx: truncado grp=%u id=0x%02x len=%d recibidos=%u; descartado\n",
                  (unsigned)group, (unsigned)cmd, pay, avail - 8u);
        return;
    }

    if (hcmd_scd_pending(iwl))
        log_rx(iwl, group, cmd, seq, pay, 0);

    /* TX_CMD (0x1c) es notificación de cola de datos, no respuesta HCMD. */
    if (cmd == TX_CMD) {
        if (group == LEGACY_GROUP || group == LONG_GROUP)
            parse_tx_resp(iwl, group, data, pay, seq);
        else
            lx_printk("iwl_trans: TX_CMD grp=%u inesperado len=%d\n",
                      (unsigned)group, pay);
        return;
    }

    owner = rx_owner_slot(iwl, group, cmd, seq);
    if (owner < 0 && iwl->cmd_pending) {
        lx_printk("iwl_rx: sin emparejar (esperando grp=%u id=0x%02x seq=0x%04x)\n",
                  (unsigned)iwl->cmd_pending_group, (unsigned)iwl->cmd_pending_id,
                  (unsigned)iwl->cmd_pending_seq);
    }
    if (owner >= 0) {
        /* Async: nadie espera la respuesta y no toca cmd_resp[], que pertenece
         * al comando sincrónico; pero su slot solo se libera aquí. */
        if (iwl->cmd_slot_state[owner] != IWL_SLOT_ASYNC) {
            rx_complete_sync(iwl, group, cmd, data, pay);
        }
        cmd_slot_done(iwl, (unsigned)owner);
    }

    if (group == 0 && cmd == UCODE_ALIVE_NTFY) {
        iwl->alive = 1;
        lx_iwlwifi_set_alive(1);
        if (pay >= (int)IWL_ALIVE_NTFY_V5_LEN) {
            const uint8_t *sku = data + IWL_ALIVE_SKU_OFF;

            iwl->sku_id[0] = (uint32_t)sku[0] | ((uint32_t)sku[1] << 8) |
                             ((uint32_t)sku[2] << 16) | ((uint32_t)sku[3] << 24);
            iwl->sku_id[1] = (uint32_t)sku[4] | ((uint32_t)sku[5] << 8) |
                             ((uint32_t)sku[6] << 16) | ((uint32_t)sku[7] << 24);
            iwl->sku_id[2] = (uint32_t)sku[8] | ((uint32_t)sku[9] << 8) |
                             ((uint32_t)sku[10] << 16) | ((uint32_t)sku[11] << 24);
            lx_printk("iwl_ax211: firmware ALIVE (UCODE_ALIVE_NTFY) sku=0x%x 0x%x 0x%x\n",
                      (unsigned)iwl->sku_id[0], (unsigned)iwl->sku_id[1],
                      (unsigned)iwl->sku_id[2]);
        } else {
            lx_printk("iwl_ax211: firmware ALIVE (UCODE_ALIVE_NTFY)\n");
        }
        return;
    }
    if (group == REGULATORY_AND_NVM_GROUP && cmd == PNVM_INIT_COMPLETE_NTFY) {
        iwl->pnvm_complete = 1;
        lx_printk("iwl_mvm: PNVM_INIT_COMPLETE_NTFY\n");
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
    if (group == MAC_CONF_GROUP && cmd == SESSION_PROTECTION_NOTIF)
        parse_session_prot_notif(iwl, data, pay);
    /* `DATA_PATH_GROUP` id 1 es UPDATE_MU_GROUPS_CMD, no una notificación de
     * Ethernet: entregar su cuerpo a la pila IP era inventarse paquetes. Los
     * datos llegan por REPLY_RX_MPDU_CMD, arriba. */
}

static void drain_rx_gen2(struct iwl_ax211_priv *iwl);
static uint32_t tx_doorbell(const struct iwl_ax211_priv *iwl, uint16_t qid, uint16_t wr);
static int gen1_tfd_set_tb(struct iwl_tfd *tfd, uint8_t idx, uint64_t addr, uint16_t len);

static void drain_rx(struct iwl_ax211_priv *iwl)
{
    if (iwl->family == IWL_DEVICE_FAMILY_8000)
        iwl_trans_8000_drain(iwl);
    else
        drain_rx_gen2(iwl);
}

static uint32_t tx_doorbell(const struct iwl_ax211_priv *iwl, uint16_t qid, uint16_t wr)
{
    if (iwl->family == IWL_DEVICE_FAMILY_8000)
        return (uint32_t)(wr & 0xffu) | ((uint32_t)qid << 8);
    return (uint32_t)(wr & 0xffu) | ((uint32_t)qid << 16);
}

static int gen1_tfd_set_tb(struct iwl_tfd *tfd, uint8_t idx, uint64_t addr, uint16_t len)
{
    struct iwl_tfd_tb *tb;

    if (idx >= IWL_NUM_OF_TBS)
        return -1;
    tb = &tfd->tbs[idx];
    tb->lo = iwl_cpu_to_le32((uint32_t)(addr & 0xffffffffu));
    tb->hi_n_len = iwl_cpu_to_le16((uint16_t)((len & 0xfffu) << 4) |
                                   (uint16_t)((addr >> 32) & 0xfu));
    tfd->num_tbs = (uint8_t)(idx + 1u);
    return 0;
}

static void drain_rx_gen2(struct iwl_ax211_priv *iwl)
{
    uint16_t hw;
    int n = 0;

    if (!iwl->rb_stts || !iwl->used_bd_cpu || !iwl->rx_page_cpu || !iwl->rx_bd_cpu)
        return;
    hw = iwl_closed_rb_idx(iwl->rb_stts, IWL_GEN2_RX_N);
    while (iwl->rx_read != hw && n++ < IWL_GEN2_RX_N) {
        uint16_t vid = iwl_rx_completed_vid(iwl, iwl->rx_read);

        /* VID 0 o fuera de rango: ranura sin buffer. Se salta sin reciclar,
         * porque reciclar un índice inventado devolvería al FW una dirección
         * que no es de ningún RB. */
        if (vid != 0 && vid <= IWL_GEN2_RX_N) {
            handle_gen2_rx(iwl,
                           (const uint8_t *)iwl->rx_page_cpu +
                               (size_t)(vid - 1u) * IWL_GEN2_RX_SZ,
                           IWL_GEN2_RX_SZ);
            iwl_rx_post_bd(iwl, iwl->rx_write, vid);
            iwl->rx_write = (uint16_t)((iwl->rx_write + 1) % IWL_GEN2_RX_N);
        } else {
            iwl->rx_vid_drop++;
        }
        iwl->rx_read = (uint16_t)((iwl->rx_read + 1) % IWL_GEN2_RX_N);
        hw = iwl_closed_rb_idx(iwl->rb_stts, IWL_GEN2_RX_N);
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

    /* Igual que los anillos: en un reinicio se reutilizan. El IML solo si su
     * tamaño no ha cambiado (otro fichero de firmware). */
    if (iwl->scratch_cpu && iwl->info_cpu && iwl->ctxt_cpu && iwl->iml_cpu &&
        iwl->iml_cpu_len == iwl->iml_len) {
        scratch = iwl->scratch_cpu;
        info = iwl->info_cpu;
        ctxt = iwl->ctxt_cpu;
        iml_cpu = iwl->iml_cpu;
        iml_dma = iwl->iml_dma;
    } else {
        scratch = lx_dma_alloc_coherent(0, sizeof(*scratch), &iwl->scratch_dma, GFP_KERNEL);
        info = lx_dma_alloc_coherent(0, IWL_PRPH_INFO_ALLOC, &iwl->info_dma, GFP_KERNEL);
        ctxt = lx_dma_alloc_coherent(0, sizeof(*ctxt), &iwl->ctxt_dma, GFP_KERNEL);
        iml_cpu = lx_dma_alloc_coherent(0, iwl->iml_len, &iml_dma, GFP_KERNEL);
        if (!scratch || !info || !ctxt || !iml_cpu)
            return -1;
        iwl->scratch_cpu = scratch;
        iwl->info_cpu = info;
        iwl->ctxt_cpu = ctxt;
        iwl->iml_cpu = iml_cpu;
        iwl->iml_dma = iml_dma;
        iwl->iml_cpu_len = iwl->iml_len;
    }

    memset(scratch, 0, sizeof(*scratch));
    scratch->ctrl_cfg.version.mac_id = (uint16_t)iwl_read32(iwl, CSR_HW_REV);
    scratch->ctrl_cfg.version.version = 0;
    scratch->ctrl_cfg.version.size = sizeof(*scratch) / 4;
    scratch->ctrl_cfg.control.control_flags = IWL_PRPH_SCRATCH_RB_SIZE_4K |
                                              IWL_PRPH_SCRATCH_MTR_MODE |
                                              IWL_PRPH_MTR_FORMAT_256B;
    scratch->ctrl_cfg.rbd_cfg.free_rbd_addr = iwl->rx_bd_dma;

    /* Linux gen3: pnvm_cfg queda a cero hasta iwl_trans_pnvm_publish tras ALIVE. */

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

static void iwl_trans_pnvm_free_dma(struct iwl_ax211_priv *iwl)
{
    unsigned i;

    if (!iwl)
        return;
    for (i = 0; i < iwl->pnvm_n_chunks; i++) {
        if (iwl->pnvm_chunks[i].cpu) {
            lx_dma_free_coherent(0, iwl->pnvm_chunks[i].len,
                                 iwl->pnvm_chunks[i].cpu, iwl->pnvm_chunks[i].dma);
            iwl->pnvm_chunks[i].cpu = 0;
            iwl->pnvm_chunks[i].dma = 0;
            iwl->pnvm_chunks[i].len = 0;
        }
    }
    iwl->pnvm_n_chunks = 0;
    if (iwl->pnvm_desc_cpu) {
        lx_dma_free_coherent(0, sizeof(struct iwl_prph_scrath_mem_desc_addr_array),
                             iwl->pnvm_desc_cpu, iwl->pnvm_desc_dma);
        iwl->pnvm_desc_cpu = 0;
        iwl->pnvm_desc_dma = 0;
    }
    if (iwl->pnvm_cont_cpu) {
        lx_dma_free_coherent(0, iwl->pnvm_cont_len, iwl->pnvm_cont_cpu,
                             iwl->pnvm_cont_dma);
        iwl->pnvm_cont_cpu = 0;
        iwl->pnvm_cont_dma = 0;
        iwl->pnvm_cont_len = 0;
    }
    iwl->pnvm_published = 0;
}

int iwl_trans_pnvm_publish(struct iwl_ax211_priv *iwl)
{
    struct iwl_pnvm_image image;
    struct iwl_prph_scratch *scratch;
    unsigned i;
    uint32_t total = 0;
    int fragmented;

    if (!iwl || !iwl->scratch_cpu)
        return -1;
    if (!iwl->pnvm_file || !iwl->pnvm_file_len)
        return 0;

    scratch = (struct iwl_prph_scratch *)iwl->scratch_cpu;
    if (scratch->ctrl_cfg.pnvm_cfg.pnvm_size) {
        lx_printk("iwl_trans: pnvm_cfg ya publicado\n");
        return 0;
    }

    if (iwl_fw_pnvm_select(iwl, &image) != 0)
        return -1;

    iwl_trans_pnvm_free_dma(iwl);
    fragmented = iwl_fw_has_capa(iwl, IWL_UCODE_TLV_CAPA_FRAGMENTED_PNVM_IMG);

    if (fragmented) {
        struct iwl_prph_scrath_mem_desc_addr_array *desc;

        desc = lx_dma_alloc_coherent(0, sizeof(*desc), &iwl->pnvm_desc_dma, GFP_KERNEL);
        if (!desc)
            return -1;
        iwl->pnvm_desc_cpu = desc;
        memset(desc, 0, sizeof(*desc));

        for (i = 0; i < image.n_chunks; i++) {
            void *cpu;
            uint64_t dma;

            cpu = lx_dma_alloc_coherent(0, image.chunks[i].len, &dma, GFP_KERNEL);
            if (!cpu) {
                iwl_trans_pnvm_free_dma(iwl);
                return -1;
            }
            memcpy(cpu, image.chunks[i].data, image.chunks[i].len);
            iwl->pnvm_chunks[i].cpu = cpu;
            iwl->pnvm_chunks[i].dma = dma;
            iwl->pnvm_chunks[i].len = image.chunks[i].len;
            desc->mem_descs[i] = dma;
            total += image.chunks[i].len;
        }
        iwl->pnvm_n_chunks = image.n_chunks;
        scratch->ctrl_cfg.pnvm_cfg.pnvm_base_addr = iwl->pnvm_desc_dma;
        scratch->ctrl_cfg.pnvm_cfg.pnvm_size = total;
        lx_printk("iwl_trans: pnvm fragmentado %u chunks %u bytes (ver=0x%x)\n",
                  (unsigned)image.n_chunks, (unsigned)total,
                  (unsigned)image.version);
    } else {
        void *cpu;
        uint64_t dma;
        uint32_t len0;
        uint32_t len1;

        if (image.n_chunks != UNFRAGMENTED_PNVM_PAYLOADS_NUMBER) {
            lx_printk("iwl_trans: pnvm continuo esperaba 2 chunks, hay %u\n",
                      (unsigned)image.n_chunks);
            return -1;
        }
        len0 = image.chunks[0].len;
        len1 = image.chunks[1].len;
        total = len0 + len1;
        cpu = lx_dma_alloc_coherent(0, total, &dma, GFP_KERNEL);
        if (!cpu)
            return -1;
        memcpy(cpu, image.chunks[0].data, len0);
        memcpy((uint8_t *)cpu + len0, image.chunks[1].data, len1);
        iwl->pnvm_cont_cpu = cpu;
        iwl->pnvm_cont_dma = dma;
        iwl->pnvm_cont_len = total;
        scratch->ctrl_cfg.pnvm_cfg.pnvm_base_addr = dma;
        scratch->ctrl_cfg.pnvm_cfg.pnvm_size = total;
        lx_printk("iwl_trans: pnvm continuo %u bytes (ver=0x%x)\n",
                  (unsigned)total, (unsigned)image.version);
    }

    iwl->pnvm_published = 1;
    return 0;
}

void iwl_trans_pnvm_doorbell(struct iwl_ax211_priv *iwl)
{
    uint32_t addr = iwl_umac_prph(iwl, UREG_DOORBELL_TO_ISR6);

    lx_printk("iwl_trans: PNVM doorbell prph=0x%x val=0x%x\n",
              (unsigned)addr, (unsigned)UREG_DOORBELL_TO_ISR6_PNVM);
    iwl_write_prph(iwl, addr, UREG_DOORBELL_TO_ISR6_PNVM);
}

void iwl_trans_poll(struct iwl_ax211_priv *iwl)
{
    if (!iwl->mmio)
        return;
    if (iwl->family == IWL_DEVICE_FAMILY_8000)
        iwl_trans_8000_drain(iwl);
    else
        drain_rx_gen2(iwl);
    poll_hcmd_first_tb(iwl);
    (void)iwl_read32(iwl, CSR_INT);
}

void iwl_trans_txq_drain_mgmt(struct iwl_ax211_priv *iwl)
{
    unsigned i;

    if (!iwl || !iwl->mgmt_txq_ready)
        return;
    for (i = 0; i < 32u; i++)
        iwl_trans_poll(iwl);
}

void iwl_trans_txq_drain_data(struct iwl_ax211_priv *iwl)
{
    unsigned i;

    if (!iwl || !iwl->data_txq_ready)
        return;
    for (i = 0; i < 32u; i++)
        iwl_trans_poll(iwl);
}

int iwl_trans_wait_mgmt_tx_resp(struct iwl_ax211_priv *iwl, unsigned iters)
{
    unsigned i;
    unsigned p;

    if (!iwl)
        return -1;
    for (i = 0; i < iters; i++) {
        if (iwl->last_mgmt_tx_status != 0)
            return 0;
        for (p = 0; p < 8u; p++)
            iwl_trans_poll(iwl);
        if (iwl->last_mgmt_tx_status != 0)
            return 0;
        lx_mdelay(5);
    }
    return -1;
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

static uint8_t txq_gen2_num_tbs(struct iwl_tfh_tfd_gen2 *tfd)
{
    return (uint8_t)(tfd->num_tbs & 0x1fu);
}

static int txq_gen2_set_tb(struct iwl_tfh_tfd_gen2 *tfd, uint64_t addr, uint16_t len)
{
    unsigned idx = txq_gen2_num_tbs(tfd);
    struct iwl_tfh_tb *tb;

    if (idx >= IWL_TFH_NUM_TBS)
        return -1;
    tb = &tfd->tbs[idx];
    tb->addr = addr;
    tb->tb_len = len;
    tfd->num_tbs = (uint16_t)(idx + 1u);
    return 0;
}

static void txq_gen2_update_byte_tbl(void *bc_cpu, unsigned idx, uint16_t byte_cnt,
                                     uint8_t num_tbs)
{
    uint16_t *bc;
    unsigned filled;
    uint8_t num_fetch;
    uint16_t ent;

    if (!bc_cpu || idx >= IWL_MGMT_QUEUE_SIZE)
        return;
    bc = (uint16_t *)bc_cpu;
    filled = (unsigned)offsetof(struct iwl_tfh_tfd_gen2, tbs) +
             (unsigned)num_tbs * (unsigned)sizeof(struct iwl_tfh_tb);
    num_fetch = (uint8_t)((filled + 63u) / 64u - 1u);
    byte_cnt = (uint16_t)((byte_cnt + 3u) / 4u);
    if (byte_cnt > 0xfffu)
        byte_cnt = 0xfffu;
    ent = (uint16_t)(byte_cnt | ((uint16_t)num_fetch << 12));
    bc[idx] = iwl_cpu_to_le16(ent);
    if (idx + TFD_QUEUE_SIZE_MAX < TFD_QUEUE_BC_SIZE)
        bc[idx + TFD_QUEUE_SIZE_MAX] = iwl_cpu_to_le16(ent);
}

static unsigned scd_dma_round(unsigned bytes)
{
    unsigned align = IWL_SCD_DMA_ALIGN;

    return (bytes + align - 1u) & ~(align - 1u);
}

static int txq_ring_alloc_dma(void **tfd_cpu, uint64_t *tfd_dma,
                              void **first_tb_cpu, uint64_t *first_tb_dma,
                              void **body_cpu, uint64_t *body_dma,
                              void **bc_cpu, uint64_t *bc_dma,
                              uint64_t invalid_dma, uint16_t invalid_size)
{
    unsigned tfd_bytes = scd_dma_round(IWL_MGMT_QUEUE_SIZE * IWL_TFH_TFD_SIZE);
    unsigned first_tb_bytes =
        scd_dma_round(IWL_MGMT_QUEUE_SIZE * IWL_FIRST_TB_SIZE_ALIGN);
    unsigned body_bytes = scd_dma_round(IWL_MGMT_QUEUE_SIZE * IWL_MGMT_TX_SLOT_SIZE);
    unsigned bc_bytes = scd_dma_round(IWL_SCD_BC_TBL_BYTES);
    unsigned i;

    if (*tfd_cpu && *first_tb_cpu && *body_cpu && *bc_cpu)
        return 0;
    *tfd_cpu = lx_dma_alloc_coherent(0, tfd_bytes, tfd_dma, GFP_KERNEL);
    *first_tb_cpu =
        lx_dma_alloc_coherent(0, first_tb_bytes, first_tb_dma, GFP_KERNEL);
    *body_cpu = lx_dma_alloc_coherent(0, body_bytes, body_dma, GFP_KERNEL);
    *bc_cpu = lx_dma_alloc_coherent(0, bc_bytes, bc_dma, GFP_KERNEL);
    if (!*tfd_cpu || !*first_tb_cpu || !*body_cpu || !*bc_cpu)
        return -1;
    memset(*tfd_cpu, 0, tfd_bytes);
    memset(*first_tb_cpu, 0, first_tb_bytes);
    memset(*body_cpu, 0, body_bytes);
    memset(*bc_cpu, 0, bc_bytes);
    for (i = 0; i < IWL_MGMT_QUEUE_SIZE; i++) {
        struct iwl_tfh_tfd_gen2 *tfd =
            (struct iwl_tfh_tfd_gen2 *)((uint8_t *)*tfd_cpu +
                                        (size_t)i * IWL_TFH_TFD_SIZE);

        iwl_txq_set_tfd_invalid_gen2(tfd, invalid_dma, invalid_size);
    }
    return 0;
}

static int mgmt_txq_alloc_dma(struct iwl_ax211_priv *iwl)
{
    if (iwl->mgmt_tfd_cpu && iwl->mgmt_first_tb_cpu && iwl->mgmt_body_cpu &&
        iwl->mgmt_bc_cpu && iwl->invalid_tx_cmd_cpu)
        return 0;
    if (!iwl->invalid_tx_cmd_cpu) {
        iwl->invalid_tx_cmd_size = (uint16_t)sizeof(struct iwl_cmd_header_wide);
        iwl->invalid_tx_cmd_cpu = lx_dma_alloc_coherent(
            0, iwl->invalid_tx_cmd_size, &iwl->invalid_tx_cmd_dma, GFP_KERNEL);
        if (!iwl->invalid_tx_cmd_cpu)
            return -1;
        iwl_invalid_tx_cmd_init((struct iwl_cmd_header_wide *)iwl->invalid_tx_cmd_cpu);
    }
    return txq_ring_alloc_dma(&iwl->mgmt_tfd_cpu, &iwl->mgmt_tfd_dma,
                              &iwl->mgmt_first_tb_cpu, &iwl->mgmt_first_tb_dma,
                              &iwl->mgmt_body_cpu, &iwl->mgmt_body_dma,
                              &iwl->mgmt_bc_cpu, &iwl->mgmt_bc_dma,
                              iwl->invalid_tx_cmd_dma, iwl->invalid_tx_cmd_size);
}

static int data_txq_alloc_dma(struct iwl_ax211_priv *iwl)
{
    if (!iwl->invalid_tx_cmd_cpu) {
        iwl->invalid_tx_cmd_size = (uint16_t)sizeof(struct iwl_cmd_header_wide);
        iwl->invalid_tx_cmd_cpu = lx_dma_alloc_coherent(
            0, iwl->invalid_tx_cmd_size, &iwl->invalid_tx_cmd_dma, GFP_KERNEL);
        if (!iwl->invalid_tx_cmd_cpu)
            return -1;
        iwl_invalid_tx_cmd_init((struct iwl_cmd_header_wide *)iwl->invalid_tx_cmd_cpu);
    }
    return txq_ring_alloc_dma(&iwl->data_tfd_cpu, &iwl->data_tfd_dma,
                              &iwl->data_first_tb_cpu, &iwl->data_first_tb_dma,
                              &iwl->data_body_cpu, &iwl->data_body_dma,
                              &iwl->data_bc_cpu, &iwl->data_bc_dma,
                              iwl->invalid_tx_cmd_dma, iwl->invalid_tx_cmd_size);
}

int iwl_trans_txq_alloc_mgmt(struct iwl_ax211_priv *iwl, uint8_t sta_id)
{
    struct iwl_tx_queue_cfg_rsp rsp;
    uint16_t qid;
    uint16_t wr;
    static int logged;
    int scd_ver;
    int ret;

    if (!iwl || !iwl->alive)
        return -1;
    if (iwl->mgmt_txq_ready)
        return (int)iwl->mgmt_txq_id;
    if (mgmt_txq_alloc_dma(iwl) != 0)
        return -1;

    scd_ver = iwl_fw_cmd_ver(iwl, DATA_PATH_GROUP, SCD_QUEUE_CONFIG_CMD);
    if (scd_ver == 3) {
        struct iwl_scd_queue_cfg_cmd scd;

        memset(&scd, 0, sizeof(scd));
        scd.operation = iwl_cpu_to_le32(IWL_SCD_QUEUE_ADD);
        scd.u.add.sta_mask = iwl_cpu_to_le32(1u << sta_id);
        scd.u.add.tid = IWL_MGMT_TID;
        scd.u.add.flags = 0;
        scd.u.add.cb_size =
            iwl_cpu_to_le32(tfd_queue_cb_size(IWL_MGMT_QUEUE_SIZE));
        scd.u.add.bc_dram_addr = iwl->mgmt_bc_dma;
        scd.u.add.tfdq_dram_addr = iwl->mgmt_tfd_dma;
        lx_printk("iwl_trans: SCD_QUEUE_CONFIG grp=%u id=0x%02x ver=%u len=%zu "
                  "tfd=0x%llx bc=0x%llx cb_size=%u n=%u sta=%u tid=%u\n",
                  (unsigned)DATA_PATH_GROUP, (unsigned)SCD_QUEUE_CONFIG_CMD,
                  scd_ver, sizeof(scd),
                  (unsigned long long)scd.u.add.tfdq_dram_addr,
                  (unsigned long long)scd.u.add.bc_dram_addr,
                  (unsigned)scd.u.add.cb_size, (unsigned)IWL_MGMT_QUEUE_SIZE,
                  (unsigned)sta_id, (unsigned)scd.u.add.tid);
        ret = iwl_trans_send_cmd_wait(iwl, DATA_PATH_GROUP, SCD_QUEUE_CONFIG_CMD,
                                      &scd, (uint16_t)sizeof(scd),
                                      IWL_MVM_HCMD_TIMEOUT_MS);
    } else if (scd_ver == 0) {
        struct iwl_tx_queue_cfg_cmd cfg;

        memset(&cfg, 0, sizeof(cfg));
        cfg.sta_id = sta_id;
        cfg.tid = IWL_MGMT_TID;
        cfg.flags = iwl_cpu_to_le16(TX_QUEUE_CFG_ENABLE_QUEUE);
        cfg.cb_size = iwl_cpu_to_le32(tfd_queue_cb_size(IWL_MGMT_QUEUE_SIZE));
        cfg.byte_cnt_addr = iwl->mgmt_bc_dma;
        cfg.tfdq_addr = iwl->mgmt_tfd_dma;
        lx_printk("iwl_trans: SCD_QUEUE_CFG grp=%u id=0x%02x ver=0 len=%zu "
                  "tfd=0x%llx bc=0x%llx cb_size=%u n=%u sta=%u\n",
                  (unsigned)LEGACY_GROUP, (unsigned)SCD_QUEUE_CFG,
                  sizeof(cfg), (unsigned long long)cfg.tfdq_addr,
                  (unsigned long long)cfg.byte_cnt_addr, (unsigned)cfg.cb_size,
                  (unsigned)IWL_MGMT_QUEUE_SIZE, (unsigned)sta_id);
        ret = iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, SCD_QUEUE_CFG, &cfg,
                                      (uint16_t)sizeof(cfg),
                                      IWL_MVM_HCMD_TIMEOUT_MS);
    } else {
        lx_printk("iwl_trans: SCD queue alloc ver=%d no soportada\n", scd_ver);
        return -1;
    }
    if (ret != 0) {
        lx_printk("iwl_trans: SCD_QUEUE_CFG mgmt falló\n");
        return -1;
    }
    if (iwl->cmd_resp_len < (uint16_t)sizeof(rsp)) {
        lx_printk("iwl_trans: SCD_QUEUE_CFG resp corta (%u B)\n",
                  (unsigned)iwl->cmd_resp_len);
        return -1;
    }
    memcpy(&rsp, iwl->cmd_resp, sizeof(rsp));
    qid = (uint16_t)(rsp.queue_number & 0x7fffu);
    wr = (uint16_t)(rsp.write_pointer & (IWL_MGMT_QUEUE_SIZE - 1u));
    if (!qid) {
        lx_printk("iwl_trans: SCD_QUEUE_CFG qid=0 (HCMD reservada)\n");
        return -1;
    }
    iwl->mgmt_txq_id = qid;
    iwl->mgmt_txq_write = wr;
    /* Cola recién creada: el consumidor arranca donde el productor, o el primer
     * envío parecería que la deja llena. */
    iwl->mgmt_txq_read = wr;
    iwl->mgmt_txq_ready = 1;
    if (!logged) {
        lx_printk("iwl_trans: TXQ mgmt qid=%u tid=%u slots=%u wr=%u "
                  "tx_api=%s\n",
                  (unsigned)qid, (unsigned)IWL_MGMT_TID,
                  (unsigned)IWL_MGMT_QUEUE_SIZE, (unsigned)wr,
                  iwl_mvm_has_new_tx_api(iwl) ? "tvqm" : "legacy");
        logged = 1;
    }
    iwl_trans_txq_drain_mgmt(iwl);
    return (int)qid;
}

int iwl_trans_txq_alloc_data(struct iwl_ax211_priv *iwl, uint8_t sta_id, uint8_t tid)
{
    struct iwl_tx_queue_cfg_rsp rsp;
    uint16_t qid;
    uint16_t wr;
    static int logged;
    int scd_ver;
    int ret;

    if (!iwl || !iwl->alive)
        return -1;
    if (iwl->data_txq_ready)
        return (int)iwl->data_txq_id;
    if (data_txq_alloc_dma(iwl) != 0)
        return -1;

    scd_ver = iwl_fw_cmd_ver(iwl, DATA_PATH_GROUP, SCD_QUEUE_CONFIG_CMD);
    if (scd_ver == 3) {
        struct iwl_scd_queue_cfg_cmd scd;

        memset(&scd, 0, sizeof(scd));
        scd.operation = iwl_cpu_to_le32(IWL_SCD_QUEUE_ADD);
        scd.u.add.sta_mask = iwl_cpu_to_le32(1u << sta_id);
        scd.u.add.tid = tid;
        scd.u.add.flags = 0;
        scd.u.add.cb_size =
            iwl_cpu_to_le32(tfd_queue_cb_size(IWL_MGMT_QUEUE_SIZE));
        scd.u.add.bc_dram_addr = iwl->data_bc_dma;
        scd.u.add.tfdq_dram_addr = iwl->data_tfd_dma;
        lx_printk("iwl_trans: SCD_QUEUE_CONFIG data grp=%u id=0x%02x ver=%u "
                  "sta=%u tid=%u\n",
                  (unsigned)DATA_PATH_GROUP, (unsigned)SCD_QUEUE_CONFIG_CMD,
                  scd_ver, (unsigned)sta_id, (unsigned)tid);
        ret = iwl_trans_send_cmd_wait(iwl, DATA_PATH_GROUP, SCD_QUEUE_CONFIG_CMD,
                                      &scd, (uint16_t)sizeof(scd),
                                      IWL_MVM_HCMD_TIMEOUT_MS);
    } else if (scd_ver == 0) {
        struct iwl_tx_queue_cfg_cmd cfg;

        memset(&cfg, 0, sizeof(cfg));
        cfg.sta_id = sta_id;
        cfg.tid = tid;
        cfg.flags = iwl_cpu_to_le16(TX_QUEUE_CFG_ENABLE_QUEUE);
        cfg.cb_size = iwl_cpu_to_le32(tfd_queue_cb_size(IWL_MGMT_QUEUE_SIZE));
        cfg.byte_cnt_addr = iwl->data_bc_dma;
        cfg.tfdq_addr = iwl->data_tfd_dma;
        ret = iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, SCD_QUEUE_CFG, &cfg,
                                      (uint16_t)sizeof(cfg),
                                      IWL_MVM_HCMD_TIMEOUT_MS);
    } else {
        lx_printk("iwl_trans: SCD data queue alloc ver=%d no soportada\n", scd_ver);
        return -1;
    }
    if (ret != 0) {
        lx_printk("iwl_trans: SCD_QUEUE_CFG data falló\n");
        return -1;
    }
    if (iwl->cmd_resp_len < (uint16_t)sizeof(rsp)) {
        lx_printk("iwl_trans: SCD_QUEUE_CFG data resp corta (%u B)\n",
                  (unsigned)iwl->cmd_resp_len);
        return -1;
    }
    memcpy(&rsp, iwl->cmd_resp, sizeof(rsp));
    qid = (uint16_t)(rsp.queue_number & 0x7fffu);
    wr = (uint16_t)(rsp.write_pointer & (IWL_MGMT_QUEUE_SIZE - 1u));
    if (!qid) {
        lx_printk("iwl_trans: SCD_QUEUE_CFG data qid=0\n");
        return -1;
    }
    iwl->data_txq_id = qid;
    iwl->data_txq_write = wr;
    iwl->data_txq_read = wr;
    iwl->data_txq_ready = 1;
    if (!logged) {
        lx_printk("iwl_trans: TXQ data qid=%u tid=%u slots=%u wr=%u\n",
                  (unsigned)qid, (unsigned)tid, (unsigned)IWL_MGMT_QUEUE_SIZE,
                  (unsigned)wr);
        logged = 1;
    }
    iwl_trans_txq_drain_data(iwl);
    return (int)qid;
}

int iwl_trans_tx(struct iwl_ax211_priv *iwl, uint16_t txq_id,
                 const void *payload, uint16_t pay_len)
{
    unsigned idx;
    struct iwl_tfh_tfd_gen2 *tfd;
    uint8_t *body;
    uint8_t *first_tb;
    struct iwl_cmd_header *hdr;
    uint64_t body_dma;
    uint64_t first_tb_dma;
    void *tfd_cpu;
    void *body_cpu;
    void *first_tb_cpu;
    void *bc_cpu;
    uint16_t *writep;
    unsigned total;
    unsigned tb1_len;
    uint16_t frame_len;
    uint8_t num_tbs;
    uint32_t doorbell;
    unsigned poll;
    int is_data;

    if (!iwl || !iwl->alive)
        return -1;
    is_data = iwl->data_txq_ready && txq_id == iwl->data_txq_id;
    if (!is_data && (!iwl->mgmt_txq_ready || txq_id != iwl->mgmt_txq_id))
        return -1;
    if (!payload || pay_len == 0 ||
        pay_len + sizeof(struct iwl_cmd_header) > IWL_MGMT_TX_SLOT_SIZE)
        return -1;
    if (iwl->in_trans || iwl->cmd_needs_recover)
        return -1;
    writep = txq_write_ptr(iwl, txq_id);
    if (!writep || txq_space_id(iwl, txq_id) == 0) {
        /* Sin printk: este camino corre dentro de `net::poll` (timer) y
         * pintar en el GOP del ROG colgó el SSH y acabó en page fault. */
        iwl->tx_full_drop++;
        return -1;
    }

    if (is_data) {
        if (data_txq_alloc_dma(iwl) != 0)
            return -1;
        tfd_cpu = iwl->data_tfd_cpu;
        body_cpu = iwl->data_body_cpu;
        first_tb_cpu = iwl->data_first_tb_cpu;
        bc_cpu = iwl->data_bc_cpu;
        body_dma = iwl->data_body_dma;
        first_tb_dma = iwl->data_first_tb_dma;
    } else {
        if (mgmt_txq_alloc_dma(iwl) != 0)
            return -1;
        tfd_cpu = iwl->mgmt_tfd_cpu;
        body_cpu = iwl->mgmt_body_cpu;
        first_tb_cpu = iwl->mgmt_first_tb_cpu;
        bc_cpu = iwl->mgmt_bc_cpu;
        body_dma = iwl->mgmt_body_dma;
        first_tb_dma = iwl->mgmt_first_tb_dma;
    }

    idx = (unsigned)(*writep & (IWL_MGMT_QUEUE_SIZE - 1u));
    body = (uint8_t *)body_cpu + idx * IWL_MGMT_TX_SLOT_SIZE;
    first_tb = (uint8_t *)first_tb_cpu + idx * IWL_FIRST_TB_SIZE_ALIGN;
    tfd = (struct iwl_tfh_tfd_gen2 *)((uint8_t *)tfd_cpu +
                                      idx * IWL_TFH_TFD_SIZE);
    body_dma += (uint64_t)idx * IWL_MGMT_TX_SLOT_SIZE;
    first_tb_dma += (uint64_t)idx * IWL_FIRST_TB_SIZE_ALIGN;

    memset(tfd, 0, sizeof(*tfd));
    memset(body, 0, IWL_MGMT_TX_SLOT_SIZE);
    hdr = (struct iwl_cmd_header *)body;
    hdr->cmd = TX_CMD;
    hdr->group_id = LEGACY_GROUP;
    hdr->sequence = (uint16_t)(QUEUE_TO_SEQ(txq_id) | INDEX_TO_SEQ(idx));
    memcpy(body + sizeof(*hdr), payload, pay_len);
    total = (unsigned)sizeof(*hdr) + (unsigned)pay_len;
    memcpy(first_tb, body, IWL_FIRST_TB_SIZE);

    if (txq_gen2_set_tb(tfd, first_tb_dma, IWL_FIRST_TB_SIZE) != 0)
        return -1;
    tb1_len = total - IWL_FIRST_TB_SIZE;
    if (txq_gen2_set_tb(tfd, body_dma + IWL_FIRST_TB_SIZE, (uint16_t)tb1_len) != 0)
        return -1;

    if (iwl->gen3) {
        const struct iwl_tx_cmd_gen3 *tc =
            (const struct iwl_tx_cmd_gen3 *)(body + sizeof(*hdr));

        frame_len = (uint16_t)tc->len;
    } else {
        const struct iwl_tx_cmd_gen2 *tc =
            (const struct iwl_tx_cmd_gen2 *)(body + sizeof(*hdr));

        frame_len = (uint16_t)tc->len;
    }
    num_tbs = txq_gen2_num_tbs(tfd);
    txq_gen2_update_byte_tbl(bc_cpu, idx, frame_len, num_tbs);

    *writep = (uint16_t)((*writep + 1u) & (IWL_MGMT_QUEUE_SIZE - 1u));
    doorbell = tx_doorbell(iwl, txq_id, *writep);
    iwl_write32(iwl, HBUS_TARG_WRPTR, doorbell);
    for (poll = 0; poll < 16; poll++)
        drain_rx_gen2(iwl);
    poll_hcmd_first_tb(iwl);
    return 0;
}

static int send_hcmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                     const void *payload, uint16_t pay_len, int async)
{
    uint16_t slot;
    uint8_t *buf;
    struct iwl_tfh_tfd_long *tfd;
    uint16_t total;
    uint16_t seq;
    unsigned poll;

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
    memset(buf, 0, IWL_CMD_SLOT_SIZE);
    if (iwl->family == IWL_DEVICE_FAMILY_8000) {
        struct iwl_tfd *tfd8000 =
            (struct iwl_tfd *)((uint8_t *)iwl->mtr_cpu +
                               (size_t)(slot % IWL_8000_TFD_RING_N) * IWL_GEN1_TFD_SIZE);
        memset(tfd8000, 0, sizeof(*tfd8000));
    } else {
        tfd = (struct iwl_tfh_tfd_long *)((uint8_t *)iwl->mtr_cpu +
                                          (size_t)slot * IWL_TFH_TFD_SIZE);
        memset(tfd, 0, sizeof(*tfd));
    }

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
        /* Linux pcie/tx-gen2.c: version = iwl_cmd_version(cmd->id) (bits 16:23
         * del WIDE_ID), no el cmd_ver del TLV CMD_VERSIONS. */
        whdr->version = 0;
        if (pay_len)
            memcpy(buf + sizeof(*whdr), payload, pay_len);
        if (group == DATA_PATH_GROUP && id == SCD_QUEUE_CONFIG_CMD) {
            static int scd_ver_logged;

            if (!scd_ver_logged) {
                lx_printk("iwl_trans: SCD wide ver_hdr=0 ver_tlv=%u\n",
                          (unsigned)iwl_fw_cmd_ver(iwl, group, id));
                scd_ver_logged = 1;
            }
        } else if (id == TX_ANT_CONFIGURATION_CMD) {
            lx_printk("iwl_trans: TX_ANT wide ver_hdr=0 len=%u (payload %u B)\n",
                      (unsigned)total, (unsigned)pay_len);
        }
        }
    }

    if (iwl->family == IWL_DEVICE_FAMILY_8000) {
        struct iwl_tfd *tfd8000 =
            (struct iwl_tfd *)((uint8_t *)iwl->mtr_cpu +
                               (size_t)(slot % IWL_8000_TFD_RING_N) * IWL_GEN1_TFD_SIZE);
        uint16_t tb0_size = total <= IWL_FIRST_TB_SIZE ? total : IWL_FIRST_TB_SIZE;
        uint8_t *first_tb =
            (uint8_t *)iwl->hcmd_first_tb_cpu + (size_t)slot * IWL_FIRST_TB_SIZE_ALIGN;
        uint64_t first_tb_dma =
            iwl->hcmd_first_tb_dma + (uint64_t)slot * IWL_FIRST_TB_SIZE_ALIGN;

        memcpy(first_tb, buf, tb0_size);
        if (gen1_tfd_set_tb(tfd8000, 0, first_tb_dma, tb0_size) != 0) {
            iwl->in_trans = 0;
            return -1;
        }
        if (total > tb0_size &&
            gen1_tfd_set_tb(tfd8000, 1,
                            iwl->mcr_dma + (uint64_t)slot * IWL_CMD_SLOT_SIZE + tb0_size,
                            (uint16_t)(total - tb0_size)) != 0) {
            iwl->in_trans = 0;
            return -1;
        }
    } else if (iwl->hcmd_first_tb_cpu) {
        uint16_t tb0_size = total <= IWL_FIRST_TB_SIZE ? total : IWL_FIRST_TB_SIZE;
        uint8_t *first_tb =
            (uint8_t *)iwl->hcmd_first_tb_cpu + (size_t)slot * IWL_FIRST_TB_SIZE_ALIGN;
        uint64_t first_tb_dma =
            iwl->hcmd_first_tb_dma + (uint64_t)slot * IWL_FIRST_TB_SIZE_ALIGN;

        tfd->num_tbs = 0;
        memcpy(first_tb, buf, tb0_size);
        if (txq_gen2_set_tb((struct iwl_tfh_tfd_gen2 *)tfd, first_tb_dma,
                            tb0_size) != 0) {
            iwl->in_trans = 0;
            return -1;
        }
        if (total > tb0_size &&
            txq_gen2_set_tb((struct iwl_tfh_tfd_gen2 *)tfd,
                            iwl->mcr_dma + (uint64_t)slot * IWL_CMD_SLOT_SIZE +
                                tb0_size,
                            (uint16_t)(total - tb0_size)) != 0) {
            iwl->in_trans = 0;
            return -1;
        }
    } else {
        tfd->num_tbs = 1;
        tfd->tbs[0].tb_len = total;
        tfd->tbs[0].addr = iwl->mcr_dma + (uint64_t)slot * IWL_CMD_SLOT_SIZE;
    }
    iwl->cmd_write = (uint16_t)((iwl->cmd_write + 1u) & (IWL_CMD_QUEUE_SIZE - 1u));
    {
        uint32_t doorbell = tx_doorbell(iwl, iwl->cmd_qid, iwl->cmd_write);
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
    for (poll = 0; poll < 8; poll++)
        drain_rx(iwl);
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
        poll_hcmd_first_tb(iwl);
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
            drain_rx(iwl);
        lx_printk("iwl_trans: timeout cmd grp=%u id=0x%02x seq=0x%04x slot=%u; "
                  "MVM parado\n",
                  (unsigned)log_grp, (unsigned)id,
                  (unsigned)iwl->cmd_pending_seq, slot);
    }
    return -1;
}
