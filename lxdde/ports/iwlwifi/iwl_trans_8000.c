/* Transporte familia 8000 (8265): carga FH; ICT tras ALIVE RT (Linux fw_alive). */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);

static uint32_t le32_u(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) |
           ((uint32_t)p[3] << 24);
}

static int rt_dest(const struct iwl_fw_rt_section *sec, uint32_t *dest,
                   const uint8_t **payload, uint32_t *plen)
{
    if (!sec || !sec->data || sec->len < 4)
        return -1;
    *dest = le32_u(sec->data);
    if (*dest == IWL_CPU1_CPU2_SEPARATOR || *dest == IWL_PAGING_SEPARATOR)
        return 1;
    if (sec->len <= 4)
        return -1;
    *payload = sec->data + 4;
    *plen = sec->len - 4;
    return 0;
}

int iwl_8000_plan_load_ex(const struct iwl_fw_rt_section *secs, int n_secs,
                          uint32_t chunk_sz, struct iwl_8000_load_plan *plan)
{
    int first = 0;
    int cpu;

    if (!secs || !plan || !chunk_sz || n_secs <= 0)
        return -1;
    memset(plan, 0, sizeof(*plan));
    plan->cpu1_final = 0xFFFFu;
    plan->cpu2_final = 0xFFFFFFFFu;
    plan->uses_context_info = 0;

    for (cpu = 1; cpu <= 2; cpu++) {
        int shift = cpu == 1 ? 0 : 16;
        uint32_t sec_num = 1;
        uint32_t acc = 0;
        int last = first;
        int i;

        if (cpu == 2)
            first++;
        if (first < 0 || first > n_secs)
            return -1;

        for (i = first; i < n_secs; i++) {
            uint32_t dest = 0;
            const uint8_t *payload = 0;
            uint32_t plen = 0;
            uint32_t off;
            int kind;

            last = i;
            kind = rt_dest(&secs[i], &dest, &payload, &plen);
            if (kind != 0)
                break;
            for (off = 0; off < plen; ) {
                uint32_t n = plen - off;

                if (n > chunk_sz)
                    n = chunk_sz;
                if (plan->n_chunks >= IWL_8000_PLAN_MAX)
                    return -1;
                plan->chunks[plan->n_chunks].dst = dest + off;
                plan->chunks[plan->n_chunks].len = n;
                plan->chunks[plan->n_chunks].off = off;
                plan->chunks[plan->n_chunks].cpu = (uint8_t)cpu;
                plan->chunks[plan->n_chunks].sec = (uint8_t)i;
                plan->n_chunks++;
                off += n;
            }
            acc |= (sec_num << shift);
            if (plan->n_status >= IWL_FW_RT_MAX)
                return -1;
            plan->status[plan->n_status].cpu = (uint8_t)cpu;
            plan->status[plan->n_status].sec = (uint8_t)i;
            plan->status[plan->n_status].load_status = acc;
            plan->n_status++;
            if (cpu == 1)
                plan->cpu1_secs++;
            else
                plan->cpu2_secs++;
            sec_num = (sec_num << 1) | 1u;
        }
        first = last;
    }
    if (!plan->cpu1_secs || !plan->cpu2_secs || !plan->n_chunks)
        return -1;
    return 0;
}

int iwl_8000_plan_load_secs(const struct iwl_fw_rt_section *secs, int n_secs,
                            struct iwl_8000_load_plan *plan)
{
    return iwl_8000_plan_load_ex(secs, n_secs, FH_MEM_TB_MAX_LENGTH, plan);
}

int iwl_8000_plan_load(const struct iwl_fw_image *fw, struct iwl_8000_load_plan *plan)
{
    if (!fw)
        return -1;
    return iwl_8000_plan_load_secs(fw->rt, fw->rt_n, plan);
}

int iwl_8000_plan_load_init(const struct iwl_fw_image *fw, struct iwl_8000_load_plan *plan)
{
    if (!fw)
        return -1;
    return iwl_8000_plan_load_secs(fw->sec_init, fw->sec_init_n, plan);
}

uint32_t iwl_8000_nic_config_value(uint32_t hw_rev, uint32_t phy_sku)
{
    uint8_t radio_type =
        (uint8_t)((phy_sku & FW_PHY_CFG_RADIO_TYPE) >> FW_PHY_CFG_RADIO_TYPE_POS);
    uint8_t radio_step =
        (uint8_t)((phy_sku & FW_PHY_CFG_RADIO_STEP) >> FW_PHY_CFG_RADIO_STEP_POS);
    uint8_t radio_dash =
        (uint8_t)((phy_sku & FW_PHY_CFG_RADIO_DASH) >> FW_PHY_CFG_RADIO_DASH_POS);
    uint32_t reg_val = CSR_HW_REV_STEP_DASH(hw_rev);

    reg_val |= (uint32_t)radio_type << CSR_HW_IF_CONFIG_REG_POS_PHY_TYPE;
    reg_val |= (uint32_t)radio_step << CSR_HW_IF_CONFIG_REG_POS_PHY_STEP;
    reg_val |= (uint32_t)radio_dash << CSR_HW_IF_CONFIG_REG_POS_PHY_DASH;
    return reg_val;
}

void iwl_8000_fh_program(uint32_t dst, uint64_t dma, uint32_t byte_cnt,
                         struct iwl_8000_fh_prog *out)
{
    uint32_t hi;

    if (!out)
        return;
    hi = (uint32_t)((dma >> 32) & 0xfu);
    out->tcsr_cfg_reg = FH_TCSR_CHNL_TX_CONFIG_REG(FH_SRVC_CHNL);
    out->tcsr_pause = FH_TCSR_TX_CONFIG_REG_VAL_DMA_CHNL_PAUSE;
    out->sram_reg = FH_SRVC_CHNL_SRAM_ADDR_REG(FH_SRVC_CHNL);
    out->sram_val = dst;
    out->tfdib0_reg = FH_TFDIB_CTRL0_REG(FH_SRVC_CHNL);
    out->tfdib0_val = (uint32_t)(dma & 0xffffffffu);
    out->tfdib1_reg = FH_TFDIB_CTRL1_REG(FH_SRVC_CHNL);
    out->tfdib1_val = (hi << FH_MEM_TFDIB_REG1_ADDR_BITSHIFT) | byte_cnt;
    out->buf_sts_reg = FH_TCSR_CHNL_TX_BUF_STS_REG(FH_SRVC_CHNL);
    out->buf_sts_val = (1u << FH_TCSR_CHNL_TX_BUF_STS_REG_POS_TB_NUM) |
                       (1u << FH_TCSR_CHNL_TX_BUF_STS_REG_POS_TB_IDX) |
                       FH_TCSR_CHNL_TX_BUF_STS_REG_VAL_TFDB_VALID;
    out->tcsr_run = FH_TCSR_TX_CONFIG_REG_VAL_DMA_CHNL_ENABLE |
                    FH_TCSR_TX_CONFIG_REG_VAL_DMA_CREDIT_DISABLE |
                    FH_TCSR_TX_CONFIG_REG_VAL_CIRQ_HOST_ENDTFD;
}

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

static uint32_t iwl_prph_msk(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return IWL_PRPH_MSK_GEN2;
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
    if (iwl_poll_bit(iwl, CSR_GP_CNTRL, mask, poll, 15) != 0)
        return -1;
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

static void iwl_write_direct32(struct iwl_ax211_priv *iwl, uint32_t off, uint32_t val)
{
    if (iwl_grab_nic_access(iwl) != 0)
        return;
    iwl_write32(iwl, off, val);
    iwl_release_nic_access(iwl);
}

static uint32_t iwl_read_direct32(struct iwl_ax211_priv *iwl, uint32_t off)
{
    uint32_t v;

    if (iwl_grab_nic_access(iwl) != 0)
        return 0xffffffffu;
    v = iwl_read32(iwl, off);
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
                     CSR_HW_IF_CONFIG_REG_BIT_NIC_READY, 200) != 0)
        return -1;
    return 0;
}

static int iwl_finish_nic_init(struct iwl_ax211_priv *iwl)
{
    iwl_set_bit(iwl, CSR_GP_CNTRL, CSR_GP_CNTRL_REG_FLAG_INIT_DONE);
    return iwl_poll_bit(iwl, CSR_GP_CNTRL, CSR_GP_CNTRL_REG_FLAG_MAC_CLOCK_READY,
                        CSR_GP_CNTRL_REG_FLAG_MAC_CLOCK_READY, 25000);
}

static void iwl_8000_nic_config(struct iwl_ax211_priv *iwl)
{
    uint32_t mask = CSR_HW_IF_CONFIG_REG_MSK_MAC_STEP_DASH |
                    CSR_HW_IF_CONFIG_REG_MSK_PHY_TYPE |
                    CSR_HW_IF_CONFIG_REG_MSK_PHY_STEP |
                    CSR_HW_IF_CONFIG_REG_MSK_PHY_DASH |
                    CSR_HW_IF_CONFIG_REG_BIT_RADIO_SI |
                    CSR_HW_IF_CONFIG_REG_BIT_MAC_SI |
                    CSR_HW_IF_CONFIG_REG_D3_DEBUG;
    uint32_t reg_val = iwl_8000_nic_config_value(iwl->hw_rev, iwl->phy_sku);
    uint32_t old = iwl_read32(iwl, CSR_HW_IF_CONFIG_REG);

    /* Familia 8000: sin CSR_HW_IF_CONFIG_REG_BIT_RADIO_SI (mvm/ops.c). */
    iwl_write32(iwl, CSR_HW_IF_CONFIG_REG, (old & ~mask) | (reg_val & mask));
}

static int iwl_8000_apm_init(struct iwl_ax211_priv *iwl)
{
    /* 8000: apmg_not_supported; mismos chicken bits que Linux >= 8000. */
    iwl_set_bit(iwl, CSR_GIO_CHICKEN_BITS, CSR_GIO_CHICKEN_BITS_REG_BIT_L1A_NO_L0S_RX);
    iwl_set_bit(iwl, CSR_DBG_HPET_MEM_REG, CSR_DBG_HPET_MEM_REG_VAL);
    iwl_set_bit(iwl, CSR_HW_IF_CONFIG_REG, CSR_HW_IF_CONFIG_REG_BIT_HAP_WAKE_L1A);
    iwl_set_bit(iwl, CSR_GIO_REG, CSR_GIO_REG_VAL_L0S_DISABLED);
    if (iwl_finish_nic_init(iwl) != 0)
        return -1;
    iwl_8000_nic_config(iwl);
    /* cfg/8000.c shadow_reg_enable=true → trans.c:568-571. */
    iwl_write32(iwl, CSR_MAC_SHADOW_REG_CTRL, 0x800FFFFFu);
    return 0;
}

static void iwl_sw_reset(struct iwl_ax211_priv *iwl)
{
    iwl_set_bit(iwl, CSR_RESET, CSR_RESET_REG_FLAG_SW_RESET);
    lx_udelay(5000);
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

static int iwl_8000_alloc_rx(struct iwl_ax211_priv *iwl)
{
    unsigned i;
    uint32_t *bd;

    if (iwl->rx_bd_cpu && iwl->rb_stts && iwl->rx_page_cpu)
        goto reiniciar;
    iwl->rx_bd_cpu = lx_dma_alloc_coherent(0, IWL_8000_RX_N * 4u, &iwl->rx_bd_dma,
                                           GFP_KERNEL);
    iwl->rb_stts = (volatile uint16_t *)lx_dma_alloc_coherent(0, 16, &iwl->rb_stts_dma,
                                                              GFP_KERNEL);
    iwl->rx_page_cpu = lx_dma_alloc_coherent(0, IWL_8000_RX_N * IWL_GEN2_RX_SZ,
                                             &iwl->rx_page_dma, GFP_KERNEL);
    if (!iwl->rx_bd_cpu || !iwl->rb_stts || !iwl->rx_page_cpu)
        return -1;
reiniciar:
    memset(iwl->rx_bd_cpu, 0, IWL_8000_RX_N * 4u);
    memset((void *)iwl->rb_stts, 0, 16);
    bd = (uint32_t *)iwl->rx_bd_cpu;
    for (i = 0; i < IWL_8000_RX_N; i++)
        bd[i] = (uint32_t)((iwl->rx_page_dma + (uint64_t)i * IWL_GEN2_RX_SZ) >> 8);
    iwl->rx_read = 0;
    /* Linux rxsq_restock: anillo casi lleno; WPTR alineado a 8 (write & ~7). */
    iwl->rx_write = (uint16_t)(IWL_8000_RX_N - 1u);
    return 0;
}

static void iwl_8000_rx_hw_init(struct iwl_ax211_priv *iwl)
{
    uint32_t cfg;

    if (iwl_grab_nic_access(iwl) != 0)
        return;
    iwl_write32(iwl, FH_MEM_RCSR_CHNL0_CONFIG_REG, 0);
    iwl_write32(iwl, FH_MEM_RCSR_CHNL0_RBDCB_WPTR, 0);
    iwl_write32(iwl, FH_MEM_RCSR_CHNL0_FLUSH_RB_REQ, 0);
    iwl_write32(iwl, FH_RSCSR_CHNL0_RDPTR, 0);
    iwl_write32(iwl, FH_RSCSR_CHNL0_WPTR, 0);
    iwl_write32(iwl, FH_RSCSR_CHNL0_RBDCB_BASE_REG, (uint32_t)(iwl->rx_bd_dma >> 8));
    iwl_write32(iwl, FH_RSCSR_CHNL0_STTS_WPTR_REG, (uint32_t)(iwl->rb_stts_dma >> 4));
    cfg = FH_RCSR_RX_CONFIG_CHNL_EN_ENABLE_VAL |
          FH_RCSR_CHNL0_RX_IGNORE_RXF_EMPTY |
          FH_RCSR_CHNL0_RX_CONFIG_IRQ_DEST_INT_HOST_VAL |
          FH_RCSR_RX_CONFIG_REG_VAL_RB_SIZE_4K |
          (RX_RB_TIMEOUT << FH_RCSR_RX_CONFIG_REG_IRQ_RBTH_POS) |
          ((uint32_t)IWL_8000_RX_LOG << FH_RCSR_RX_CONFIG_RBDCB_SIZE_POS);
    iwl_write32(iwl, FH_MEM_RCSR_CHNL0_CONFIG_REG, cfg);
    iwl_release_nic_access(iwl);
    iwl_write32(iwl, FH_RSCSR_CHNL0_WPTR, (uint32_t)(iwl->rx_write & ~7u));
}

void iwl_trans_8000_drain(struct iwl_ax211_priv *iwl)
{
    uint16_t closed;
    int n = 0;

    if (!iwl->rb_stts || !iwl->rx_page_cpu || !iwl->rx_bd_cpu)
        return;
    closed = iwl_closed_rb_idx(iwl->rb_stts, IWL_8000_RX_N);
    while (iwl->rx_read != closed && n++ < IWL_8000_RX_N) {
        unsigned slot = (unsigned)iwl->rx_read % IWL_8000_RX_N;
        const uint8_t *buf = (const uint8_t *)iwl->rx_page_cpu +
                             (size_t)slot * IWL_GEN2_RX_SZ;

        iwl_trans_rx_packet(iwl, buf, IWL_GEN2_RX_SZ);
        iwl->rx_write = (uint16_t)((iwl->rx_write + 1u) % IWL_8000_RX_N);
        iwl->rx_read = (uint16_t)((iwl->rx_read + 1u) % IWL_8000_RX_N);
        closed = iwl_closed_rb_idx(iwl->rb_stts, IWL_8000_RX_N);
    }
    iwl_write32(iwl, FH_RSCSR_CHNL0_WPTR, (uint32_t)(iwl->rx_write & ~7u));
}

/* Linux pcie/trans.c:687 — solo FH_TX completa el chunk; TSSR idle no hace ACK. */
static int iwl_8000_wait_chunk(struct iwl_ax211_priv *iwl)
{
    int t;

    for (t = 0; t < 5000; t++) {
        uint32_t inta = iwl_read32(iwl, CSR_INT);

        if (inta & CSR_INT_BIT_FH_TX) {
            iwl_write32(iwl, CSR_FH_INT_STATUS, CSR_FH_INT_TX_MASK);
            iwl_write32(iwl, CSR_INT, CSR_INT_BIT_FH_TX);
            return 0;
        }
        lx_udelay(1000);
    }
    lx_printk("iwl_trans: timeout carga FH chunk INT=0x%08x TSSR=0x%08x\n",
              iwl_read32(iwl, CSR_INT), iwl_read32(iwl, FH_TSSR_TX_STATUS_REG));
    return -1;
}

/* Hostcheck: TSSR idle sin FH_TX no debe contar como chunk OK. */
int iwl_8000_chunk_done_policy(uint32_t inta, uint32_t tssr)
{
    if (inta & CSR_INT_BIT_FH_TX)
        return 1;
    if (tssr & FH_TSSR_TX_STATUS_REG_MSK_CHNL_IDLE(FH_SRVC_CHNL))
        return 0;
    return 0;
}

static int iwl_8000_load_chunk(struct iwl_ax211_priv *iwl, uint32_t dst,
                               uint64_t dma, uint32_t len)
{
    struct iwl_8000_fh_prog fh;
    int ext = 0;

    if (dst >= IWL_FW_MEM_EXTENDED_START && dst <= IWL_FW_MEM_EXTENDED_END) {
        uint32_t chick = iwl_read_prph(iwl, LMPM_CHICK);

        iwl_write_prph(iwl, LMPM_CHICK, chick | LMPM_CHICK_EXTENDED_ADDR_SPACE);
        ext = 1;
    }
    iwl_8000_fh_program(dst, dma, len, &fh);
    if (iwl_grab_nic_access(iwl) != 0)
        return -1;
    iwl_write32(iwl, fh.tcsr_cfg_reg, fh.tcsr_pause);
    iwl_write32(iwl, fh.sram_reg, fh.sram_val);
    iwl_write32(iwl, fh.tfdib0_reg, fh.tfdib0_val);
    iwl_write32(iwl, fh.tfdib1_reg, fh.tfdib1_val);
    iwl_write32(iwl, fh.buf_sts_reg, fh.buf_sts_val);
    iwl_write32(iwl, fh.tcsr_cfg_reg, fh.tcsr_run);
    iwl_release_nic_access(iwl);
    if (iwl_8000_wait_chunk(iwl) != 0)
        return -1;
    if (ext) {
        uint32_t chick = iwl_read_prph(iwl, LMPM_CHICK);

        iwl_write_prph(iwl, LMPM_CHICK, chick & ~LMPM_CHICK_EXTENDED_ADDR_SPACE);
    }
    return 0;
}

static int iwl_8000_load_cpu_sections(struct iwl_ax211_priv *iwl,
                                      const struct iwl_fw_rt_section *secs,
                                      int n_secs, int cpu, int *first_sec,
                                      void *chunk_cpu, uint64_t chunk_dma,
                                      uint32_t chunk_sz)
{
    int shift = cpu == 1 ? 0 : 16;
    uint32_t sec_num = 1;
    int last = *first_sec;
    int si;

    if (cpu == 2)
        (*first_sec)++;
    for (si = *first_sec; si < n_secs; si++) {
        uint32_t dest = 0;
        const uint8_t *payload = 0;
        uint32_t plen = 0;
        uint32_t off;
        int kind = rt_dest(&secs[si], &dest, &payload, &plen);

        last = si;
        if (kind != 0)
            break;
        for (off = 0; off < plen; off += chunk_sz) {
            uint32_t n = plen - off;

            if (n > chunk_sz)
                n = chunk_sz;
            memcpy(chunk_cpu, payload + off, n);
            if (iwl_8000_load_chunk(iwl, dest + off, chunk_dma, n) != 0)
                return -1;
        }
        {
            uint32_t val = iwl_read_direct32(iwl, FH_UCODE_LOAD_STATUS);

            val |= (sec_num << shift);
            iwl_write_direct32(iwl, FH_UCODE_LOAD_STATUS, val);
        }
        sec_num = (sec_num << 1) | 1u;
    }
    *first_sec = last;

    /* Linux load_cpu_sections_8000: máscara completa antes del kick final. */
    iwl_write32(iwl, CSR_INT_MASK, CSR_INI_SET_MASK);
    if (cpu == 1)
        iwl_write_direct32(iwl, FH_UCODE_LOAD_STATUS, 0xFFFFu);
    else
        iwl_write_direct32(iwl, FH_UCODE_LOAD_STATUS, 0xFFFFFFFFu);
    {
        uint32_t load_st = iwl_read_direct32(iwl, FH_UCODE_LOAD_STATUS);
        uint32_t sb1 = iwl_read_prph(iwl, SB_CPU_1_STATUS);
        uint32_t sb2 = iwl_read_prph(iwl, SB_CPU_2_STATUS);
        uint32_t tssr = iwl_read32(iwl, FH_TSSR_TX_STATUS_REG);

        lx_printk("iwl_trans: kick CPU%d LOAD_ST=0x%08x SB1=0x%08x SB2=0x%08x "
                  "FH_TSSR=0x%08x\n",
                  cpu, load_st, sb1, sb2, tssr);
    }
    return 0;
}

static int iwl_8000_run_load(struct iwl_ax211_priv *iwl,
                             const struct iwl_8000_load_plan *plan,
                             const struct iwl_fw_rt_section *secs, int n_secs,
                             void *chunk_cpu, uint64_t chunk_dma, uint32_t chunk_sz)
{
    int first = 0;

    (void)plan;
    if (iwl_8000_load_cpu_sections(iwl, secs, n_secs, 1, &first, chunk_cpu,
                                   chunk_dma, chunk_sz) != 0)
        return -1;
    return iwl_8000_load_cpu_sections(iwl, secs, n_secs, 2, &first, chunk_cpu,
                                    chunk_dma, chunk_sz);
}

static void iwl_8000_log_alive_timeout(struct iwl_ax211_priv *iwl, const char *phase)
{
    uint32_t inta = iwl_read32(iwl, CSR_INT);
    uint32_t gp = iwl_read32(iwl, CSR_GP_CNTRL);
    uint16_t rb_hw = iwl->rb_stts ? (iwl->rb_stts[0] & 0x0fffu) : 0;
    uint32_t wptr = iwl_read32(iwl, FH_RSCSR_CHNL0_WPTR);
    uint32_t load_st = iwl_read_direct32(iwl, FH_UCODE_LOAD_STATUS);

    {
        uint32_t sb1 = iwl_read_prph(iwl, SB_CPU_1_STATUS);
        uint32_t sb2 = iwl_read_prph(iwl, SB_CPU_2_STATUS);

        lx_printk("iwl_trans: timeout ALIVE %s (8265/8000) INT=0x%08x GP=0x%08x "
                  "rb_hw=0x%03x rx_read=%u rx_write=%u WPTR=0x%x LOAD_ST=0x%08x "
                  "SB1=0x%08x SB2=0x%08x%s%s\n",
                  phase, inta, gp, rb_hw, (unsigned)iwl->rx_read,
                  (unsigned)iwl->rx_write, wptr, load_st, sb1, sb2,
                  (inta & CSR_INT_BIT_SW_ERR) ? " SW_ERR" : "",
                  !(gp & CSR_GP_CNTRL_REG_FLAG_HW_RF_KILL_SW) ? " RF_KILL" : "");
    }
}

unsigned iwl_8000_tx_slots(unsigned qid, unsigned cmd_qid)
{
    if (qid == cmd_qid)
        return IWL_CMD_QUEUE_SIZE;
    return IWL_DEFAULT_QUEUE_SIZE;
}

void iwl_8000_tx_init_program(uint64_t kw_dma, const uint64_t *ring_dma,
                              unsigned n_queues, struct iwl_8000_tx_init_prog *out)
{
    unsigned qid;

    if (!out)
        return;
    memset(out, 0, sizeof(*out));
    out->scd_txfact = 0;
    out->kw_reg = FH_KW_MEM_ADDR_REG;
    out->kw_val = (uint32_t)(kw_dma >> 4);
    if (ring_dma) {
        for (qid = 0; qid < n_queues && qid < IWL_8000_NUM_QUEUES; qid++)
            out->cbbc[qid] = (uint32_t)(ring_dma[qid] >> 8);
    }
    out->gp_ctrl_set = SCD_GP_CTRL_AUTO_ACTIVE_MODE;
    if (n_queues > 20)
        out->gp_ctrl_set |= SCD_GP_CTRL_ENABLE_31_QUEUES;
}

int iwl_8000_tx_preload_ready(const struct iwl_ax211_priv *iwl)
{
    return iwl && iwl->tx_preload_ready;
}

static void iwl_8000_set_bits_prph(struct iwl_ax211_priv *iwl, uint32_t addr, uint32_t bits)
{
    uint32_t v = iwl_read_prph(iwl, addr);

    iwl_write_prph(iwl, addr, v | bits);
}

static int iwl_8000_tx_alloc(struct iwl_ax211_priv *iwl)
{
    unsigned qid;

    if (!iwl->kw_cpu) {
        iwl->kw_cpu = lx_dma_alloc_coherent(0, 4096, &iwl->kw_dma, GFP_KERNEL);
        if (!iwl->kw_cpu)
            return -1;
    }
    for (qid = 0; qid < IWL_8000_NUM_QUEUES; qid++) {
        unsigned slots = iwl_8000_tx_slots(qid, (unsigned)iwl->cmd_qid);
        size_t bytes = (size_t)slots * IWL_GEN1_TFD_SIZE;

        if (!iwl->tx_ring_cpu[qid]) {
            iwl->tx_ring_cpu[qid] = lx_dma_alloc_coherent(0, bytes,
                                                          &iwl->tx_ring_dma[qid],
                                                          GFP_KERNEL);
            if (!iwl->tx_ring_cpu[qid])
                return -1;
        }
        memset(iwl->tx_ring_cpu[qid], 0, bytes);
    }
    return 0;
}

/* Linux pcie/tx.c:546 — tx_init completo antes de load_given_ucode_8000. */
static int iwl_8000_tx_init(struct iwl_ax211_priv *iwl)
{
    unsigned qid;

    if (iwl_8000_tx_alloc(iwl) != 0)
        return -1;

    iwl_write_prph(iwl, SCD_TXFACT, 0);
    iwl_write_direct32(iwl, FH_KW_MEM_ADDR_REG, (uint32_t)(iwl->kw_dma >> 4));

    for (qid = 0; qid < IWL_8000_NUM_QUEUES; qid++) {
        if (iwl_grab_nic_access(iwl) != 0)
            return -1;
        iwl_write32(iwl, FH_MEM_CBBC_QUEUE(qid),
                    (uint32_t)(iwl->tx_ring_dma[qid] >> 8));
        iwl_release_nic_access(iwl);
    }

    iwl_8000_set_bits_prph(iwl, SCD_GP_CTRL, SCD_GP_CTRL_AUTO_ACTIVE_MODE);
    if (IWL_8000_NUM_QUEUES > 20)
        iwl_8000_set_bits_prph(iwl, SCD_GP_CTRL, SCD_GP_CTRL_ENABLE_31_QUEUES);

    iwl->tx_preload_ready = 1;
    lx_printk("iwl_trans: tx_init 8000 %u colas CBBC+SCD_GP_CTRL ok\n",
              (unsigned)IWL_8000_NUM_QUEUES);
    return 0;
}

static void iwl_8000_poll_alive_int(struct iwl_ax211_priv *iwl)
{
    uint32_t inta = iwl_read32(iwl, CSR_INT);

    if (inta == 0xffffffffu)
        return;

    /* Linux pcie/rx.c:1902 — ACK pending bits before service. */
    iwl_write32(iwl, CSR_INT, inta);

    if (inta & (CSR_INT_BIT_FH_RX | CSR_INT_BIT_SW_RX))
        iwl_write32(iwl, CSR_FH_INT_STATUS, CSR_FH_INT_RX_MASK);
    if (inta & CSR_INT_BIT_FH_TX)
        iwl_write32(iwl, CSR_FH_INT_STATUS, CSR_FH_INT_TX_MASK);

    iwl_trans_8000_drain(iwl);
}

static int iwl_8000_wait_alive(struct iwl_ax211_priv *iwl, const char *phase)
{
    unsigned i;

    for (i = 0; i < 500; i++) {
        iwl_8000_poll_alive_int(iwl);
        if (iwl->alive)
            return 0;
        lx_mdelay(10);
    }
    iwl_8000_log_alive_timeout(iwl, phase);
    return -1;
}

static void iwl_8000_reset_ict(struct iwl_ax211_priv *iwl)
{
    if (!iwl->ict_cpu)
        return;
    memset(iwl->ict_cpu, 0, IWL_ICT_SIZE);
    {
        uint32_t val = (uint32_t)(iwl->ict_dma >> IWL_ICT_SHIFT);

        val |= CSR_DRAM_INT_TBL_ENABLE | CSR_DRAM_INIT_TBL_WRAP_CHECK |
               CSR_DRAM_INIT_TBL_WRITE_POINTER;
        iwl_write32(iwl, CSR_DRAM_INT_TBL_REG, val);
    }
}

static void iwl_8000_begin_fw_load(struct iwl_ax211_priv *iwl)
{
    /* Linux load_given_ucode_8000: WFPM → RELEASE → carga inmediata. */
    iwl_write_prph(iwl, WFPM_GP2, 0x01010101u);
    iwl_write_direct32(iwl, FH_UCODE_LOAD_STATUS, 0);
    iwl_write32(iwl, CSR_INT_MASK, CSR_INT_BIT_FH_TX);
    iwl_write_prph(iwl, RELEASE_CPU_RESET, RELEASE_CPU_RESET_BIT);
}

static int iwl_8000_start_hw(struct iwl_ax211_priv *iwl)
{
    if (iwl_prepare_card_hw(iwl) != 0)
        return -1;
    iwl_sw_reset(iwl);
    if (iwl_prepare_card_hw(iwl) != 0)
        return -1;

    iwl_write32(iwl, CSR_INT, 0xffffffffu);
    if (iwl_pcie_check_hw_rf_kill(iwl) != 0)
        return -1;
    iwl_write32(iwl, CSR_UCODE_DRV_GP1_CLR, CSR_UCODE_SW_BIT_RFKILL);
    iwl_write32(iwl, CSR_UCODE_DRV_GP1_CLR, CSR_UCODE_DRV_GP1_BIT_CMD_BLOCKED);
    iwl_write32(iwl, CSR_INT, 0xffffffffu);

    if (iwl_8000_apm_init(iwl) != 0) {
        lx_printk("iwl_trans: APM 8000 falló GP=0x%08x\n", iwl_read32(iwl, CSR_GP_CNTRL));
        return -1;
    }
    lx_printk("iwl_trans: APM 8000 ok GP=0x%08x step=0x%x\n",
              iwl_read32(iwl, CSR_GP_CNTRL), (unsigned)(iwl->hw_rev & 0xfu));
    iwl_write8(iwl, CSR_INT_COALESCING, IWL_HOST_INT_TIMEOUT_DEF);
    if (iwl_pcie_check_hw_rf_kill(iwl) != 0)
        return -1;

    if (iwl_8000_alloc_rx(iwl) != 0)
        return -1;
    iwl_8000_rx_hw_init(iwl);
    if (iwl_8000_tx_init(iwl) != 0)
        return -1;
    iwl_write32(iwl, CSR_INT, 0xffffffffu);
    return 0;
}

static int iwl_8000_reinit_for_rt(struct iwl_ax211_priv *iwl)
{
    iwl_sw_reset(iwl);
    iwl->alive = 0;
    lx_iwlwifi_set_alive(0);
    iwl->tx_8000_ready = 0;
    iwl->tx_preload_ready = 0;
    return iwl_8000_start_hw(iwl);
}

int iwl_trans_8000_start(struct iwl_ax211_priv *iwl)
{
    struct iwl_8000_load_plan plan_init;
    struct iwl_8000_load_plan plan_rt;
    void *chunk_cpu;
    uint64_t chunk_dma = 0;
    uint32_t chunk_sz = FH_MEM_TB_MAX_LENGTH;

    if (!iwl || !iwl->mmio)
        return -1;
    if (!iwl->fw.sec_init_n || !iwl->fw.rt_n) {
        lx_printk("iwl_trans: 8000 requiere SEC_INIT (%d) y SEC_RT (%d)\n",
                  iwl->fw.sec_init_n, iwl->fw.rt_n);
        return -1;
    }
    if (iwl_8000_plan_load_init(&iwl->fw, &plan_init) != 0) {
        lx_printk("iwl_trans: plan carga 8000 inválido (SEC_INIT)\n");
        return -1;
    }
    if (iwl_8000_plan_load(&iwl->fw, &plan_rt) != 0) {
        lx_printk("iwl_trans: plan carga 8000 inválido (SEC_RT)\n");
        return -1;
    }

    if (!iwl->ict_cpu)
        iwl->ict_cpu = lx_dma_alloc_coherent(0, IWL_ICT_SIZE, &iwl->ict_dma,
                                             GFP_KERNEL);

    chunk_cpu = lx_dma_alloc_coherent(0, chunk_sz, &chunk_dma, GFP_KERNEL);
    if (!chunk_cpu) {
        chunk_sz = 4096;
        chunk_cpu = lx_dma_alloc_coherent(0, chunk_sz, &chunk_dma, GFP_KERNEL);
    }
    if (!chunk_cpu)
        return -1;

    if (iwl_8000_start_hw(iwl) != 0)
        return -1;
    if (!iwl->tx_preload_ready) {
        lx_printk("iwl_trans: tx_init 8000 incompleto; no se carga INIT\n");
        return -1;
    }

    iwl_8000_begin_fw_load(iwl);
    lx_printk("iwlwifi: RELEASE_CPU_RESET; carga FH INIT cpu1=%u cpu2=%u chunks=%u\n",
              (unsigned)plan_init.cpu1_secs, (unsigned)plan_init.cpu2_secs,
              (unsigned)plan_init.n_chunks);
    if (iwl_8000_run_load(iwl, &plan_init, iwl->fw.sec_init, iwl->fw.sec_init_n,
                          chunk_cpu, chunk_dma, chunk_sz) != 0)
        return -1;
    if (iwl_8000_wait_alive(iwl, "INIT") != 0)
        return -1;
    lx_printk("iwl_trans: INIT ucode ALIVE ok\n");

    if (iwl_8000_reinit_for_rt(iwl) != 0)
        return -1;

    iwl_8000_begin_fw_load(iwl);
    lx_printk("iwlwifi: carga FH RT cpu1=%u cpu2=%u chunks=%u\n",
              (unsigned)plan_rt.cpu1_secs, (unsigned)plan_rt.cpu2_secs,
              (unsigned)plan_rt.n_chunks);
    if (iwl_8000_run_load(iwl, &plan_rt, iwl->fw.rt, iwl->fw.rt_n,
                          chunk_cpu, chunk_dma, chunk_sz) != 0)
        return -1;

    if (iwl_trans_8000_alloc_hcmd(iwl) != 0) {
        lx_printk("iwl_trans: reserva HCMD 8000 falló\n");
        return -1;
    }

    if (iwl_8000_wait_alive(iwl, "RT") != 0)
        return -1;
    iwl_8000_reset_ict(iwl);
    if (iwl_trans_8000_fw_alive(iwl) != 0) {
        lx_printk("iwl_trans: fw_alive 8000 falló\n");
        return -1;
    }
    return 0;
}

static void iwl_8000_write_mem32(struct iwl_ax211_priv *iwl, uint32_t addr, uint32_t val)
{
    if (iwl_grab_nic_access(iwl) != 0)
        return;
    iwl_write32(iwl, HBUS_TARG_MEM_WADDR, addr);
    iwl_write32(iwl, HBUS_TARG_MEM_WDAT, val);
    iwl_release_nic_access(iwl);
}

static void iwl_8000_clear_scd_ctx(struct iwl_ax211_priv *iwl)
{
    unsigned off;
    unsigned end = SCD_TRANS_TBL_OFFSET_QUEUE(IWL_8000_NUM_QUEUES);

    if (!iwl->scd_base_addr)
        return;
    for (off = SCD_CONTEXT_MEM_LOWER_BOUND; off < end; off += 4u)
        iwl_8000_write_mem32(iwl, iwl->scd_base_addr + off, 0);
}

static void iwl_8000_scd_txq_inactive(struct iwl_ax211_priv *iwl, unsigned qid)
{
    iwl_write_prph(iwl, SCD_QUEUE_STATUS_BITS(qid),
                   (0u << SCD_QUEUE_STTS_REG_POS_ACTIVE) |
                   (1u << SCD_QUEUE_STTS_REG_POS_SCD_ACT_EN));
}

static void iwl_8000_ac_cmd_enable(struct iwl_ax211_priv *iwl)
{
    unsigned qid = (unsigned)iwl->cmd_qid;
    uint32_t ctx2 = ((SCD_WIN_SIZE & 0x7fu) |
                     ((SCD_FRAME_LIMIT & 0x7fu) << 16));

    iwl_write_prph(iwl, SCD_EN_CTRL, 0);
    iwl_8000_scd_txq_inactive(iwl, qid);
    iwl->cmd_write = 0;
    iwl->cmd_read = 0;
    iwl_write32(iwl, HBUS_TARG_WRPTR, (uint32_t)(qid << 8));
    iwl_write_prph(iwl, SCD_QUEUE_RDPTR(qid), 0);
    iwl_8000_write_mem32(iwl, iwl->scd_base_addr + SCD_CONTEXT_QUEUE_OFFSET(qid), 0);
    iwl_8000_write_mem32(iwl, iwl->scd_base_addr + SCD_CONTEXT_QUEUE_OFFSET(qid) + 4u,
                          ctx2);
    iwl_write_prph(iwl, SCD_QUEUE_STATUS_BITS(qid),
                   (1u << SCD_QUEUE_STTS_REG_POS_ACTIVE) |
                   (IWL_MVM_TX_FIFO_CMD << SCD_QUEUE_STTS_REG_POS_TXF) |
                   (1u << SCD_QUEUE_STTS_REG_POS_WSL) |
                   SCD_QUEUE_STTS_REG_MSK);
    iwl_write_prph(iwl, SCD_EN_CTRL, 1u << qid);
}

int iwl_trans_8000_alloc_hcmd(struct iwl_ax211_priv *iwl)
{
    unsigned i;
    unsigned bc_bytes = IWL_8000_NUM_QUEUES * IWL_SCD_BC_TBL_BYTES;
    unsigned tfd_bytes = IWL_8000_TFD_RING_N * IWL_GEN1_TFD_SIZE;

    if (iwl->tx_8000_ready)
        return 0;
    if (iwl->mtr_cpu && iwl->mcr_cpu && iwl->hcmd_first_tb_cpu && iwl->scd_bc_cpu)
        goto reiniciar;
    iwl->scd_bc_cpu = lx_dma_alloc_coherent(0, bc_bytes, &iwl->scd_bc_dma, GFP_KERNEL);
    iwl->mtr_cpu = lx_dma_alloc_coherent(0, tfd_bytes, &iwl->mtr_dma, GFP_KERNEL);
    iwl->mcr_cpu = lx_dma_alloc_coherent(0, IWL_CMD_QUEUE_SIZE * IWL_CMD_SLOT_SIZE,
                                         &iwl->mcr_dma, GFP_KERNEL);
    iwl->hcmd_first_tb_cpu = lx_dma_alloc_coherent(
        0, IWL_CMD_QUEUE_SIZE * IWL_FIRST_TB_SIZE_ALIGN, &iwl->hcmd_first_tb_dma,
        GFP_KERNEL);
    if (!iwl->scd_bc_cpu || !iwl->mtr_cpu || !iwl->mcr_cpu || !iwl->hcmd_first_tb_cpu)
        return -1;
reiniciar:
    memset(iwl->scd_bc_cpu, 0, bc_bytes);
    memset(iwl->mtr_cpu, 0, tfd_bytes);
    memset(iwl->mcr_cpu, 0, IWL_CMD_QUEUE_SIZE * IWL_CMD_SLOT_SIZE);
    memset(iwl->hcmd_first_tb_cpu, 0, IWL_CMD_QUEUE_SIZE * IWL_FIRST_TB_SIZE_ALIGN);
    for (i = 0; i < IWL_8000_TFD_RING_N; i++) {
        struct iwl_tfd *tfd = (struct iwl_tfd *)((uint8_t *)iwl->mtr_cpu +
                                                 (size_t)i * IWL_GEN1_TFD_SIZE);
        memset(tfd, 0, sizeof(*tfd));
    }
    iwl->cmd_write = 0;
    iwl->cmd_read = 0;
    if (iwl_grab_nic_access(iwl) == 0) {
        iwl_write32(iwl, FH_MEM_CBBC_QUEUE(iwl->cmd_qid),
                    (uint32_t)(iwl->mtr_dma >> 8));
        iwl_release_nic_access(iwl);
    }
    iwl->tx_8000_ready = 1;
    lx_printk("iwl_trans: HCMD 8000 mtr=0x%llx mcr=0x%llx bc=0x%llx\n",
              (unsigned long long)iwl->mtr_dma, (unsigned long long)iwl->mcr_dma,
              (unsigned long long)iwl->scd_bc_dma);
    return 0;
}

int iwl_trans_8000_fw_alive(struct iwl_ax211_priv *iwl)
{
    uint32_t reg_val;
    int chan;

    if (!iwl || !iwl->alive || !iwl->tx_8000_ready)
        return -1;
    iwl->scd_base_addr = iwl_read_prph(iwl, SCD_SRAM_BASE_ADDR);
    iwl_8000_clear_scd_ctx(iwl);
    iwl_write_prph(iwl, SCD_DRAM_BASE_ADDR, (uint32_t)(iwl->scd_bc_dma >> 10));
    iwl_write_prph(iwl, SCD_CHAINEXT_EN, 0);
    iwl_8000_ac_cmd_enable(iwl);
    iwl_write_prph(iwl, SCD_TXFACT, 0xffu);
    for (chan = 0; chan < FH_TCSR_CHNL_NUM; chan++) {
        iwl_write_direct32(iwl, FH_TCSR_CHNL_TX_CONFIG_REG(chan),
                           FH_TCSR_TX_CONFIG_REG_VAL_DMA_CHNL_ENABLE |
                           FH_TCSR_TX_CONFIG_REG_VAL_DMA_CREDIT_ENABLE);
    }
    reg_val = iwl_read_direct32(iwl, FH_TX_CHICKEN_BITS_REG);
    iwl_write_direct32(iwl, FH_TX_CHICKEN_BITS_REG,
                        reg_val | FH_TX_CHICKEN_BITS_SCD_AUTO_RETRY_EN);
    lx_printk("iwl_trans: fw_alive 8000 SCD=0x%08x qid=%u\n",
              iwl->scd_base_addr, (unsigned)iwl->cmd_qid);
    return 0;
}
