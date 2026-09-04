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

static void iwl_write64(struct iwl_ax211_priv *iwl, uint32_t off, uint64_t val)
{
    iwl_write32(iwl, off, (uint32_t)(val & 0xffffffffu));
    iwl_write32(iwl, off + 4, (uint32_t)(val >> 32));
}

static int iwl_wait_mac_ready(struct iwl_ax211_priv *iwl, int ms)
{
    while (ms-- > 0) {
        uint32_t gp = iwl_read32(iwl, CSR_GP_CNTRL);
        if (gp & CSR_GP_CNTRL_REG_FLAG_MAC_CLOCK_READY)
            return 0;
        lx_mdelay(1);
    }
    return -1;
}

static void iwl_reset(struct iwl_ax211_priv *iwl)
{
    iwl_write32(iwl, CSR_RESET, CSR_RESET_REG_FLAG_SW_RESET);
    lx_mdelay(10);
    iwl_write32(iwl, CSR_RESET, 0);
    lx_mdelay(10);
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

static void handle_notification(struct iwl_ax211_priv *iwl, struct iwl_rx_packet *pkt)
{
    uint16_t len = pkt->len_n_flags & 0x3fff;
    if (len < sizeof(*pkt))
        return;
    if (pkt->group_id == 0 && pkt->id == UCODE_ALIVE_NTFY) {
        iwl->alive = 1;
        lx_iwlwifi_set_alive(1);
        lx_printk("iwl_ax211: firmware ALIVE (UCODE_ALIVE_NTFY)\n");
        return;
    }
    if (pkt->group_id == SCAN_GROUP) {
        parse_scan_notify(iwl, pkt->data, (int)len - (int)sizeof(*pkt));
    }
    if (pkt->group_id == DATA_PATH_GROUP && pkt->id == 0x1) {
        int pay = (int)len - (int)sizeof(*pkt);
        if (pay > 14)
            iwl_ax211_deliver_rx(pkt->data, pay);
    }
}

static void drain_mcr(struct iwl_ax211_priv *iwl)
{
    while (iwl->mcr_read != iwl->mcr_write) {
        struct iwl_rx_packet *pkt =
            (struct iwl_rx_packet *)(iwl->mcr_cpu + (iwl->mcr_read % IWL_MCR_SIZE) * 256);
        handle_notification(iwl, pkt);
        iwl->mcr_read = (iwl->mcr_read + 1) % IWL_MCR_SIZE;
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
        lx_printk("iwlwifi: firmware ALIVE (AX200 gen2)\n");
        return;
    }
    if (group == SCAN_GROUP)
        parse_scan_notify(iwl, data, pay);
    if (group == DATA_PATH_GROUP && cmd == 0x1 && pay > 14)
        iwl_ax211_deliver_rx(data, pay);
}

static void drain_rx_gen2(struct iwl_ax211_priv *iwl)
{
    uint16_t *used;
    uint64_t *bd;
    uint16_t hw;
    int n = 0;
    if (!iwl->rb_stts || !iwl->used_bd_cpu || !iwl->rx_page_cpu || !iwl->rx_bd_cpu)
        return;
    used = (uint16_t *)iwl->used_bd_cpu;
    bd = (uint64_t *)iwl->rx_bd_cpu;
    hw = iwl->rb_stts[0] & 0x0fff;
    while (iwl->rx_read != hw && n++ < IWL_GEN2_RX_N) {
        uint16_t idx = used[iwl->rx_read % IWL_GEN2_RX_N] & 0x0fff;
        if (idx < IWL_GEN2_RX_N) {
            handle_gen2_rx(iwl, (const uint8_t *)iwl->rx_page_cpu + (size_t)idx * IWL_GEN2_RX_SZ);
            bd[iwl->rx_write % IWL_GEN2_RX_N] =
                iwl->rx_page_dma + (uint64_t)idx * IWL_GEN2_RX_SZ;
            iwl->rx_write = (uint16_t)((iwl->rx_write + 1) % IWL_GEN2_RX_N);
        }
        iwl->rx_read = (uint16_t)((iwl->rx_read + 1) % IWL_GEN2_RX_N);
        hw = iwl->rb_stts[0] & 0x0fff;
    }
    iwl_write32(iwl, RFH_Q0_FRBDCB_WIDX_TRG, (uint32_t)(iwl->rx_write & ~7u));
}

int iwl_trans_gen2_start(struct iwl_ax211_priv *iwl)
{
    struct iwl_context_info *ctxt;
    uint64_t *bd;
    unsigned i;

    iwl_reset(iwl);
    iwl_write32(iwl, CSR_GP_CNTRL, CSR_GP_CNTRL_REG_FLAG_MAC_ACCESS_REQ);
    if (iwl_wait_mac_ready(iwl, 2000) != 0)
        return -1;

    ctxt = lx_dma_alloc_coherent(0, sizeof(*ctxt), &iwl->ctxt_dma, GFP_KERNEL);
    iwl->rx_bd_cpu = lx_dma_alloc_coherent(0, IWL_GEN2_RX_N * 8, &iwl->rx_bd_dma, GFP_KERNEL);
    iwl->used_bd_cpu = lx_dma_alloc_coherent(0, IWL_GEN2_RX_N * 2, &iwl->used_bd_dma, GFP_KERNEL);
    iwl->rb_stts = (volatile uint16_t *)lx_dma_alloc_coherent(0, 16, &iwl->rb_stts_dma, GFP_KERNEL);
    iwl->rx_page_cpu = lx_dma_alloc_coherent(0, IWL_GEN2_RX_N * IWL_GEN2_RX_SZ,
                                             &iwl->rx_page_dma, GFP_KERNEL);
    iwl->mtr_cpu = lx_dma_alloc_coherent(0, IWL_CMD_QUEUE_SIZE * IWL_TFH_TFD_SIZE,
                                         &iwl->mtr_dma, GFP_KERNEL);
    iwl->mcr_cpu = lx_dma_alloc_coherent(0, IWL_CMD_QUEUE_SIZE * 256, &iwl->mcr_dma, GFP_KERNEL);
    if (!ctxt || !iwl->rx_bd_cpu || !iwl->used_bd_cpu || !iwl->rb_stts ||
        !iwl->rx_page_cpu || !iwl->mtr_cpu || !iwl->mcr_cpu)
        return -1;

    memset(ctxt, 0, sizeof(*ctxt));
    memset(iwl->rx_bd_cpu, 0, IWL_GEN2_RX_N * 8);
    memset(iwl->used_bd_cpu, 0, IWL_GEN2_RX_N * 2);
    memset((void *)iwl->rb_stts, 0, 16);
    memset(iwl->mtr_cpu, 0, IWL_CMD_QUEUE_SIZE * IWL_TFH_TFD_SIZE);
    memset(iwl->mcr_cpu, 0, IWL_CMD_QUEUE_SIZE * 256);

    bd = (uint64_t *)iwl->rx_bd_cpu;
    for (i = 0; i < IWL_GEN2_RX_N - 1; i++)
        bd[i] = iwl->rx_page_dma + (uint64_t)i * IWL_GEN2_RX_SZ;
    iwl->rx_write = IWL_GEN2_RX_N - 1;
    iwl->rx_read = 0;
    iwl->cmd_write = 0;
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
    iwl_write64(iwl, CSR_CTXT_INFO_BA, iwl->ctxt_dma);
    lx_printk("iwlwifi: context-info gen2 BA=0x%llx\n", (unsigned long long)iwl->ctxt_dma);

    for (int t = 0; t < 500; t++) {
        drain_rx_gen2(iwl);
        if (iwl->alive)
            return 0;
        lx_mdelay(10);
    }
    lx_printk("iwl_trans: timeout ALIVE (AX200 gen2)\n");
    return -1;
}

int iwl_trans_gen3_start(struct iwl_ax211_priv *iwl)
{
    uint64_t dma;
    struct iwl_prph_scratch *scratch;
    struct iwl_prph_info *info;
    struct iwl_context_info_gen3 *ctxt;

    iwl_reset(iwl);
    if (iwl_wait_mac_ready(iwl, 2000) != 0)
        return -1;

    scratch = lx_dma_alloc_coherent(0, sizeof(*scratch), &iwl->scratch_dma, GFP_KERNEL);
    info = lx_dma_alloc_coherent(0, sizeof(*info), &iwl->info_dma, GFP_KERNEL);
    ctxt = lx_dma_alloc_coherent(0, sizeof(*ctxt), &iwl->ctxt_dma, GFP_KERNEL);
    if (!scratch || !info || !ctxt)
        return -1;

    memset(scratch, 0, sizeof(*scratch));
    scratch->ctrl_cfg.version.version = 1;
    scratch->ctrl_cfg.version.size = sizeof(*scratch) / 4;
    scratch->ctrl_cfg.control.control_flags = IWL_PRPH_SCRATCH_RB_SIZE_4K;

    if (iwl->pnvm_data && iwl->pnvm_len) {
        void *pnvm_cpu = lx_dma_alloc_coherent(0, iwl->pnvm_len, &dma, GFP_KERNEL);
        if (pnvm_cpu) {
            memcpy(pnvm_cpu, iwl->pnvm_data, iwl->pnvm_len);
            scratch->ctrl_cfg.pnvm_cfg.pnvm_base_addr = dma;
            scratch->ctrl_cfg.pnvm_cfg.pnvm_size = iwl->pnvm_len;
        }
    }

    if (iwl_fw_upload_sections(iwl, &scratch->dram) != 0)
        return -1;

    memset(info, 0, sizeof(*info));
    memset(ctxt, 0, sizeof(*ctxt));
    ctxt->version = 1;
    ctxt->size = sizeof(*ctxt) / 4;
    ctxt->prph_info_base_addr = iwl->info_dma;
    ctxt->prph_scratch_base_addr = iwl->scratch_dma;
    ctxt->prph_scratch_size = sizeof(*scratch) / 4;
    ctxt->mtr_size = IWL_MTR_SIZE;
    ctxt->mcr_size = IWL_MCR_SIZE;

    iwl->mtr_cpu = lx_dma_alloc_coherent(0, IWL_MTR_SIZE * 256, &iwl->mtr_dma, GFP_KERNEL);
    iwl->mcr_cpu = lx_dma_alloc_coherent(0, IWL_MCR_SIZE * 256, &iwl->mcr_dma, GFP_KERNEL);
    if (!iwl->mtr_cpu || !iwl->mcr_cpu)
        return -1;
    ctxt->mtr_base_addr = iwl->mtr_dma;
    ctxt->mcr_base_addr = iwl->mcr_dma;

    uint64_t cr_head_dma = 0, cr_tail_dma = 0, tr_head_dma = 0, tr_tail_dma = 0;
    void *cr_head_cpu = lx_dma_alloc_coherent(0, 64, &cr_head_dma, GFP_KERNEL);
    void *cr_tail_cpu = lx_dma_alloc_coherent(0, 64, &cr_tail_dma, GFP_KERNEL);
    void *tr_head_cpu = lx_dma_alloc_coherent(0, 64, &tr_head_dma, GFP_KERNEL);
    void *tr_tail_cpu = lx_dma_alloc_coherent(0, 64, &tr_tail_dma, GFP_KERNEL);
    if (!cr_head_cpu || !cr_tail_cpu || !tr_head_cpu || !tr_tail_cpu)
        return -1;
    (void)cr_head_cpu;
    (void)cr_tail_cpu;
    (void)tr_head_cpu;
    (void)tr_tail_cpu;
    iwl->cr_head = cr_head_dma;
    iwl->cr_tail = cr_tail_dma;
    iwl->tr_head = tr_head_dma;
    iwl->tr_tail = tr_tail_dma;
    ctxt->cr_head_idx_arr_base_addr = cr_head_dma;
    ctxt->cr_tail_idx_arr_base_addr = cr_tail_dma;
    ctxt->tr_head_idx_arr_base_addr = tr_head_dma;
    ctxt->tr_tail_idx_arr_base_addr = tr_tail_dma;
    ctxt->cr_idx_arr_size = 1;
    ctxt->tr_idx_arr_size = 1;

    iwl_write32(iwl, CSR_GP_CNTRL, CSR_GP_CNTRL_REG_FLAG_MAC_ACCESS_REQ);
    if (iwl_wait_mac_ready(iwl, 2000) != 0)
        return -1;

    iwl_write64(iwl, CSR_CTXT_INFO_ADDR, iwl->ctxt_dma);
    iwl_write32(iwl, CSR_CTXT_INFO_BOOT_CTRL, CSR_AUTO_FUNC_BOOT_ENA);

    for (int i = 0; i < 500; i++) {
        drain_mcr(iwl);
        if (iwl->alive)
            return 0;
        lx_mdelay(10);
    }
    lx_printk("iwl_trans: timeout ALIVE\n");
    return -1;
}

void iwl_trans_poll(struct iwl_ax211_priv *iwl)
{
    if (!iwl->mmio)
        return;
    if (iwl->gen3) {
        if (iwl->alive)
            drain_mcr(iwl);
    } else {
        drain_rx_gen2(iwl);
    }
    (void)iwl_read32(iwl, CSR_INT);
}

int iwl_trans_send_cmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                       const void *payload, uint16_t pay_len)
{
    if (!iwl->alive || !iwl->mtr_cpu)
        return -1;
    if (pay_len + sizeof(struct iwl_cmd_header) > 240)
        return -1;

    if (!iwl->gen3) {
        uint16_t slot = iwl->cmd_write % IWL_CMD_QUEUE_SIZE;
        uint8_t *buf = (uint8_t *)iwl->mcr_cpu + slot * 256;
        struct iwl_cmd_header *hdr = (struct iwl_cmd_header *)buf;
        struct iwl_tfh_tfd_long *tfd =
            (struct iwl_tfh_tfd_long *)((uint8_t *)iwl->mtr_cpu + (size_t)slot * IWL_TFH_TFD_SIZE);
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

    uint16_t slot = iwl->mtr_write % IWL_MTR_SIZE;
    uint8_t *buf = iwl->mtr_cpu + slot * 256;
    struct iwl_cmd_header *hdr = (struct iwl_cmd_header *)buf;
    hdr->cmd = id;
    hdr->group_id = group;
    hdr->sequence = iwl->cmd_seq++;
    hdr->length = (uint8_t)pay_len;
    if (pay_len)
        memcpy(buf + sizeof(*hdr), payload, pay_len);

    struct iwl_ax211_tfd *tfd = &iwl->tfd[slot];
    tfd->addr = iwl->mtr_dma + slot * 256;
    tfd->num_bytes = sizeof(*hdr) + pay_len;

    iwl->mtr_write = (iwl->mtr_write + 1) % IWL_MTR_SIZE;
    drain_mcr(iwl);
    return 0;
}
