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

static void iwl_write_prph_no_grab(struct iwl_ax211_priv *iwl, uint32_t addr, uint32_t val)
{
    iwl_write32(iwl, HBUS_TARG_PRPH_WADDR, (addr & 0x000fffffu) | (3u << 24));
    iwl_write32(iwl, HBUS_TARG_PRPH_WDATA, val);
}

static uint32_t iwl_read_prph_no_grab(struct iwl_ax211_priv *iwl, uint32_t addr)
{
    iwl_write32(iwl, HBUS_TARG_PRPH_RADDR, (addr & 0x000fffffu) | (3u << 24));
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

    /* Linux `iwl_clear_persistence_bit`: PRPH sin grab. El MAC aún no tiene
     * reloj (INIT_DONE va en APM, después); grab_nic_access espera
     * MAC_CLOCK_READY y abortaba todo el start (GP=0x08040008). */
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
    iwl->mcr_cpu = lx_dma_alloc_coherent(0, IWL_CMD_QUEUE_SIZE * 256, &iwl->mcr_dma, GFP_KERNEL);
    if (!iwl->rx_bd_cpu || !iwl->used_bd_cpu || !iwl->rb_stts ||
        !iwl->rx_page_cpu || !iwl->mtr_cpu || !iwl->mcr_cpu)
        return -1;
    memset(iwl->rx_bd_cpu, 0, IWL_GEN2_RX_N * 8);
    memset(iwl->used_bd_cpu, 0, used_sz);
    memset((void *)iwl->rb_stts, 0, 16);
    memset(iwl->mtr_cpu, 0, IWL_CMD_QUEUE_SIZE * IWL_TFH_TFD_SIZE);
    memset(iwl->mcr_cpu, 0, IWL_CMD_QUEUE_SIZE * 256);
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

static void parse_scan_notify(struct iwl_ax211_priv *iwl, const uint8_t *data, int len)
{
    /* Notificación UMAC scan: SSID ASCII tras cabecera mínima */
    if (len < 40)
        return;
    for (int off = 20; off + 4 < len && off < 200; off++) {
        if (data[off] == 0 || data[off] > 32)
            continue;
        int ssid_len = (int)data[off];
        if (ssid_len <= 0 || ssid_len > IWL_AX211_SSID_MAX || off + 1 + ssid_len >= len)
            continue;
        struct iwl_ax211_bss bss;
        memset(&bss, 0, sizeof(bss));
        memcpy(bss.ssid, &data[off + 1], (size_t)ssid_len);
        bss.rssi = -60;
        bss.channel = 6;
        bss.open = 1;
        iwl_ax211_add_bss(&bss);
        return;
    }
}

static void handle_gen2_rx(struct iwl_ax211_priv *iwl, const uint8_t *buf)
{
    uint32_t len_n_flags = (uint32_t)buf[0] | ((uint32_t)buf[1] << 8) |
                           ((uint32_t)buf[2] << 16) | ((uint32_t)buf[3] << 24);
    uint16_t len = (uint16_t)(len_n_flags & 0x3fff);
    uint8_t cmd = buf[4];
    uint8_t group = buf[5];
    const uint8_t *data = buf + 8;
    int pay = (int)len - 4;
    if (pay < 0)
        pay = 0;
    if (group == 0 && cmd == UCODE_ALIVE_NTFY) {
        iwl->alive = 1;
        lx_iwlwifi_set_alive(1);
        lx_printk("iwl_ax211: firmware ALIVE (UCODE_ALIVE_NTFY)\n");
        return;
    }
    if (group == SCAN_GROUP)
        parse_scan_notify(iwl, data, pay);
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
                                   (size_t)idx * IWL_GEN2_RX_SZ);
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
                               (size_t)idx * IWL_GEN2_RX_SZ);
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
    ctxt->version = 0;
    ctxt->size = (uint16_t)(sizeof(*ctxt) / 4);
    ctxt->cr_idx_arr_size = 1;
    ctxt->tr_idx_arr_size = 1;
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
    iwl_write32(iwl, RFH_Q0_FRBDCB_WIDX_TRG, (uint32_t)(iwl->rx_write & ~7u));
    iwl_write64(iwl, CSR_CTXT_INFO_ADDR, iwl->ctxt_dma);
    iwl_write64(iwl, CSR_IML_DATA_ADDR, iml_dma);
    iwl_write32(iwl, CSR_IML_SIZE_ADDR, (uint32_t)iwl->iml_len);
    iwl_write32(iwl, CSR_CTXT_INFO_BOOT_CTRL, CSR_AUTO_FUNC_BOOT_ENA);
    gp = iwl_read32(iwl, CSR_GP_CNTRL);
    iwl_write32(iwl, CSR_GP_CNTRL,
                gp | CSR_GP_CNTRL_REG_FLAG_INIT_DONE |
                    CSR_GP_CNTRL_REG_FLAG_MAC_ACCESS_REQ | CSR_AUTO_FUNC_INIT);
    lx_printk("iwlwifi: context-info gen3 BA=0x%llx iml=%lu\n",
              (unsigned long long)iwl->ctxt_dma, iwl->iml_len);

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

int iwl_trans_send_cmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                       const void *payload, uint16_t pay_len)
{
    uint16_t slot;
    uint8_t *buf;
    struct iwl_cmd_header *hdr;
    struct iwl_tfh_tfd_long *tfd;

    if (!iwl->alive || !iwl->mtr_cpu || !iwl->mcr_cpu)
        return -1;
    if (pay_len + sizeof(struct iwl_cmd_header) > 240)
        return -1;

    slot = iwl->cmd_write % IWL_CMD_QUEUE_SIZE;
    buf = (uint8_t *)iwl->mcr_cpu + slot * 256;
    hdr = (struct iwl_cmd_header *)buf;
    tfd = (struct iwl_tfh_tfd_long *)((uint8_t *)iwl->mtr_cpu + (size_t)slot * IWL_TFH_TFD_SIZE);
    memset(buf, 0, 256);
    memset(tfd, 0, sizeof(*tfd));
    hdr->cmd = id;
    hdr->group_id = group;
    hdr->sequence = iwl->cmd_seq++;
    hdr->length = (uint8_t)pay_len;
    if (pay_len)
        memcpy(buf + sizeof(*hdr), payload, pay_len);
    tfd->num_tbs = 1;
    tfd->tbs[0].tb_len = (uint16_t)(sizeof(*hdr) + pay_len);
    tfd->tbs[0].addr = iwl->mcr_dma + (uint64_t)slot * 256;
    iwl->cmd_write = (uint16_t)((iwl->cmd_write + 1) % IWL_CMD_QUEUE_SIZE);
    iwl_write32(iwl, HBUS_TARG_WRPTR,
                ((uint32_t)iwl->cmd_write & 0xffu) | ((uint32_t)iwl->cmd_qid << 8));
    drain_rx_gen2(iwl);
    return 0;
}
