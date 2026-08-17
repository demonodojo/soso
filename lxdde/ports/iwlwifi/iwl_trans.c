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

#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);
extern int strncmp(const char *a, const char *b, unsigned long n);

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
    if (!iwl->mmio || !iwl->alive)
        return;
    drain_mcr(iwl);
    (void)iwl_read32(iwl, CSR_INT);
}

int iwl_trans_send_cmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                       const void *payload, uint16_t pay_len)
{
    if (!iwl->alive || !iwl->mtr_cpu)
        return -1;
    if (pay_len + sizeof(struct iwl_cmd_header) > 240)
        return -1;

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
