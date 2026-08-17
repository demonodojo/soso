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

static uint64_t le64(const uint8_t *p)
{
    return (uint64_t)le32(p) | ((uint64_t)le32(p + 4) << 32);
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

int iwl_fw_parse_tlv(struct iwl_ax211_priv *iwl, const uint8_t *fw, unsigned long fw_len)
{
    if (!fw || fw_len < sizeof(struct iwl_tlv_ucode_header))
        return -1;

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
        if (pos + length > end)
            break;
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
        default:
            break;
        }
        pos += (length + 3) & ~3u;
    }

    if (!iwl->fw.inst.len || !iwl->fw.data.len) {
        lx_printk("iwl_fw: secciones INST/DATA ausentes\n");
        return -1;
    }
    lx_printk("iwl_fw: inst=%u data=%u init=%u\n",
              iwl->fw.inst.len, iwl->fw.data.len, iwl->fw.init.len);
    return 0;
}

int iwl_fw_parse_pnvm(struct iwl_ax211_priv *iwl, const uint8_t *pnvm, unsigned long pnvm_len)
{
    if (!pnvm || pnvm_len < 8)
        return 0;
    const uint8_t *pos = pnvm;
    const uint8_t *end = pnvm + pnvm_len;
    while (pos + 8 <= end) {
        uint32_t type = le32(pos);
        uint32_t length = le32(pos + 4);
        pos += 8;
        if (pos + length > end)
            break;
        if (type == IWL_UCODE_TLV_PNVM_SKU || type == 64) {
            uint8_t *buf = lx_kmalloc(length, GFP_KERNEL);
            if (!buf)
                return -1;
            memcpy(buf, pos, length);
            iwl->pnvm_data = buf;
            iwl->pnvm_len = length;
            lx_printk("iwl_fw: pnvm %u bytes\n", length);
            break;
        }
        pos += (length + 3) & ~3u;
    }
    return 0;
}

static int fill_dram_map(uint64_t *map, const struct iwl_fw_section *sec)
{
    if (!sec->len)
        return 0;
    uint64_t dma;
    void *cpu = lx_dma_alloc_coherent(0, sec->len, &dma, GFP_KERNEL);
    if (!cpu)
        return -1;
    memcpy(cpu, sec->data, sec->len);
    unsigned off = 0;
    int idx = 0;
    while (off < sec->len && idx < IWL_MAX_DRAM_ENTRY) {
        unsigned chunk = sec->len - off;
        if (chunk > 32768)
            chunk = 32768;
        map[idx++] = dma + off;
        off += chunk;
    }
    return idx;
}

int iwl_fw_upload_sections(struct iwl_ax211_priv *iwl, struct iwl_context_info_dram *dram)
{
    memset(dram, 0, sizeof(*dram));
    if (fill_dram_map(dram->umac_img, &iwl->fw.inst) < 0)
        return -1;
    if (fill_dram_map(dram->lmac_img, &iwl->fw.data) < 0)
        return -1;
    if (iwl->fw.init.len &&
        fill_dram_map(dram->virtual_img, &iwl->fw.init) < 0)
        return -1;
    return 0;
}
