/* G3b paso 4: bootloader RISC-V en sysmem + `GspFwWprMeta`. Ver gsp_wpr.h. */
#include "gsp_wpr.h"
#include "gsp_mmio.h"

/* Definidos en el shim (lxdde/shim/src/shims.c). */
void *memcpy(void *dst, const void *src, unsigned long n);
void *memset(void *dst, int c, unsigned long n);

/* Si la estructura no mide 256 bytes exactos, el firmware lee basura: que no
 * compile antes que descubrirlo con un DMA. */
typedef char gsp_wpr_meta_size_check[sizeof(struct gsp_wpr_meta) == 256 ? 1 : -1];
typedef char gsp_wpr_meta_fb_check[offsetof(struct gsp_wpr_meta, fbSize) == 0xb0 ? 1 : -1];
typedef char gsp_wpr_meta_pmu_check[offsetof(struct gsp_wpr_meta, pmuReservedSize) == 0xf4 ? 1 : -1];
typedef char gsp_wpr_meta_ver_check[offsetof(struct gsp_wpr_meta, verified) == 0xf8 ? 1 : -1];

#define GSP_FW_WPR_META_MAGIC    0xdc3aae21371a60b3ull
#define GSP_FW_WPR_META_REVISION 1ull

/* `ga102_fb_vidmem_size`: el registro da la VRAM en MiB. Vale de Ampere a
 * Blackwell (gb202_fb también lo usa). Está dentro de los 16 MiB de BAR0. */
#define NV_FB_VIDMEM_SIZE_MB 0x001183a4u

/* Parámetros de heap de `r570_wpr_libos3_gb20x` (nvkm/subdev/gsp/rm/r570/rm.c)
 * y `GSP_FW_HEAP_PARAM_*` de los headers nvrm. Todos son de GB20x: otras
 * familias tienen otros y esta ruta es solo la del FMC. */
#define WPR_OS_CARVEOUT_SIZE   (22u << 20)          /* LIBOS3 baremetal */
#define WPR_BASE_RM_SIZE       (14u << 20)          /* Hopper+ */
#define WPR_SIZE_PER_GB_FB     (96u << 10)
#define WPR_CLIENT_ALLOC_SIZE  ((48ull << 10) * 2048ull)
#define WPR_HEAP_NON_WPR       0x220000u            /* gb20x */
#define WPR_RSVD_SIZE_PMU      0x1820000u           /* ALIGN(0x800000+0x1000000+0x1000, 0x20000) */
#define WPR_FRTS_SIZE          0x100000u
#define WPR_VGA_WORKSPACE_SIZE (128u * 1024u)

/* Cabecera NVIDIA de los blobs de arranque (`nvfw_bin_hdr`, include/nvfw/fw.h). */
struct gsp_bin_hdr {
    uint32_t bin_magic;      /* 0x10de */
    uint32_t bin_ver;
    uint32_t bin_size;
    uint32_t header_offset;  /* → RM_RISCV_UCODE_DESC */
    uint32_t data_offset;    /* → imagen RISC-V */
    uint32_t data_size;
};

/* `RM_RISCV_UCODE_DESC` (rm/r535/nvrm/gsp.h). Verificado contra el blob real:
 * version=5, manifest 0..0xa00, data 0xa00..0xb200, code 0xb200..0x30a00. */
struct gsp_riscv_desc {
    uint32_t version;
    uint32_t bootloaderOffset;
    uint32_t bootloaderSize;
    uint32_t bootloaderParamOffset;
    uint32_t bootloaderParamSize;
    uint32_t riscvElfOffset;
    uint32_t riscvElfSize;
    uint32_t appVersion;
    uint32_t manifestOffset;
    uint32_t manifestSize;
    uint32_t monitorDataOffset;
    uint32_t monitorDataSize;
    uint32_t monitorCodeOffset;
    uint32_t monitorCodeSize;
    uint32_t bIsMonitorEnabled;
    uint32_t swbromCodeOffset;
    uint32_t swbromCodeSize;
    uint32_t swbromDataOffset;
    uint32_t swbromDataSize;
    uint32_t fbReservedSize;
    uint32_t bSignedAsCode;
};

#define GSP_BIN_MAGIC 0x000010deu

static uint64_t align_up_u64(uint64_t v, uint64_t a)
{
    return (v + a - 1ull) & ~(a - 1ull);
}

uint64_t gsp_wpr_vidmem_size(void)
{
    uint32_t mb = gsp_mmio_rd32(NV_FB_VIDMEM_SIZE_MB);

    /* 0 o 0xffffffff = el registro no responde (sin BAR0, o chip que no es este). */
    if (mb == 0 || mb == 0xffffffffu) {
        return 0;
    }
    return (uint64_t)mb << 20;
}

/* `tu102_gsp_wpr_heap_size`. El `max()` con heap_size_min de upstream se queda
 * fuera a propósito: allí vale 170 (MiB sin convertir) contra un total en bytes,
 * así que nunca gana; replicarlo aquí solo confundiría. */
static uint64_t wpr_heap_size(uint64_t fb_bytes)
{
    uint64_t fb_gb = (fb_bytes + (1ull << 30) - 1ull) >> 30;

    return (uint64_t)WPR_OS_CARVEOUT_SIZE +
           (uint64_t)WPR_BASE_RM_SIZE +
           align_up_u64((uint64_t)WPR_SIZE_PER_GB_FB * fb_gb, 1ull << 20) +
           align_up_u64(WPR_CLIENT_ALLOC_SIZE, 1ull << 20);
}

/* Copia la imagen RISC-V del bootloader a memoria coherente y saca sus offsets.
 * Todo lo que viene del blob se comprueba contra su longitud antes de usarlo. */
static int boot_fw_prepare(struct gsp_boot_fw *boot)
{
    const struct gsp_fw_blob *blob = gsp_fw_get(GSP_FW_BOOTLOADER);
    const struct gsp_bin_hdr *hdr;
    const struct gsp_riscv_desc *desc;
    uint64_t end;

    if (!blob || !blob->data || blob->len < sizeof(*hdr)) {
        lx_printk("nouveau-lx: WPR sin bootloader cargado\n");
        return -1;
    }
    hdr = (const struct gsp_bin_hdr *)blob->data;
    if (hdr->bin_magic != GSP_BIN_MAGIC) {
        lx_printk("nouveau-lx: bootloader magic 0x%08x inesperado\n", hdr->bin_magic);
        return -1;
    }
    if ((uint64_t)hdr->header_offset + sizeof(*desc) > blob->len ||
        (uint64_t)hdr->data_offset + hdr->data_size > blob->len || hdr->data_size == 0) {
        lx_printk("nouveau-lx: bootloader cabecera fuera del blob\n");
        return -1;
    }
    desc = (const struct gsp_riscv_desc *)(blob->data + hdr->header_offset);

    /* Manifest, datos y código tienen que caber en la imagen que vamos a copiar. */
    end = (uint64_t)desc->monitorCodeOffset + desc->monitorCodeSize;
    if (end > hdr->data_size ||
        (uint64_t)desc->monitorDataOffset + desc->monitorDataSize > hdr->data_size ||
        (uint64_t)desc->manifestOffset + desc->manifestSize > hdr->data_size) {
        lx_printk("nouveau-lx: bootloader offsets fuera de la imagen (data=%u)\n",
                  hdr->data_size);
        return -1;
    }
    if (!desc->bIsMonitorEnabled) {
        lx_printk("nouveau-lx: bootloader sin monitor habilitado\n");
        return -1;
    }

    {
        /* Publicar va/phys/size a la vez: si se dejara `va` puesto con `size` a
         * cero, el release liberaría un tamaño que no es el reservado. */
        uint64_t phys = 0;
        void *va = lx_dma_alloc_coherent(NULL, hdr->data_size, &phys, GFP_KERNEL);
        if (!va || !phys) {
            if (va) {
                lx_dma_free_coherent(NULL, hdr->data_size, va, phys);
            }
            lx_printk("nouveau-lx: WPR sin memoria para el bootloader (%u bytes)\n",
                      hdr->data_size);
            return -1;
        }
        boot->va = va;
        boot->phys = phys;
        boot->size = hdr->data_size;
    }
    memcpy(boot->va, blob->data + hdr->data_offset, hdr->data_size);
    boot->code_offset = desc->monitorCodeOffset;
    boot->data_offset = desc->monitorDataOffset;
    boot->manifest_offset = desc->manifestOffset;
    boot->app_version = desc->appVersion;

    lx_printk("nouveau-lx: bootloader v%u %lu KiB code=0x%x data=0x%x manifest=0x%x @0x%llx\n",
              desc->version, boot->size / 1024u, boot->code_offset, boot->data_offset,
              boot->manifest_offset, (unsigned long long)boot->phys);
    return 0;
}

/* Releer lo escrito: el FMC no avisa de un campo mal puesto, se cuelga. */
static int wpr_meta_verify(const struct gsp_wpr *w, const struct gsp_rm_fw *rm)
{
    const struct gsp_wpr_meta *m = w->meta;

    if (m->magic != GSP_FW_WPR_META_MAGIC || m->revision != GSP_FW_WPR_META_REVISION) {
        lx_printk("nouveau-lx: WPR meta magic/revision mal escritos\n");
        return -1;
    }
    if (m->sysmemAddrOfRadix3Elf != rm->rx3.mem[0].phys ||
        m->sizeOfRadix3Elf != rm->img_len ||
        m->sysmemAddrOfSignature != rm->sig_phys ||
        m->sizeOfSignature != rm->sig_len) {
        lx_printk("nouveau-lx: WPR meta no cuadra con la imagen GSP-RM\n");
        return -1;
    }
    if (m->sysmemAddrOfBootloader != w->boot.phys || m->sizeOfBootloader != w->boot.size) {
        lx_printk("nouveau-lx: WPR meta no cuadra con el bootloader\n");
        return -1;
    }
    if (!m->gspFwHeapSize || !m->nonWprHeapSize || !m->frtsSize || !m->vgaWorkspaceSize) {
        lx_printk("nouveau-lx: WPR meta con tamaños a cero\n");
        return -1;
    }
    /* En la ruta FMC las direcciones de WPR las pone el propio FMC: si aquí
     * hubiera algo distinto de cero sería que nos hemos inventado un layout. */
    if (m->gspFwWprStart || m->gspFwWprEnd || m->gspFwHeapOffset || m->gspFwOffset ||
        m->bootBinOffset || m->frtsOffset || m->nonWprHeapOffset || m->gspFwRsvdStart) {
        lx_printk("nouveau-lx: WPR meta con offsets que debe fijar el FMC\n");
        return -1;
    }
    if (m->bootCount || m->verified) {
        lx_printk("nouveau-lx: WPR meta con estado de arranque previo\n");
        return -1;
    }
    return 0;
}

int gsp_wpr_prepare(const struct gsp_rm_fw *rm, struct gsp_wpr *out)
{
    struct gsp_wpr_meta *m;

    if (!out || !rm || !rm->ready) {
        return -1;
    }
    memset(out, 0, sizeof(*out));

    out->fb_bytes = gsp_wpr_vidmem_size();
    if (!out->fb_bytes) {
        lx_printk("nouveau-lx: WPR sin tamaño de VRAM (0x%08x)\n", NV_FB_VIDMEM_SIZE_MB);
        return -1;
    }
    out->heap_size = wpr_heap_size(out->fb_bytes);
    /* `gh100_gsp_init`: rsvd = heap fuera de WPR + reserva del PMU, a 2 MiB. */
    out->rsvd_size = (uint32_t)align_up_u64((uint64_t)WPR_HEAP_NON_WPR +
                                            (uint64_t)WPR_RSVD_SIZE_PMU, 0x200000ull);

    if (boot_fw_prepare(&out->boot) != 0) {
        gsp_wpr_release(out);
        return -1;
    }

    out->meta = lx_dma_alloc_coherent(NULL, sizeof(*out->meta), &out->meta_phys, GFP_KERNEL);
    if (!out->meta || !out->meta_phys) {
        lx_printk("nouveau-lx: WPR sin memoria para el meta\n");
        gsp_wpr_release(out);
        return -1;
    }
    m = out->meta;
    memset(m, 0, sizeof(*m));

    /* Exactamente los campos que pone `gh100_gsp_wpr_meta_init`, ni uno más: el
     * resto (offsets de WPR, FRTS, VGA, contadores) los rellena el FMC. */
    m->magic = GSP_FW_WPR_META_MAGIC;
    m->revision = GSP_FW_WPR_META_REVISION;

    m->sysmemAddrOfRadix3Elf = rm->rx3.mem[0].phys;
    m->sizeOfRadix3Elf = rm->img_len;

    m->sysmemAddrOfBootloader = out->boot.phys;
    m->sizeOfBootloader = out->boot.size;
    m->bootloaderCodeOffset = out->boot.code_offset;
    m->bootloaderDataOffset = out->boot.data_offset;
    m->bootloaderManifestOffset = out->boot.manifest_offset;

    m->sysmemAddrOfSignature = rm->sig_phys;
    m->sizeOfSignature = rm->sig_len;

    m->nonWprHeapSize = WPR_HEAP_NON_WPR;
    m->gspFwHeapSize = out->heap_size;
    m->frtsSize = WPR_FRTS_SIZE;
    m->vgaWorkspaceSize = WPR_VGA_WORKSPACE_SIZE;
    m->pmuReservedSize = WPR_RSVD_SIZE_PMU;

    if (wpr_meta_verify(out, rm) != 0) {
        gsp_wpr_release(out);
        return -1;
    }

    out->ready = 1;
    lx_printk("nouveau-lx: VRAM real %u MiB (0x%08x)\n",
              (unsigned)(out->fb_bytes >> 20), NV_FB_VIDMEM_SIZE_MB);
    lx_printk("nouveau-lx: WPR meta verificado @0x%llx (256 B) heap=%u MiB nonWpr=%u KiB pmu=%u MiB\n",
              (unsigned long long)out->meta_phys, (unsigned)(out->heap_size >> 20),
              (unsigned)(WPR_HEAP_NON_WPR >> 10), (unsigned)(WPR_RSVD_SIZE_PMU >> 20));
    return 0;
}

void gsp_wpr_release(struct gsp_wpr *w)
{
    if (!w) {
        return;
    }
    if (w->meta) {
        lx_dma_free_coherent(NULL, sizeof(*w->meta), w->meta, w->meta_phys);
    }
    if (w->boot.va) {
        lx_dma_free_coherent(NULL, w->boot.size, w->boot.va, w->boot.phys);
    }
    memset(w, 0, sizeof(*w));
}
