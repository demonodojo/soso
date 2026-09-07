/* G3 ola 2: falcon MMIO + DMA ga102 (HS v2 ucode, sin WPR nvkm). */
#include "falcon_lx.h"
#include "gsp_mmio.h"
#include "gsp_top.h"
#include "nvfw_lx.h"
#include "lx_emul.h"

void *memset(void *dst, int c, unsigned long n);

#define FLCN_IMEM 0
#define FLCN_DMEM 1

struct flcn_fw_ctx {
    const unsigned char *img;
    unsigned dma_handle;
    unsigned imem_base_img;
    unsigned imem_base;
    unsigned imem_size;
    unsigned dmem_base_img;
    unsigned dmem_base;
    unsigned dmem_size;
    unsigned dmem_sign;
    unsigned boot_addr;
    unsigned engine_id;
    unsigned ucode_id;
    unsigned fuse_ver;
};

static unsigned flcn_rd32(unsigned base, unsigned off)
{
    return gsp_mmio_rd32(base + off);
}

static void flcn_wr32(unsigned base, unsigned off, unsigned val)
{
    gsp_mmio_wr32(base + off, val);
}

static void flcn_mask(unsigned base, unsigned off, unsigned mask, unsigned val)
{
    unsigned cur = flcn_rd32(base, off);
    flcn_wr32(base, off, (cur & ~mask) | (val & mask));
}

static int flcn_select(unsigned base)
{
    unsigned v = flcn_rd32(base, LX_FLCN_ADDR2 + 0x668u);
    if ((v & 0x10u) != 0u) {
        flcn_wr32(base, LX_FLCN_ADDR2 + 0x668u, 0u);
        {
            unsigned t = 10u;
            while (t--) {
                if (flcn_rd32(base, LX_FLCN_ADDR2 + 0x668u) & 1u) {
                    return 0;
                }
                lx_mdelay(1);
            }
        }
        return -1;
    }
    return 0;
}

/* Reset del falcon antes de cada carga (`nvkm_falcon_reset` / ga102 HAL). */
int falcon_lx_reset(unsigned base)
{
    unsigned t;
    unsigned hwcfg2;

    hwcfg2 = flcn_rd32(base, 0x10cu);
    (void)hwcfg2;

    t = 150u;
    while (t--) {
        if (flcn_rd32(base, 0x10cu) & 0x1u) {
            break;
        }
        lx_udelay(1);
    }

    flcn_mask(base, 0x3c0u, 0x1u, 0x1u);
    lx_udelay(10);
    flcn_mask(base, 0x3c0u, 0x1u, 0x0u);

    t = 2000u;
    while (t--) {
        hwcfg2 = flcn_rd32(base, 0x10cu);
        if ((hwcfg2 & 0x6u) == 0u) {
            break;
        }
        lx_mdelay(1);
    }
    if ((hwcfg2 & 0x6u) != 0u) {
        return -1;
    }

    flcn_wr32(base, 0x10cu, 0u);
    return flcn_select(base);
}

static int flcn_reset_wait_mem_scrubbing(unsigned base)
{
    unsigned t;

    flcn_wr32(base, 0x040u, 0u);
    t = 20u;
    while (t--) {
        if (!(flcn_rd32(base, 0x0f4u) & 0x1000u)) {
            return 0;
        }
        lx_mdelay(1);
    }
    return -1;
}

int falcon_lx_enable(unsigned falcon_base, uint8_t top_type, uint8_t top_inst)
{
    uint32_t pmc_mask = 0;

    if (gsp_top_pmc_enable_mask(top_type, top_inst, &pmc_mask) == 0) {
        lx_printk("nouveau-lx: falcon enable PMC mask=0x%x enable=0x%08x\n",
                  pmc_mask, gsp_mmio_rd32(0x000600u));
        gsp_mc_device_enable(pmc_mask);
    }
    if (flcn_reset_wait_mem_scrubbing(falcon_base) != 0) {
        lx_printk("nouveau-lx: falcon enable mem scrub timeout\n");
        return -1;
    }
    flcn_wr32(falcon_base, 0x084u, gsp_mmio_rd32(0u));
    return 0;
}

static int flcn_dma_done(unsigned base)
{
    return (flcn_rd32(base, 0x118u) & 2u) ? 1 : 0;
}

static int flcn_dma_wr(unsigned base, const unsigned char *img, unsigned dma_handle,
                       unsigned dma_base, int mem_type, unsigned mem_base, unsigned len, int sec)
{
    unsigned cmd;
    unsigned dst, src;
    const unsigned dmalen = 256u;
    unsigned dma_addr = dma_handle;

    if (len == 0u || (len & (dmalen - 1u)) != 0u) {
        return -1;
    }

    cmd = (6u << 8u);
    if (mem_type == FLCN_IMEM) {
        cmd |= 0x10u;
    }
    if (sec) {
        cmd |= 4u;
    }

    if (mem_type == FLCN_DMEM) {
        dma_addr += dma_base;
    }

    flcn_wr32(base, 0x110u, dma_addr >> 8);
    flcn_wr32(base, 0x128u, 0u);

    dst = mem_base;
    src = dma_base;
    while (len >= dmalen) {
        flcn_wr32(base, 0x114u, dst);
        flcn_wr32(base, 0x11cu, src - (mem_type == FLCN_DMEM ? dma_base : 0u));
        flcn_wr32(base, 0x118u, cmd);

        {
            unsigned t = 2000u;
            while (t--) {
                if (flcn_dma_done(base)) {
                    break;
                }
                lx_mdelay(1);
            }
            if (t == 0u) {
                return -1;
            }
        }

        (void)img;
        src += dmalen;
        dst += dmalen;
        len -= dmalen;
    }
    return 0;
}

static int flcn_parse_hs_v2(const struct acr_fw_blob *blob, struct flcn_fw_ctx *fw)
{
    const struct nvfw_bin_hdr *hdr;
    const struct nvfw_hs_header_v2 *hshdr;
    const struct nvfw_hs_load_header_v2 *lhdr;
    const unsigned char *data;
    const unsigned *meta;

    if (!blob || !blob->data || blob->len < 128u) {
        return -1;
    }
    data = blob->data;
    hdr = (const struct nvfw_bin_hdr *)data;
    if (hdr->bin_magic != NVFW_BIN_MAGIC) {
        return -1;
    }
    hshdr = (const struct nvfw_hs_header_v2 *)(data + hdr->header_offset);
    meta = (const unsigned *)(data + hshdr->meta_data_offset);
    lhdr = (const struct nvfw_hs_load_header_v2 *)(data + hshdr->header_offset);

    fw->img = data + hdr->data_offset;
    fw->dma_handle = blob->dma_handle;
    fw->imem_base_img = lhdr->app[0].offset;
    fw->imem_base = 0u;
    fw->imem_size = lhdr->app[0].size;
    fw->dmem_base_img = lhdr->os_data_offset;
    fw->dmem_base = 0u;
    fw->dmem_size = lhdr->os_data_size;
    fw->dmem_sign = *(const unsigned *)(data + hshdr->patch_loc) - lhdr->os_data_offset;
    fw->boot_addr = lhdr->app[0].offset;
    fw->fuse_ver = meta[0];
    fw->engine_id = meta[1];
    fw->ucode_id = meta[2];
    return 0;
}

static int flcn_fw_load(unsigned base, struct flcn_fw_ctx *fw)
{
    flcn_mask(base, 0x624u, 0x80u, 0x80u);
    flcn_wr32(base, 0x10cu, 0u);
    flcn_mask(base, 0x600u, 0x00010007u, (0u << 16) | (1u << 2) | 1u);

    if (flcn_dma_wr(base, fw->img, fw->dma_handle, fw->imem_base_img, FLCN_IMEM,
                    fw->imem_base, fw->imem_size, 1) != 0) {
        return -1;
    }
    if (flcn_dma_wr(base, fw->img, fw->dma_handle, fw->dmem_base_img, FLCN_DMEM,
                    fw->dmem_base, fw->dmem_size, 0) != 0) {
        return -1;
    }
    return 0;
}

static int flcn_fw_boot_ga102(unsigned base, struct flcn_fw_ctx *fw,
                              unsigned mbox0_in, unsigned mbox1_in,
                              int check_mbox0, unsigned mbox0_ok, unsigned timeout_ms)
{
    unsigned mbox0, mbox1;
    unsigned t;

    flcn_wr32(base, LX_FLCN_ADDR2 + 0x210u, fw->dmem_sign);
    flcn_wr32(base, LX_FLCN_ADDR2 + 0x19cu, fw->engine_id);
    flcn_wr32(base, LX_FLCN_ADDR2 + 0x198u, fw->ucode_id);
    flcn_wr32(base, LX_FLCN_ADDR2 + 0x180u, 1u);

    flcn_wr32(base, 0x040u, mbox0_in);
    flcn_wr32(base, 0x044u, mbox1_in);
    flcn_wr32(base, 0x104u, fw->boot_addr);
    flcn_wr32(base, 0x100u, 2u);

    t = timeout_ms ? timeout_ms : 2000u;
    while (t--) {
        if (flcn_rd32(base, 0x100u) & 0x10u) {
            break;
        }
        lx_mdelay(1);
    }
    if (t == 0u) {
        return -1;
    }

    mbox0 = flcn_rd32(base, 0x040u);
    mbox1 = flcn_rd32(base, 0x044u);
    lx_printk("nouveau-lx: falcon %s mbox0=0x%x mbox1=0x%x (expect 0x%x)\n",
              "boot", mbox0, mbox1, mbox0_ok);
    if (check_mbox0 && mbox0 != mbox0_ok) {
        return -1;
    }
    return 0;
}

int falcon_lx_hsfw_boot_mbox(unsigned falcon_base, const struct acr_fw_blob *blob,
                             const char *name, unsigned mbox0, unsigned mbox1,
                             int check_mbox0)
{
    struct flcn_fw_ctx fw;

    if (flcn_parse_hs_v2(blob, &fw) != 0) {
        lx_printk("nouveau-lx: falcon %s parse HS v2 falló\n", name);
        return -1;
    }

    if (falcon_lx_reset(falcon_base) != 0) {
        lx_printk("nouveau-lx: falcon %s reset falló\n", name);
        return -1;
    }

    if (falcon_lx_enable(falcon_base, GSP_TOP_TYPE_SEC2, 0) != 0) {
        lx_printk("nouveau-lx: falcon %s enable falló\n", name);
        return -1;
    }

    lx_printk("nouveau-lx: falcon %s load imem=%u dmem=%u engine=%u ucode=%u\n",
              name, fw.imem_size, fw.dmem_size, fw.engine_id, fw.ucode_id);

    if (flcn_fw_load(falcon_base, &fw) != 0) {
        lx_printk("nouveau-lx: falcon %s DMA load falló\n", name);
        return -1;
    }

    if (flcn_fw_boot_ga102(falcon_base, &fw, mbox0, mbox1, check_mbox0, 0u,
                          check_mbox0 ? 2000u : 4000u) != 0) {
        lx_printk("nouveau-lx: falcon %s boot falló\n", name);
        return -1;
    }

    lx_printk("nouveau-lx: falcon %s boot ok\n", name);
    return 0;
}

int falcon_lx_hsfw_boot(unsigned falcon_base, const struct acr_fw_blob *blob, const char *name)
{
    return falcon_lx_hsfw_boot_mbox(falcon_base, blob, name, 0xcafebeefu, 0u, 1);
}

int falcon_lx_raw_boot(unsigned falcon_base, const struct falcon_lx_raw *raw)
{
    struct flcn_fw_ctx fw;
    const char *name;

    if (!raw || !raw->img || !raw->imem_len || !raw->dmem_len || !raw->dma_handle) {
        return -1;
    }
    name = raw->name ? raw->name : "raw";

    memset(&fw, 0, sizeof(fw));
    fw.img = raw->img;
    fw.dma_handle = raw->dma_handle;
    fw.imem_base_img = raw->imem_src;
    fw.imem_base = raw->imem_dst;
    fw.imem_size = raw->imem_len;
    fw.dmem_base_img = raw->dmem_src;
    fw.dmem_base = raw->dmem_dst;
    fw.dmem_size = raw->dmem_len;
    fw.dmem_sign = raw->pkc_data_offset;
    fw.engine_id = raw->engine_id_mask;
    fw.ucode_id = raw->ucode_id;
    fw.boot_addr = raw->boot_addr;

    if (falcon_lx_reset(falcon_base) != 0) {
        lx_printk("nouveau-lx: falcon %s reset falló\n", name);
        return -1;
    }

    if (falcon_lx_enable(falcon_base, GSP_TOP_TYPE_GSP, 0) != 0) {
        lx_printk("nouveau-lx: falcon %s enable falló\n", name);
        return -1;
    }

    lx_printk("nouveau-lx: falcon %s raw imem=%u dmem=%u engine=0x%x ucode=%u boot=0x%x\n",
              name, fw.imem_size, fw.dmem_size, fw.engine_id, fw.ucode_id, fw.boot_addr);

    if (flcn_fw_load(falcon_base, &fw) != 0) {
        lx_printk("nouveau-lx: falcon %s raw DMA load falló\n", name);
        return -1;
    }

    if (flcn_fw_boot_ga102(falcon_base, &fw, raw->mbox0, raw->mbox1,
                            raw->check_mbox0, 0u,
                            raw->timeout_ms ? raw->timeout_ms : 4000u) != 0) {
        lx_printk("nouveau-lx: falcon %s raw boot falló\n", name);
        return -1;
    }
    lx_printk("nouveau-lx: falcon %s raw boot ok\n", name);
    return 0;
}
