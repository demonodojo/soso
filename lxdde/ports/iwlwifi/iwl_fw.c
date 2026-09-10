/* Parser TLV del firmware iwlwifi + PNVM. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);

static uint32_t le32(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) |
           ((uint32_t)p[3] << 24);
}

static int copy_section(struct iwl_fw_section *sec, const uint8_t *data, uint32_t len)
{
    if (!len)
        return 0;
    uint8_t *buf = lx_kmalloc(len, GFP_KERNEL);
    if (!buf)
        return -1;
    memcpy(buf, data, len);
    sec->data = buf;
    sec->len = len;
    return 0;
}

static int append_rt(struct iwl_fw_image *fw, const uint8_t *data, uint32_t len)
{
    if (fw->rt_n >= IWL_FW_RT_MAX)
        return -1;
    if (len < 4)
        return -1;
    uint8_t *buf = lx_kmalloc(len, GFP_KERNEL);
    if (!buf)
        return -1;
    memcpy(buf, data, len);
    fw->rt[fw->rt_n].data = buf;
    fw->rt[fw->rt_n].len = len;
    fw->rt_n++;
    return 0;
}

static int is_separator(const struct iwl_fw_rt_section *sec, uint32_t magic)
{
    return sec->len >= 4 && le32(sec->data) == magic;
}

int iwl_fw_parse_tlv(struct iwl_ax211_priv *iwl, const uint8_t *fw, unsigned long fw_len)
{
    if (!fw || fw_len < sizeof(struct iwl_tlv_ucode_header))
        return -1;

    memset(&iwl->fw, 0, sizeof(iwl->fw));
    memset(iwl->fw_capa, 0, sizeof(iwl->fw_capa));

    const struct iwl_tlv_ucode_header *hdr = (const struct iwl_tlv_ucode_header *)fw;
    if (le32((const uint8_t *)&hdr->magic) != IWL_TLV_UCODE_MAGIC) {
        lx_printk("iwl_fw: magic TLV invalido\n");
        return -1;
    }

    const uint8_t *pos = hdr->data;
    const uint8_t *end = fw + fw_len;

    while (pos + 8 <= end) {
        uint32_t type = le32(pos);
        uint32_t length = le32(pos + 4);
        pos += 8;
        if (length > (unsigned long)(end - pos))
            return -1;
        switch (type) {
        case IWL_UCODE_TLV_INST:
            if (copy_section(&iwl->fw.inst, pos, length))
                return -1;
            break;
        case IWL_UCODE_TLV_DATA:
            if (copy_section(&iwl->fw.data, pos, length))
                return -1;
            break;
        case IWL_UCODE_TLV_INIT:
            if (copy_section(&iwl->fw.init, pos, length))
                return -1;
            break;
        case IWL_UCODE_TLV_INIT_DATA:
            if (copy_section(&iwl->fw.init_data, pos, length))
                return -1;
            break;
        case IWL_UCODE_TLV_BOOT:
            if (copy_section(&iwl->fw.boot, pos, length))
                return -1;
            break;
        case IWL_UCODE_TLV_SEC_RT:
            if (append_rt(&iwl->fw, pos, length))
                return -1;
            break;
        case IWL_UCODE_TLV_IML: {
            uint8_t *buf = lx_kmalloc(length, GFP_KERNEL);
            if (!buf)
                return -1;
            memcpy(buf, pos, length);
            iwl->iml = buf;
            iwl->iml_len = length;
            lx_printk("iwl_fw: IML %u bytes\n", length);
            break;
        }
        case IWL_UCODE_TLV_PHY_SKU:
            if (length >= 4) {
                iwl->phy_sku = le32(pos);
                iwl->fw_valid_tx_ant =
                    (uint8_t)((iwl->phy_sku & FW_PHY_CFG_TX_CHAIN) >>
                              FW_PHY_CFG_TX_CHAIN_POS);
                iwl->fw_valid_rx_ant =
                    (uint8_t)((iwl->phy_sku & FW_PHY_CFG_RX_CHAIN) >>
                              FW_PHY_CFG_RX_CHAIN_POS);
                lx_printk("iwl_fw: PHY_SKU 0x%08x tx=0x%x rx=0x%x\n",
                          iwl->phy_sku, (unsigned)iwl->fw_valid_tx_ant,
                          (unsigned)iwl->fw_valid_rx_ant);
            }
            break;
        case IWL_UCODE_TLV_N_SCAN:
            if (length >= 1) {
                iwl->n_scan_channels = pos[0];
                lx_printk("iwl_fw: N_SCAN_CHANNELS %u\n",
                          (unsigned)iwl->n_scan_channels);
            }
            break;
        case IWL_UCODE_TLV_CMD_VERSIONS: {
            unsigned n = length / sizeof(struct iwl_fw_cmd_version);
            unsigned i;

            iwl->cmd_ver_count = 0;
            for (i = 0; i < n && iwl->cmd_ver_count < 64u; i++) {
                const struct iwl_fw_cmd_version *cv =
                    (const struct iwl_fw_cmd_version *)(pos +
                                                        i * sizeof(*cv));
                iwl->cmd_ver[iwl->cmd_ver_count++] = *cv;
            }
            lx_printk("iwl_fw: CMD_VERSIONS %u entradas\n", iwl->cmd_ver_count);
            break;
        }
        case IWL_UCODE_TLV_ENABLED_CAPABILITIES: {
            if (length == sizeof(struct iwl_ucode_capa)) {
                const struct iwl_ucode_capa *uc =
                    (const struct iwl_ucode_capa *)pos;
                uint32_t idx = le32((const uint8_t *)&uc->api_index);
                uint32_t flags = le32((const uint8_t *)&uc->api_capa);

                if (idx < IWL_FW_CAPA_SETS)
                    iwl->fw_capa[idx] |= flags;
            }
            break;
        }
        default:
            break;
        }
        {
            unsigned long padded = ((unsigned long)length + 3ul) & ~3ul;
            if (padded > (unsigned long)(end - pos))
                return -1;
            pos += padded;
        }
    }
    if (pos != end)
        return -1;

    if (iwl->fw.rt_n > 0) {
        lx_printk("iwl_fw: SEC_RT %d secciones\n", iwl->fw.rt_n);
        return 0;
    }
    if (!iwl->fw.inst.len || !iwl->fw.data.len) {
        lx_printk("iwl_fw: secciones INST/DATA ausentes\n");
        return -1;
    }
    lx_printk("iwl_fw: inst=%u data=%u init=%u\n",
              iwl->fw.inst.len, iwl->fw.data.len, iwl->fw.init.len);
    return 0;
}

int iwl_fw_has_capa(const struct iwl_ax211_priv *iwl, unsigned capa_bit)
{
    unsigned set = capa_bit / 32u;
    unsigned bit = capa_bit % 32u;

    if (!iwl || set >= IWL_FW_CAPA_SETS)
        return 0;
    return (iwl->fw_capa[set] & (1u << bit)) != 0;
}

/* `iwl_fw_lookup_notif_ver`: versión del *layout de notificación/respuesta*. */
int iwl_fw_notif_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    unsigned i;
    uint8_t grp = group;

    if (group == LEGACY_GROUP)
        grp = LONG_GROUP;
    for (i = 0; i < iwl->cmd_ver_count; i++) {
        if (iwl->cmd_ver[i].group == grp && iwl->cmd_ver[i].cmd == cmd) {
            return iwl->cmd_ver[i].notif_version;
        }
    }
    return 0;
}

int iwl_fw_cmd_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    unsigned i;
    /* Linux `iwl_fw_lookup_cmd_ver` (fw/img.c): grp 0 → LONG_GROUP (DEF_ID). */
    uint8_t grp = group;

    if (group == LEGACY_GROUP)
        grp = LONG_GROUP;
    for (i = 0; i < iwl->cmd_ver_count; i++) {
        if (iwl->cmd_ver[i].group == grp && iwl->cmd_ver[i].cmd == cmd) {
            return iwl->cmd_ver[i].version;
        }
    }
    return 0;
}

int iwl_fw_parse_pnvm(struct iwl_ax211_priv *iwl, const uint8_t *pnvm, unsigned long pnvm_len)
{
    const uint8_t *pos;
    const uint8_t *end;
    int collecting = 0;
    uint8_t *acc = 0;
    unsigned long acc_len = 0;

    if (!pnvm || pnvm_len < 8)
        return 0;
    pos = pnvm;
    end = pnvm + pnvm_len;
    while (pos + 8 <= end) {
        uint32_t type = le32(pos);
        uint32_t length = le32(pos + 4);
        pos += 8;
        if (pos + length > end)
            break;
        if (type == IWL_UCODE_TLV_PNVM_SKU) {
            if (collecting && acc_len)
                break;
            collecting = 1;
        } else if (collecting && type == IWL_UCODE_TLV_SEC_RT && length > 4) {
            const uint8_t *payload = pos + 4;
            uint32_t plen = length - 4;
            uint8_t *nbuf = lx_kmalloc(acc_len + plen, GFP_KERNEL);
            if (!nbuf)
                return -1;
            if (acc && acc_len)
                memcpy(nbuf, acc, acc_len);
            memcpy(nbuf + acc_len, payload, plen);
            if (acc)
                lx_kfree(acc);
            acc = nbuf;
            acc_len += plen;
        }
        pos += (length + 3) & ~3u;
    }
    if (acc && acc_len) {
        iwl->pnvm_data = acc;
        iwl->pnvm_len = acc_len;
        lx_printk("iwl_fw: pnvm %lu bytes\n", acc_len);
    }
    return 0;
}

static int dram_push(uint64_t *map, int idx, const uint8_t *data, uint32_t len)
{
    if (!len || idx >= IWL_MAX_DRAM_ENTRY)
        return -1;
    uint64_t dma;
    void *cpu = lx_dma_alloc_coherent(0, len, &dma, GFP_KERNEL);
    if (!cpu)
        return -1;
    memcpy(cpu, data, len);
    map[idx] = dma;
    return idx + 1;
}

static int upload_rt(struct iwl_fw_image *fw, struct iwl_context_info_dram *dram)
{
    enum { PHASE_LMAC, PHASE_UMAC, PHASE_PAGING } phase = PHASE_LMAC;
    int lmac = 0, umac = 0, paging = 0;
    int i;

    for (i = 0; i < fw->rt_n; i++) {
        const struct iwl_fw_rt_section *sec = &fw->rt[i];
        if (is_separator(sec, IWL_CPU1_CPU2_SEPARATOR)) {
            phase = PHASE_UMAC;
            continue;
        }
        if (is_separator(sec, IWL_PAGING_SEPARATOR)) {
            phase = PHASE_PAGING;
            continue;
        }
        /* iwl_store_ucode_sec: el primer u32 es el offset SRAM, no ucode.
         * ctxt-info apunta al payload igual que iwl_pcie_init_fw_sec. Copiar
         * el offset al DMA desplazaba cuatro bytes cada sección firmada. */
        if (sec->len <= 4)
            return -1;
        switch (phase) {
        case PHASE_LMAC:
            lmac = dram_push(dram->lmac_img, lmac, sec->data + 4, sec->len - 4);
            if (lmac < 0)
                return -1;
            break;
        case PHASE_UMAC:
            umac = dram_push(dram->umac_img, umac, sec->data + 4, sec->len - 4);
            if (umac < 0)
                return -1;
            break;
        case PHASE_PAGING:
            paging = dram_push(dram->virtual_img, paging, sec->data + 4, sec->len - 4);
            if (paging < 0)
                return -1;
            break;
        }
    }
    if (!lmac || !umac) {
        lx_printk("iwl_fw: SEC_RT sin lmac/umac (l=%d u=%d p=%d)\n", lmac, umac, paging);
        return -1;
    }
    lx_printk("iwl_fw: dram lmac=%d umac=%d paging=%d\n", lmac, umac, paging);
    return 0;
}

static int fill_dram_map(uint64_t *map, const struct iwl_fw_section *sec)
{
    if (!sec->len)
        return 0;
    return dram_push(map, 0, sec->data, sec->len);
}

int iwl_fw_upload_sections(struct iwl_ax211_priv *iwl, struct iwl_context_info_dram *dram)
{
    memset(dram, 0, sizeof(*dram));
    if (iwl->fw.rt_n > 0)
        return upload_rt(&iwl->fw, dram);
    if (fill_dram_map(dram->umac_img, &iwl->fw.inst) < 0)
        return -1;
    if (fill_dram_map(dram->lmac_img, &iwl->fw.data) < 0)
        return -1;
    if (iwl->fw.init.len &&
        fill_dram_map(dram->virtual_img, &iwl->fw.init) < 0)
        return -1;
    return 0;
}
