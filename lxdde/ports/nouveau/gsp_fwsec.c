/* FWSEC-FRTS: extrae ucode FWSEC de la VBIOS (PROM @0x300000, cadena PCIR/NPDE,
 * BIT 0x70 → PmuLookupTable 0x85), parchea FRTS + firma por fuse y arranca en
 * el falcon GSP antes del booter_load SEC2 (nova-core vbios.rs + fwsec.rs). */
#include "gsp_fwsec.h"
#include "falcon_lx.h"
#include "gsp_dma.h"
#include "gsp_mmio.h"
#include "gsp_top.h"
#include "lx_emul.h"

void *memcpy(void *dst, const void *src, unsigned long n);
void *memset(void *dst, int c, unsigned long n);

#define NV_PROM_BASE            0x00300000u
#define NV_PROM_MAX             0x00100000u

#define NV_PFB_PRI_MMU_WPR2_ADDR_LO 0x001fa824u
#define NV_PFB_PRI_MMU_WPR2_ADDR_HI 0x001fa828u
#define NV_PBUS_SW_SCRATCH_0E   0x00001438u
#define LX_FLCN_ENG_IDLE        0x100u

#define NV_FUSE_OPT_FPF_NVDEC_UCODE1_VERSION 0x00824100u
#define NV_FUSE_OPT_FPF_SEC2_UCODE1_VERSION  0x00824140u
#define NV_FUSE_OPT_FPF_GSP_UCODE1_VERSION   0x008241c0u
#define NV_FUSE_OPT_FPF_SIZE                 16u

#define NVFW_FALCON_APPIF_ID_DMEMMAPPER 0x4u
#define NVFW_FALCON_APPIF_DMEMMAPPER_CMD_FRTS 0x15u
#define NVFW_FRTS_CMD_REGION_TYPE_FB 2u
#define PMU_APPID_FWSEC_PROD 0x85u
#define BIT_TOKEN_FALCON_DATA 0x70u
#define BCRT30_RSA3K_SIG_SIZE 384u

struct falcon_ucode_desc_v2 {
    uint32_t hdr;
    uint32_t stored_size;
    uint32_t uncompressed_size;
    uint32_t virtual_entry;
    uint32_t interface_offset;
    uint32_t imem_phys_base;
    uint32_t imem_load_size;
    uint32_t imem_virt_base;
    uint32_t imem_sec_base;
    uint32_t imem_sec_size;
    uint32_t dmem_offset;
    uint32_t dmem_phys_base;
    uint32_t dmem_load_size;
    uint32_t alt_imem_load_size;
    uint32_t alt_dmem_load_size;
};

struct falcon_ucode_desc_v3 {
    uint32_t hdr;
    uint32_t stored_size;
    uint32_t pkc_data_offset;
    uint32_t interface_offset;
    uint32_t imem_phys_base;
    uint32_t imem_load_size;
    uint32_t imem_virt_base;
    uint32_t dmem_phys_base;
    uint32_t dmem_load_size;
    uint16_t engine_id_mask;
    uint8_t ucode_id;
    uint8_t signature_count;
    uint16_t signature_versions;
    uint16_t reserved;
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
} __attribute__((packed));

struct frts_region_cmd {
    uint32_t ver;
    uint32_t hdr;
    uint32_t addr;
    uint32_t size;
    uint32_t ftype;
} __attribute__((packed));

struct frts_cmd {
    struct read_vbios_cmd read_vbios;
    struct frts_region_cmd frts_region;
} __attribute__((packed));

#define NVFW_DMAP_SIGNATURE 0x50414d44u /* "DMAP" */

struct fwsec_ucode_info {
    uint8_t version;
    unsigned desc_size;
    unsigned interface_offset;
    unsigned imem_load_size;
    unsigned dmem_load_size;
    unsigned imem_phys_base;
    unsigned dmem_phys_base;
    unsigned imem_src;
    unsigned dmem_src;
    unsigned pkc_data_offset;
    uint16_t engine_id_mask;
    uint8_t ucode_id;
    uint8_t signature_count;
    uint16_t signature_versions;
    unsigned ucode_rom_off;
    unsigned ucode_len;
    unsigned sig_rom_off;
};

static unsigned rom_data_len;
static int fwsec_last_patch_appif;

static uint8_t rom_rd8(unsigned off)
{
    unsigned word = off & ~3u;
    uint32_t v;

    if (off >= rom_data_len) {
        return 0;
    }
    v = gsp_mmio_rd32(NV_PROM_BASE + word);
    return (uint8_t)(v >> ((off & 3u) * 8u));
}

static uint16_t rom_rd16(unsigned off)
{
    return (uint16_t)rom_rd8(off) | ((uint16_t)rom_rd8(off + 1) << 8);
}

static uint32_t rom_rd32(unsigned off)
{
    return (uint32_t)rom_rd8(off) |
           ((uint32_t)rom_rd8(off + 1) << 8) |
           ((uint32_t)rom_rd8(off + 2) << 16) |
           ((uint32_t)rom_rd8(off + 3) << 24);
}

static int rom_match(unsigned off, const char *sig, unsigned n)
{
    unsigned i;

    for (i = 0; i < n; i++) {
        if (rom_rd8(off + i) != (uint8_t)sig[i]) {
            return 0;
        }
    }
    return 1;
}

static int rom_copy(unsigned off, void *dst, unsigned len)
{
    unsigned char *d = dst;
    unsigned i;

    if (off + len > rom_data_len) {
        return -1;
    }
    for (i = 0; i < len; i++) {
        d[i] = rom_rd8(off + i);
    }
    return 0;
}

static unsigned align_up512(unsigned v)
{
    return (v + 511u) & ~511u;
}

static unsigned desc_hdr_size(uint32_t hdr)
{
    return (unsigned)(hdr >> 16);
}

static unsigned desc_version(uint32_t hdr)
{
    return (unsigned)((hdr >> 8) & 0xffu);
}

static unsigned popcount16(uint16_t v)
{
    unsigned n = 0;
    while (v) {
        n += v & 1u;
        v >>= 1;
    }
    return n;
}

static unsigned fuse_reg_version(uint16_t data)
{
    unsigned i;

    /* ga100_flcn_fw_signature / nova-core: fls (1-based). */
    for (i = 16u; i > 0u; i--) {
        if (data & (1u << (i - 1u)))
            return i;
    }
    return 0;
}

static int prom_ifr_offset(unsigned *start_out)
{
    uint32_t sig = rom_rd32(0);

    if (sig != 0x4947564eu) { /* "NVGI" */
        *start_out = 0;
        return 0;
    }

    {
        uint32_t fixed1 = rom_rd32(4);
        unsigned ver = (unsigned)((fixed1 >> 8) & 0xffu);
        unsigned fixed_data_size = (unsigned)((fixed1 >> 16) & 0x7fffu);

        if (ver == 1u || ver == 2u) {
            *start_out = fixed_data_size + 4u;
            return 0;
        }
        if (ver == 3u) {
            uint32_t total = rom_rd32(8) & 0xfffffu;
            uint32_t flash_status;
            unsigned dir_off;

            if (total + 4u > rom_data_len) {
                return -1;
            }
            flash_status = rom_rd32(total);
            dir_off = (unsigned)flash_status + 4096u;
            if (dir_off + 12u > rom_data_len) {
                return -1;
            }
            if (rom_rd32(dir_off) != 0x44465252u) { /* "RFRD" */
                lx_printk("nouveau-lx: VBIOS IFR v3 sin directorio RFRD\n");
                return -1;
            }
            *start_out = rom_rd32(dir_off + 8u);
            return 0;
        }
        lx_printk("nouveau-lx: VBIOS IFR versión %u no soportada\n", ver);
        return -1;
    }
}

static unsigned image_size_bytes(unsigned img_off)
{
    unsigned pcir_off = img_off + rom_rd16(img_off + 0x18u);
    uint16_t image_len;
    unsigned npde_off;

    if (pcir_off + 22u > rom_data_len) {
        return 0;
    }
    image_len = (uint16_t)rom_rd8(pcir_off + 16u) |
                ((uint16_t)rom_rd8(pcir_off + 17u) << 8);

    npde_off = (pcir_off + rom_rd16(pcir_off + 10u) + 0x0fu) & ~0x0fu;
    if (npde_off + 12u <= rom_data_len && rom_match(npde_off, "NPDE", 4)) {
        uint16_t sub = (uint16_t)rom_rd8(npde_off + 8u) |
                       ((uint16_t)rom_rd8(npde_off + 9u) << 8);
        if (sub) {
            return (unsigned)sub * 512u;
        }
    }
    if (!image_len) {
        return 0;
    }
    return (unsigned)image_len * 512u;
}

static int image_is_last(unsigned img_off)
{
    unsigned pcir_off = img_off + rom_rd16(img_off + 0x18u);
    unsigned npde_off;
    uint8_t code_type;

    if (pcir_off + 22u > rom_data_len) {
        return 1;
    }
    code_type = rom_rd8(pcir_off + 20u);
    if (code_type == 0x70u) {
        return 1;
    }

    npde_off = (pcir_off + rom_rd16(pcir_off + 10u) + 0x0fu) & ~0x0fu;
    if (npde_off + 12u <= rom_data_len && rom_match(npde_off, "NPDE", 4)) {
        return (rom_rd8(npde_off + 0xau) & 0x80u) != 0;
    }
    return (rom_rd8(pcir_off + 21u) & 0x80u) != 0;
}

static uint8_t image_code_type(unsigned img_off)
{
    unsigned pcir_off = img_off + rom_rd16(img_off + 0x18u);

    if (pcir_off + 21u > rom_data_len) {
        return 0xffu;
    }
    return rom_rd8(pcir_off + 20u);
}

static void log_image_chain(unsigned start)
{
    unsigned off = start;
    unsigned n = 0;

    lx_printk("nouveau-lx: VBIOS cadena imágenes @0x%x:\n", start);
    while (off < rom_data_len && n < 16u) {
        unsigned sz;
        uint16_t sig;

        if (off + 2u > rom_data_len) {
            break;
        }
        sig = (uint16_t)rom_rd8(off) | ((uint16_t)rom_rd8(off + 1) << 8);
        if (sig != 0xaa55u && sig != 0x4e56u) {
            lx_printk("nouveau-lx:   [%u] @0x%x sig=0x%04x (fin)\n", n, off, sig);
            break;
        }
        sz = image_size_bytes(off);
        if (!sz) {
            lx_printk("nouveau-lx:   [%u] @0x%x tipo=0x%02x tamaño inválido\n",
                      n, off, image_code_type(off));
            break;
        }
        lx_printk("nouveau-lx:   [%u] @0x%x tipo=0x%02x size=0x%x last=%u\n",
                  n, off, image_code_type(off), sz, image_is_last(off));
        if (image_is_last(off)) {
            break;
        }
        off = align_up512(off + sz);
        n++;
    }
}

static int find_bit_falcon_ptr(unsigned pciat_off, unsigned pciat_len, unsigned *ptr_out)
{
    unsigned i;
    unsigned end = pciat_off + pciat_len;

    for (i = pciat_off; i + 6u < end; i++) {
        if (rom_rd8(i) != 0xffu || rom_rd8(i + 1) != 0xb8u) {
            continue;
        }
        if (!rom_match(i + 2, "BIT", 3) || rom_rd8(i + 5) != 0u) {
            continue;
        }
        {
            unsigned hdr = i;
            unsigned hdr_size = rom_rd8(hdr + 8);
            unsigned token_size = rom_rd8(hdr + 9);
            unsigned token_count = rom_rd8(hdr + 10);
            unsigned tok_base = hdr + hdr_size;
            unsigned j;

            for (j = 0; j < token_count; j++) {
                unsigned eoff = tok_base + j * token_size;
                uint8_t id;
                uint16_t data_off;

                if (eoff + 6u > end) {
                    break;
                }
                id = rom_rd8(eoff);
                if (id != BIT_TOKEN_FALCON_DATA) {
                    continue;
                }
                data_off = (uint16_t)rom_rd8(eoff + 4) |
                           ((uint16_t)rom_rd8(eoff + 5) << 8);
                if ((unsigned)data_off + 4u > pciat_len) {
                    lx_printk("nouveau-lx: BIT 0x70 data_offset 0x%x fuera de PCI-AT\n",
                              data_off);
                    return -1;
                }
                *ptr_out = rom_rd32(pciat_off + data_off);
                lx_printk("nouveau-lx: BIT 0x70 ptr=0x%x (pciat_len=0x%x)\n",
                          *ptr_out, pciat_len);
                return 0;
            }
        }
    }
    lx_printk("nouveau-lx: VBIOS sin token BIT 0x70 (Falcon data)\n");
    return -1;
}

static int pmu_lookup_fwsec(unsigned fwsec_off, unsigned fwsec_len,
                            unsigned pciat_len, unsigned falcon_data_off,
                            unsigned *ucode_off_out)
{
    unsigned hdr_len;
    unsigned entry_len;
    unsigned entry_count;
    unsigned i;

    unsigned base = fwsec_off + falcon_data_off;

    if (falcon_data_off + 4u > fwsec_len) {
        return -1;
    }
    hdr_len = rom_rd8(base + 1);
    entry_len = rom_rd8(base + 2);
    entry_count = rom_rd8(base + 3);

    lx_printk("nouveau-lx: PmuLookupTable @0x%x ver=%u hdr=%u entry=%u count=%u\n",
              falcon_data_off, rom_rd8(base), hdr_len, entry_len, entry_count);

    for (i = 0; i < entry_count; i++) {
        unsigned eoff = falcon_data_off + hdr_len + i * entry_len;
        uint8_t app_id;
        uint32_t data;

        if (eoff + entry_len > fwsec_len || entry_len < 6u) {
            break;
        }
        app_id = rom_rd8(fwsec_off + eoff);
        data = rom_rd32(fwsec_off + eoff + 2);
        if (app_id == PMU_APPID_FWSEC_PROD) {
            if (data < pciat_len) {
                lx_printk("nouveau-lx: PMU 0x85 data=0x%x < pciat_len\n", data);
                return -1;
            }
            *ucode_off_out = data - pciat_len;
            lx_printk("nouveau-lx: PMU 0x85 ucode_off=0x%x\n", *ucode_off_out);
            return 0;
        }
    }
    lx_printk("nouveau-lx: PmuLookupTable sin entrada 0x85 (FWSEC_PROD)\n");
    return -1;
}

static int parse_fwsec_desc(unsigned fwsec_off, unsigned fwsec_len,
                            unsigned ucode_rel_off, struct fwsec_ucode_info *info)
{
    unsigned abs = fwsec_off + ucode_rel_off;
    uint32_t hdr;
    uint8_t ver;

    if (abs + 8u > fwsec_off + fwsec_len) {
        return -1;
    }
    hdr = rom_rd32(abs);
    ver = (uint8_t)desc_version(hdr);
    info->version = ver;
    info->desc_size = desc_hdr_size(hdr);

    if (ver == 3u) {
        struct falcon_ucode_desc_v3 d;

        if (info->desc_size < sizeof(d) ||
            rom_copy(abs, &d, sizeof(d)) != 0) {
            return -1;
        }
        info->interface_offset = d.interface_offset;
        info->imem_load_size = d.imem_load_size;
        info->dmem_load_size = d.dmem_load_size;
        info->imem_phys_base = d.imem_phys_base;
        info->dmem_phys_base = d.dmem_phys_base;
        info->pkc_data_offset = d.pkc_data_offset;
        info->engine_id_mask = d.engine_id_mask;
        info->ucode_id = d.ucode_id;
        info->signature_count = d.signature_count;
        info->signature_versions = d.signature_versions;
        info->imem_src = 0;
        info->dmem_src = d.imem_load_size;
    } else if (ver == 2u) {
        struct falcon_ucode_desc_v2 d;

        if (info->desc_size < sizeof(d) ||
            rom_copy(abs, &d, sizeof(d)) != 0) {
            return -1;
        }
        info->interface_offset = d.interface_offset;
        info->imem_load_size = d.imem_load_size;
        info->dmem_load_size = d.dmem_load_size;
        info->imem_phys_base = d.imem_phys_base;
        info->dmem_phys_base = d.dmem_phys_base;
        info->pkc_data_offset = 0;
        info->engine_id_mask = 0;
        info->ucode_id = 0;
        info->signature_count = 0;
        info->signature_versions = 0;
        info->imem_src = 0;
        info->dmem_src = d.dmem_offset;
    } else {
        lx_printk("nouveau-lx: FWSEC desc v%u no soportada\n", ver);
        return -1;
    }

    info->ucode_len = info->imem_load_size + info->dmem_load_size;
    if (ver == 3u) {
        /* nvkm: firmas en ROM justo tras el struct v3 (0x2c); ucode en desc+desc_size. */
        info->sig_rom_off = abs + (unsigned)sizeof(struct falcon_ucode_desc_v3);
        info->ucode_rom_off = abs + info->desc_size;
    } else {
        info->sig_rom_off = abs + info->desc_size;
        info->ucode_rom_off = info->sig_rom_off +
                              (unsigned)info->signature_count * BCRT30_RSA3K_SIG_SIZE;
    }

    if (!info->ucode_len ||
        info->ucode_rom_off + info->ucode_len > fwsec_off + fwsec_len) {
        lx_printk("nouveau-lx: FWSEC ucode fuera de sección FwSec\n");
        return -1;
    }

    lx_printk("nouveau-lx: FWSEC desc v%u imem=%u dmem=%u if=0x%x engine=0x%x "
              "ucode_id=%u sig=%u vers=0x%x\n",
              ver, info->imem_load_size, info->dmem_load_size,
              info->interface_offset, info->engine_id_mask, info->ucode_id,
              info->signature_count, info->signature_versions);
    return 0;
}

static int vbios_locate_fwsec(struct fwsec_ucode_info *info)
{
    unsigned start;
    unsigned off;
    unsigned pciat_off = 0;
    unsigned pciat_len = 0;
    unsigned fwsec_off = 0;
    unsigned fwsec_len = 0;
    unsigned falcon_ptr = 0;
    unsigned falcon_data_off;
    unsigned ucode_rel_off = 0;
    int have_pciat = 0;
    int have_fwsec = 0;

    rom_data_len = NV_PROM_MAX;
    if (prom_ifr_offset(&start) != 0) {
        log_image_chain(0);
        return -1;
    }

    off = start;
    while (off < rom_data_len) {
        unsigned sz;
        uint16_t sig;
        uint8_t ctype;

        if (off + 2u > rom_data_len) {
            break;
        }
        sig = (uint16_t)rom_rd8(off) | ((uint16_t)rom_rd8(off + 1) << 8);
        if (sig != 0xaa55u && sig != 0x4e56u) {
            break;
        }
        sz = image_size_bytes(off);
        if (!sz) {
            break;
        }
        ctype = image_code_type(off);

        if (ctype == 0x00u && !have_pciat) {
            pciat_off = off;
            pciat_len = sz;
            have_pciat = 1;
        } else if (ctype == 0xe0u && !have_fwsec) {
            fwsec_off = off;
            fwsec_len = rom_data_len - off;
            have_fwsec = 1;
        }

        if (have_pciat && have_fwsec && image_is_last(off)) {
            break;
        }
        off = align_up512(off + sz);
    }

    if (!have_pciat || !have_fwsec) {
        log_image_chain(start);
        lx_printk("nouveau-lx: VBIOS falta imagen %s%s\n",
                  have_pciat ? "" : "PCI-AT ",
                  have_fwsec ? "" : "FwSec");
        return -1;
    }

    if (find_bit_falcon_ptr(pciat_off, pciat_len, &falcon_ptr) != 0) {
        log_image_chain(start);
        return -1;
    }
    if (falcon_ptr < pciat_len) {
        lx_printk("nouveau-lx: BIT falcon ptr 0x%x < pciat_len 0x%x\n",
                  falcon_ptr, pciat_len);
        return -1;
    }
    falcon_data_off = falcon_ptr - pciat_len;

    if (pmu_lookup_fwsec(fwsec_off, fwsec_len, pciat_len, falcon_data_off,
                         &ucode_rel_off) != 0) {
        return -1;
    }

    return parse_fwsec_desc(fwsec_off, fwsec_len, ucode_rel_off, info);
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

static uint16_t read_fuse_ucode_version(uint16_t engine_id_mask, uint8_t ucode_id)
{
    unsigned base;
    unsigned idx;

    if (ucode_id < 1u || ucode_id > NV_FUSE_OPT_FPF_SIZE) {
        return 0;
    }
    idx = (unsigned)ucode_id - 1u;

    if (engine_id_mask & 0x0400u) {
        base = NV_FUSE_OPT_FPF_GSP_UCODE1_VERSION;
    } else if (engine_id_mask & 0x0001u) {
        base = NV_FUSE_OPT_FPF_SEC2_UCODE1_VERSION;
    } else if (engine_id_mask & 0x0004u) {
        base = NV_FUSE_OPT_FPF_NVDEC_UCODE1_VERSION;
    } else {
        return 0;
    }
    return (uint16_t)(gsp_mmio_rd32(base + idx * 4u) & 0xffffu);
}

static unsigned align_up256(unsigned v)
{
    return (v + 255u) & ~255u;
}

static unsigned fwsec_dmem_base(const struct fwsec_ucode_info *info)
{
    return info->dmem_src;
}

static int patch_fwsec_signature(unsigned char *ucode, unsigned ulen,
                                 const struct fwsec_ucode_info *info)
{
    uint16_t fuse_data;
    unsigned fuse_ver;
    uint16_t mask;
    unsigned sig_idx;
    unsigned patch_off;
    unsigned sig_off;

    if (!info->signature_count) {
        return 0;
    }

    fuse_data = read_fuse_ucode_version(info->engine_id_mask, info->ucode_id);
    fuse_ver = fuse_reg_version(fuse_data);
    if (fuse_ver == 0u || fuse_ver >= 16u) {
        lx_printk("nouveau-lx: FWSEC fuse inválido data=0x%x\n", fuse_data);
        return -1;
    }
    mask = (uint16_t)(1u << fuse_ver);

    lx_printk("nouveau-lx: FWSEC fuse data=0x%x ver=%u sig_versions=0x%x\n",
              fuse_data, fuse_ver, info->signature_versions);

    if (!(info->signature_versions & mask)) {
        lx_printk("nouveau-lx: FWSEC sin firma para fuse ver %u (mask=0x%x)\n",
                  fuse_ver, mask);
        return -1;
    }

    sig_idx = popcount16((uint16_t)(info->signature_versions & (mask - 1u)));
    if (sig_idx >= info->signature_count) {
        return -1;
    }

    patch_off = fwsec_dmem_base(info) + info->pkc_data_offset;
    sig_off = info->sig_rom_off + sig_idx * BCRT30_RSA3K_SIG_SIZE;

    if (patch_off + BCRT30_RSA3K_SIG_SIZE > ulen) {
        return -1;
    }

    if (rom_copy(sig_off, ucode + patch_off, BCRT30_RSA3K_SIG_SIZE) != 0) {
        return -1;
    }

    lx_printk("nouveau-lx: FWSEC firma idx=%u → pkc+0x%x\n", sig_idx,
              info->pkc_data_offset);
    return 0;
}

static int patch_frts_cmd_prepare(unsigned char *ucode, unsigned ulen,
                                  const struct fwsec_ucode_info *info,
                                  struct falcon_appif_dmemmapper_v3 *map,
                                  uint64_t frts_addr, uint64_t frts_size)
{
    struct frts_cmd *cmd;

    if (info->imem_load_size + map->cmd_in_buffer_offset + sizeof(*cmd) > ulen) {
        lx_printk("nouveau-lx: FWSEC cmd buf off=0x%x fuera de ucode (0x%x)\n",
                  map->cmd_in_buffer_offset, ulen);
        return -1;
    }
    map->init_cmd = NVFW_FALCON_APPIF_DMEMMAPPER_CMD_FRTS;
    cmd = (struct frts_cmd *)(ucode + fwsec_dmem_base(info) +
                              map->cmd_in_buffer_offset);
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

static struct falcon_appif_dmemmapper_v3 *find_dmap_mapper(unsigned char *ucode,
                                                           unsigned ulen,
                                                           unsigned imem_sz)
{
    unsigned off;

    for (off = imem_sz; off + sizeof(struct falcon_appif_dmemmapper_v3) <= ulen;
         off += 4u) {
        struct falcon_appif_dmemmapper_v3 *map;

        map = (struct falcon_appif_dmemmapper_v3 *)(ucode + off);
        if (map->signature == NVFW_DMAP_SIGNATURE && map->version == 3u) {
            return map;
        }
    }
    return NULL;
}

int gsp_fwsec_patch_via_appif(void)
{
    return fwsec_last_patch_appif;
}

static int patch_fwsec_frts(unsigned char *ucode, unsigned ulen,
                            const struct fwsec_ucode_info *info,
                            uint64_t frts_addr, uint64_t frts_size)
{
    unsigned hdr_off;
    struct falcon_appif_hdr_v1 hdr;
    unsigned i;
    struct falcon_appif_dmemmapper_v3 *map;

    unsigned dmem_base = fwsec_dmem_base(info);

    fwsec_last_patch_appif = 0;
    hdr_off = dmem_base + info->interface_offset;
    if (hdr_off + sizeof(hdr) > ulen) {
        lx_printk("nouveau-lx: FWSEC appif fuera de ucode (off=0x%x ulen=0x%x)\n",
                  hdr_off, ulen);
        goto scan_dmap;
    }
    memcpy(&hdr, ucode + hdr_off, sizeof(hdr));
    if (hdr.version != 1u) {
        lx_printk("nouveau-lx: FWSEC appif ver=%u (esperaba 1) off=0x%x\n",
                  hdr.version, hdr_off);
        goto scan_dmap;
    }

    lx_printk("nouveau-lx: FWSEC appif off=0x%x cnt=%u esz=%u hs=%u\n",
              hdr_off, hdr.entry_count, hdr.entry_size, hdr.header_size);

    for (i = 0; i < hdr.entry_count; i++) {
        unsigned eoff = hdr_off + hdr.header_size + i * hdr.entry_size;
        struct falcon_appif_v1 app;

        if (eoff + sizeof(app) > ulen) {
            break;
        }
        memcpy(&app, ucode + eoff, sizeof(app));
        if (app.id != NVFW_FALCON_APPIF_ID_DMEMMAPPER) {
            continue;
        }
        if (dmem_base + app.dmem_base + sizeof(*map) > ulen) {
            lx_printk("nouveau-lx: FWSEC mapper off=0x%x fuera de ucode\n",
                      app.dmem_base);
            goto scan_dmap;
        }
        map = (struct falcon_appif_dmemmapper_v3 *)(ucode + dmem_base +
                                                      app.dmem_base);
        if (patch_frts_cmd_prepare(ucode, ulen, info, map, frts_addr, frts_size) == 0) {
            lx_printk("nouveau-lx: FWSEC DMEMMAPPER vía appif dmem=0x%x\n",
                      app.dmem_base);
            fwsec_last_patch_appif = 1;
            return 0;
        }
    }

scan_dmap:
    map = find_dmap_mapper(ucode, ulen, dmem_base);
    if (map && patch_frts_cmd_prepare(ucode, ulen, info, map, frts_addr, frts_size) == 0) {
        lx_printk("nouveau-lx: FWSEC DMEMMAPPER vía firma DMAP @0x%x\n",
                  (unsigned)((unsigned char *)map - ucode));
        return 0;
    }
    lx_printk("nouveau-lx: FWSEC-FRTS sin DMEMMAPPER (if=0x%x imem=0x%x dmem=0x%x)\n",
              info->interface_offset, info->imem_load_size, info->dmem_load_size);
    return -1;
}

static int gsp_fwsec_wait_wpr2(uint64_t *lo_out, unsigned budget_ms)
{
    unsigned t;

    for (t = 0; t < budget_ms; t++) {
        uint64_t lo;

        if (gsp_fwsec_wpr2_present(&lo, NULL)) {
            if (lo_out) {
                *lo_out = lo;
            }
            return 0;
        }
        lx_mdelay(1);
    }
    return -1;
}

#define NV_PMC_ENABLE 0x000600u

static void gsp_fwsec_prepare_hw(void)
{
    uint32_t pmc;

    gsp_mc_init_ampere();
    pmc = gsp_mmio_rd32(NV_PMC_ENABLE);
    lx_printk("nouveau-lx: FWSEC prepare PMC enable=0x%08x\n", pmc);
}

static void gsp_fwsec_wait_engine_idle(unsigned falcon_base, unsigned budget_ms)
{
    /* 0x100 es CPUCTL: bit 4 SET = HALTED (el falcon ya paró en raw_boot). */
    uint32_t eng = gsp_mmio_rd32(falcon_base + LX_FLCN_ENG_IDLE);
    uint32_t riscv = gsp_mmio_rd32(falcon_base + 0x1000u + 0x388u);

    (void)budget_ms;
    lx_printk("nouveau-lx: FWSEC falcon eng=0x%08x riscv=0x%08x\n", eng, riscv);
}

int gsp_fwsec_probe(uint64_t frts_addr, uint64_t frts_size)
{
    struct fwsec_ucode_info info;
    unsigned char *ucode;
    struct gsp_dma_buf dma;

    (void)frts_addr;
    (void)frts_size;

    memset(&info, 0, sizeof(info));
    if (vbios_locate_fwsec(&info) != 0) {
        return -1;
    }

    ucode = lx_kmalloc(info.ucode_len, GFP_KERNEL);
    if (!ucode) {
        return -1;
    }
    if (rom_copy(info.ucode_rom_off, ucode, info.ucode_len) != 0) {
        lx_kfree(ucode);
        return -1;
    }
    if (patch_fwsec_signature(ucode, info.ucode_len, &info) != 0) {
        lx_printk("nouveau-lx: FWSEC-FRTS parche firma falló\n");
        lx_kfree(ucode);
        return -1;
    }
    if (patch_fwsec_frts(ucode, info.ucode_len, &info, frts_addr, frts_size) != 0) {
        lx_printk("nouveau-lx: FWSEC-FRTS parche DMEMMAPPER falló\n");
        lx_kfree(ucode);
        return -1;
    }

    if (gsp_dma_alloc_copy(&dma, ucode, info.ucode_len, "fwsec-frts") != 0) {
        lx_kfree(ucode);
        return -1;
    }
    lx_kfree(ucode);
    gsp_dma_free(&dma);
    lx_printk("nouveau-lx: FWSEC probe OK (imem=%u dmem=%u)\n",
              info.imem_load_size, info.dmem_load_size);
    return 0;
}

int gsp_fwsec_run_frts(uint64_t frts_addr, uint64_t frts_size)
{
    struct fwsec_ucode_info info;
    unsigned char *ucode;
    struct gsp_dma_buf dma;
    struct falcon_lx_raw raw;
    uint32_t scratch;
    uint64_t wpr2_lo;

    if (!gsp_mmio_alive()) {
        return -1;
    }

    if (gsp_fwsec_wpr2_present(NULL, NULL)) {
        lx_printk("nouveau-lx: WPR2 ya presente (0x%llx-0x%llx) — omito FWSEC-FRTS\n",
                  (unsigned long long)wpr2_lo_bound(),
                  (unsigned long long)wpr2_hi_bound());
        return 0;
    }

    memset(&info, 0, sizeof(info));
    if (vbios_locate_fwsec(&info) != 0) {
        lx_printk("nouveau-lx: Ampere FWSEC-FRTS: VBIOS incompleta\n");
        return -1;
    }

    ucode = lx_kmalloc(info.ucode_len, GFP_KERNEL);
    if (!ucode) {
        return -1;
    }
    if (rom_copy(info.ucode_rom_off, ucode, info.ucode_len) != 0) {
        lx_kfree(ucode);
        return -1;
    }
    if (patch_fwsec_signature(ucode, info.ucode_len, &info) != 0) {
        lx_printk("nouveau-lx: FWSEC-FRTS parche firma falló\n");
        lx_kfree(ucode);
        return -1;
    }
    if (patch_fwsec_frts(ucode, info.ucode_len, &info, frts_addr, frts_size) != 0) {
        lx_printk("nouveau-lx: FWSEC-FRTS parche DMEMMAPPER falló\n");
        lx_kfree(ucode);
        return -1;
    }

    if (gsp_dma_alloc_copy(&dma, ucode, info.ucode_len, "fwsec-frts") != 0) {
        lx_kfree(ucode);
        return -1;
    }
    lx_kfree(ucode);
    ucode = dma.va;

    lx_printk("nouveau-lx: FWSEC-FRTS frts=0x%llx size=0x%llx imem=%u dmem=%u\n",
              (unsigned long long)frts_addr, (unsigned long long)frts_size,
              info.imem_load_size, info.dmem_load_size);

    memset(&raw, 0, sizeof(raw));
    raw.img = ucode;
    raw.dma_handle = dma.phys;
    raw.imem_src = info.imem_src;
    raw.imem_dst = info.imem_phys_base;
    raw.imem_len = info.imem_load_size;
    raw.dmem_src = info.dmem_src;
    raw.dmem_dst = info.dmem_phys_base;
    raw.dmem_len = align_up256(info.dmem_load_size);
    raw.pkc_data_offset = info.pkc_data_offset;
    raw.engine_id_mask = info.engine_id_mask;
    raw.ucode_id = info.ucode_id;
    raw.boot_addr = 0;
    raw.mbox0 = 0;
    raw.mbox1 = 0;
    raw.check_mbox0 = 1;
    raw.timeout_ms = 4000u;
    raw.name = "fwsec-frts";

#ifndef SOSO_FWSEC_HOSTCHECK
    gsp_fwsec_prepare_hw();
    if (falcon_lx_raw_boot(LX_FLCN_GSP_BASE, &raw) != 0) {
        gsp_dma_free(&dma);
        return -1;
    }
    gsp_fwsec_wait_engine_idle(LX_FLCN_GSP_BASE, 2000u);
#endif

    scratch = gsp_mmio_rd32(NV_PBUS_SW_SCRATCH_0E);
    if (scratch & 0xffff0000u) {
        lx_printk("nouveau-lx: FWSEC-FRTS error scratch=0x%08x (hi=0x%x lo=0x%x)\n",
                  scratch, scratch >> 16, scratch & 0xffffu);
        gsp_dma_free(&dma);
        return -1;
    }
    if (scratch & 0x0000ffffu) {
        lx_printk("nouveau-lx: FWSEC-FRTS scratch lower=0x%04x\n", scratch & 0xffffu);
    }
    if (gsp_fwsec_wait_wpr2(&wpr2_lo, 4000u) != 0) {
        uint32_t wpr2_raw_lo = gsp_mmio_rd32(NV_PFB_PRI_MMU_WPR2_ADDR_LO);
        uint32_t wpr2_raw_hi = gsp_mmio_rd32(NV_PFB_PRI_MMU_WPR2_ADDR_HI);
        lx_printk("nouveau-lx: FWSEC-FRTS terminó pero WPR2 sigue vacío "
                  "(raw lo=0x%08x hi=0x%08x scratch=0x%08x)\n",
                  wpr2_raw_lo, wpr2_raw_hi, scratch);
        gsp_dma_free(&dma);
        return -1;
    }
    gsp_dma_free(&dma);

    if (wpr2_lo != frts_addr) {
        lx_printk("nouveau-lx: FWSEC WPR2 @0x%llx esperaba 0x%llx\n",
                  (unsigned long long)wpr2_lo, (unsigned long long)frts_addr);
        return -1;
    }
    lx_printk("nouveau-lx: FWSEC-FRTS OK — WPR2 0x%llx-0x%llx\n",
              (unsigned long long)wpr2_lo_bound(),
              (unsigned long long)wpr2_hi_bound());
    return 0;
}
