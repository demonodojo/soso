/* FWSEC-FRTS: extrae el ucode de la VBIOS (BIT 'p'), parchea FRTS y lo ejecuta en
 * el falcon GSP antes del booter_load de SEC2 (`tu102_gsp_oneinit`). */
#include "gsp_fwsec.h"
#include "falcon_lx.h"
#include "gsp_dma.h"
#include "gsp_mmio.h"
#include "lx_emul.h"

void *memcpy(void *dst, const void *src, unsigned long n);
void *memset(void *dst, int c, unsigned long n);

#define NV_VBIOS_BASE           0x00110000u
#define NV_VBIOS_SIZE           0x20000u

#define NV_PFB_PRI_MMU_WPR2_ADDR_LO 0x001fa824u
#define NV_PFB_PRI_MMU_WPR2_ADDR_HI 0x001fa828u
#define NV_PBUS_SW_SCRATCH_0E   0x00134014u

#define NVFW_FALCON_APPIF_ID_DMEMMAPPER 0x4u
#define NVFW_FALCON_APPIF_DMEMMAPPER_CMD_FRTS 0x15u
#define NVFW_FRTS_CMD_REGION_TYPE_FB 2u

struct falcon_ucode_desc_v2 {
    uint32_t version;
    uint32_t bootloader_offset;
    uint32_t bootloader_size;
    uint32_t bootloader_param_offset;
    uint32_t bootloader_param_size;
    uint32_t imem_offset;
    uint32_t imem_size;
    uint32_t imem_load_size;
    uint32_t dmem_offset;
    uint32_t dmem_size;
    uint32_t dmem_load_size;
    uint32_t interface_offset;
    uint32_t interface_size;
    uint32_t imem_phys_base;
    uint32_t dmem_phys_base;
    uint32_t engine_id_mask;
    uint32_t ucode_id;
    uint32_t signature_count;
    uint32_t signature_versions;
    uint32_t pkc_data_offset;
    uint32_t pkc_data_size;
};

struct falcon_appif_hdr_v1 {
    uint8_t version;
    uint8_t header_size;
    uint8_t entry_size;
    uint8_t entry_count;
};

struct falcon_appif_v1 {
    uint32_t id;
    uint32_t dmem_base;
};

struct falcon_appif_dmemmapper_v3 {
    uint32_t signature;
    uint16_t version;
    uint16_t size;
    uint32_t cmd_in_buffer_offset;
    uint32_t cmd_in_buffer_size;
    uint32_t cmd_out_buffer_offset;
    uint32_t cmd_out_buffer_size;
    uint32_t nvf_img_data_buffer_offset;
    uint32_t nvf_img_data_buffer_size;
    uint32_t printf_buffer_hdr;
    uint32_t ucode_build_time_stamp;
    uint32_t ucode_signature;
    uint32_t init_cmd;
    uint32_t ucode_feature;
    uint32_t ucode_cmd_mask0;
    uint32_t ucode_cmd_mask1;
    uint32_t multi_tgt_tbl;
};

struct read_vbios_cmd {
    uint32_t ver;
    uint32_t hdr;
    uint64_t addr;
    uint32_t size;
    uint32_t flags;
};

struct frts_region_cmd {
    uint32_t ver;
    uint32_t hdr;
    uint32_t addr;
    uint32_t size;
    uint32_t ftype;
};

struct frts_cmd {
    struct read_vbios_cmd read_vbios;
    struct frts_region_cmd frts_region;
};

static uint8_t vbios_rd8(unsigned off)
{
    unsigned word = off & ~3u;
    uint32_t v = gsp_mmio_rd32(NV_VBIOS_BASE + word);
    return (uint8_t)(v >> ((off & 3u) * 8u));
}

static int vbios_match(unsigned off, const char *sig, unsigned n)
{
    unsigned i;
    for (i = 0; i < n; i++) {
        if (vbios_rd8(off + i) != (uint8_t)sig[i]) {
            return 0;
        }
    }
    return 1;
}

static int vbios_find_bit_token(unsigned token, unsigned *data_off, unsigned *data_len)
{
    unsigned i;
    for (i = 0; i + 12u < NV_VBIOS_SIZE; i++) {
        if (!vbios_match(i, "BIT", 3)) {
            continue;
        }
        unsigned hdr = i;
        unsigned pos = hdr + vbios_rd8(hdr + 4) + vbios_rd8(hdr + 5);
        unsigned n = vbios_rd8(hdr + 6);
        unsigned j;
        for (j = 0; j < n; j++) {
            if (pos + 3u >= NV_VBIOS_SIZE) {
                break;
            }
            if (vbios_rd8(pos) != token) {
                pos += 3u;
                pos += vbios_rd8(pos + 1) | ((unsigned)vbios_rd8(pos + 2) << 8);
                continue;
            }
            unsigned len = vbios_rd8(pos + 1) | ((unsigned)vbios_rd8(pos + 2) << 8);
            *data_off = pos + 3u;
            *data_len = len;
            return 0;
        }
    }
    return -1;
}

static int vbios_copy(unsigned off, void *dst, unsigned len)
{
    unsigned char *d = dst;
    unsigned i;
    for (i = 0; i < len; i++) {
        d[i] = vbios_rd8(off + i);
    }
    return 0;
}

static uint64_t wpr2_hi_bound(void)
{
    uint32_t hi = gsp_mmio_rd32(NV_PFB_PRI_MMU_WPR2_ADDR_HI);
    return ((uint64_t)(hi >> 4) & 0x0fffffffull) << 12;
}

static uint64_t wpr2_lo_bound(void)
{
    uint32_t lo = gsp_mmio_rd32(NV_PFB_PRI_MMU_WPR2_ADDR_LO);
    return ((uint64_t)(lo >> 4) & 0x0fffffffull) << 12;
}

int gsp_fwsec_wpr2_present(uint64_t *lo_out, uint64_t *hi_out)
{
    uint64_t hi = wpr2_hi_bound();
    if (hi == 0) {
        return 0;
    }
    if (lo_out) {
        *lo_out = wpr2_lo_bound();
    }
    if (hi_out) {
        *hi_out = hi;
    }
    return 1;
}

static int patch_fwsec_frts(unsigned char *ucode, unsigned ulen,
                            const struct falcon_ucode_desc_v2 *desc,
                            uint64_t frts_addr, uint64_t frts_size)
{
    unsigned hdr_off;
    struct falcon_appif_hdr_v1 hdr;
    unsigned i;

    if (desc->imem_load_size + desc->interface_offset + sizeof(hdr) > ulen) {
        return -1;
    }
    hdr_off = desc->imem_load_size + desc->interface_offset;
    memcpy(&hdr, ucode + hdr_off, sizeof(hdr));
    if (hdr.version != 1u) {
        return -1;
    }

    for (i = 0; i < hdr.entry_count; i++) {
        unsigned eoff = hdr_off + hdr.header_size + i * hdr.entry_size;
        struct falcon_appif_v1 app;
        struct falcon_appif_dmemmapper_v3 *map;
        struct frts_cmd *cmd;

        if (eoff + sizeof(app) > ulen) {
            return -1;
        }
        memcpy(&app, ucode + eoff, sizeof(app));
        if (app.id != NVFW_FALCON_APPIF_ID_DMEMMAPPER) {
            continue;
        }

        if (desc->imem_load_size + app.dmem_base + sizeof(*map) > ulen) {
            return -1;
        }
        map = (struct falcon_appif_dmemmapper_v3 *)(ucode + desc->imem_load_size + app.dmem_base);
        map->init_cmd = NVFW_FALCON_APPIF_DMEMMAPPER_CMD_FRTS;

        if (desc->imem_load_size + map->cmd_in_buffer_offset + sizeof(*cmd) > ulen) {
            return -1;
        }
        cmd = (struct frts_cmd *)(ucode + desc->imem_load_size + map->cmd_in_buffer_offset);
        cmd->read_vbios.ver = 1;
        cmd->read_vbios.hdr = (uint32_t)sizeof(cmd->read_vbios);
        cmd->read_vbios.addr = 0;
        cmd->read_vbios.size = 0;
        cmd->read_vbios.flags = 2;
        cmd->frts_region.ver = 1;
        cmd->frts_region.hdr = (uint32_t)sizeof(cmd->frts_region);
        cmd->frts_region.addr = (uint32_t)(frts_addr >> 12);
        cmd->frts_region.size = (uint32_t)(frts_size >> 12);
        cmd->frts_region.ftype = NVFW_FRTS_CMD_REGION_TYPE_FB;
        return 0;
    }
    return -1;
}

int gsp_fwsec_run_frts(uint64_t frts_addr, uint64_t frts_size)
{
    unsigned data_off, data_len;
    struct falcon_ucode_desc_v2 desc;
    unsigned char *ucode;
    unsigned ulen;
    struct gsp_dma_buf dma;
    uint32_t err;

    if (!gsp_mmio_alive()) {
        return -1;
    }

    if (gsp_fwsec_wpr2_present(NULL, NULL)) {
        lx_printk("nouveau-lx: WPR2 ya presente (0x%llx-0x%llx) — omito FWSEC-FRTS\n",
                  (unsigned long long)wpr2_lo_bound(),
                  (unsigned long long)wpr2_hi_bound());
        return 0;
    }

    if (vbios_find_bit_token('p', &data_off, &data_len) != 0 || data_len < sizeof(desc)) {
        lx_printk("nouveau-lx: VBIOS sin partición FWSEC (BIT 'p')\n");
        return -1;
    }
    vbios_copy(data_off, &desc, sizeof(desc));
    if (desc.version != 2u && desc.version != 3u) {
        lx_printk("nouveau-lx: FWSEC desc v%u no soportada\n", desc.version);
        return -1;
    }

    ulen = desc.imem_load_size + desc.dmem_load_size;
    if (!ulen || data_off + sizeof(desc) + ulen > NV_VBIOS_SIZE) {
        lx_printk("nouveau-lx: FWSEC ucode fuera de VBIOS\n");
        return -1;
    }

    ucode = lx_kmalloc(ulen, GFP_KERNEL);
    if (!ucode) {
        return -1;
    }
    vbios_copy(data_off + sizeof(desc), ucode, ulen);
    if (patch_fwsec_frts(ucode, ulen, &desc, frts_addr, frts_size) != 0) {
        lx_printk("nouveau-lx: FWSEC-FRTS parche DMEMMAPPER falló\n");
        lx_kfree(ucode);
        return -1;
    }

    if (gsp_dma_alloc_copy(&dma, ucode, ulen, "fwsec-frts") != 0) {
        lx_kfree(ucode);
        return -1;
    }
    lx_kfree(ucode);
    ucode = dma.va;
    ulen = (unsigned)dma.size;

    lx_printk("nouveau-lx: FWSEC-FRTS frts=0x%llx size=0x%llx imem=%u dmem=%u\n",
              (unsigned long long)frts_addr, (unsigned long long)frts_size,
              desc.imem_load_size, desc.dmem_load_size);

    if (falcon_lx_vbios_boot(LX_FLCN_GSP_BASE, ucode, ulen,
                             desc.imem_offset, desc.imem_load_size,
                             desc.dmem_offset, desc.dmem_load_size,
                             desc.imem_offset, (unsigned)dma.phys, "fwsec-frts") != 0) {
        gsp_dma_free(&dma);
        return -1;
    }
    gsp_dma_free(&dma);

    err = gsp_mmio_rd32(NV_PBUS_SW_SCRATCH_0E) & 0xffu;
    if (err) {
        lx_printk("nouveau-lx: FWSEC-FRTS error scratch=0x%02x\n", err);
        return -1;
    }
    if (!gsp_fwsec_wpr2_present(NULL, NULL)) {
        lx_printk("nouveau-lx: FWSEC-FRTS terminó pero WPR2 sigue vacío\n");
        return -1;
    }
    lx_printk("nouveau-lx: FWSEC-FRTS OK — WPR2 0x%llx-0x%llx\n",
              (unsigned long long)wpr2_lo_bound(),
              (unsigned long long)wpr2_hi_bound());
    return 0;
}
