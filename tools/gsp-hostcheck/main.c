/* Banco de pruebas en host para los pasos 3 a 6 de la cadena FSP/COT (gsp_rm.c,
 * gsp_wpr.c, gsp_libos.c, fmc_lx.c y fsp_lx.c): los mismos fuentes, con la capa
 * lx y un FSP simulado detrás del MMIO, sobre los blobs reales. Comprueba el
 * parseo de los ELF y cabeceras NVIDIA, la radix3, el GspFwWprMeta, las colas
 * compartidas, los boot args de libos y el paquete COT byte a byte.
 *
 * No sustituye a la prueba en hardware: valida parseo, layout y protocolo, no el
 * arranque real del GSP. Pero el paso 6 escribe en la GPU, así que aquí es donde
 * se caza un paquete mal formado sin arriesgar un cuelgue. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdarg.h>
#include <stddef.h>

#define GFP_KERNEL 0x40u
struct lx_pci_dev;

/* VRAM que finge el registro 0x1183a4 (en MiB), como la RTX 5070 de la máquina. */
#define FAKE_VRAM_MB 12288u

static int lx_printk(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int n = vprintf(fmt, ap);
    va_end(ap);
    return n;
}

/* "Física" simulada: base ficticia + offset del puntero, alineada a página. */
#define FAKE_PHYS_BASE 0x100000000ull
static uint64_t lx_virt_to_phys(const void *p)
{
    return p ? FAKE_PHYS_BASE + ((uintptr_t)p & 0xffffffffull) : 0;
}

static void *lx_alloc_pages_exact(size_t size)
{
    void *p = NULL;
    if (posix_memalign(&p, 4096, (size + 4095) & ~(size_t)4095))
        return NULL;
    memset(p, 0, (size + 4095) & ~(size_t)4095);
    return p;
}
static void lx_free_pages_exact(void *p, size_t size) { (void)size; free(p); }

static void *lx_dma_alloc_coherent(struct lx_pci_dev *d, size_t size, uint64_t *dma, unsigned gfp)
{
    (void)d; (void)gfp;
    void *p = lx_alloc_pages_exact(size);
    if (p && dma)
        *dma = lx_virt_to_phys(p);
    return p;
}
static void lx_dma_free_coherent(struct lx_pci_dev *d, size_t size, void *va, uint64_t dma)
{ (void)d; (void)dma; lx_free_pages_exact(va, size); }

static void lx_mdelay(unsigned ms) { (void)ms; }
static void *lx_kzalloc(size_t size, unsigned gfp)
{ (void)gfp; void *p = malloc(size); if (p) memset(p, 0, size); return p; }
static void lx_kfree(void *p) { free(p); }

/* --- FSP simulado -----------------------------------------------------------
 * Suficiente para ejercitar el camino completo del paso 6 sin GPU: captura el
 * paquete que se escribe en EMEM, responde como respondería el FSP y libera el
 * lockdown tras unas cuantas vueltas. */
#define R_VRAM   0x001183a4u
#define R_THERM  0x00ad00bcu
#define R_QHEAD  0x008f2c00u
#define R_QTAIL  0x008f2c04u
#define R_MHEAD  0x008f2c80u
#define R_MTAIL  0x008f2c84u
#define R_EMEMC  0x008f2ac0u
#define R_EMEMD  0x008f2ac4u
#define R_MBOX0  0x00110040u
#define R_MBOX1  0x00110044u
#define R_HWCFG2 0x001100f4u
#define R_CPUCTL 0x00111388u

static uint32_t fsp_emem[512];
static unsigned fsp_emem_ptr;
static uint32_t fsp_qhead, fsp_qtail, fsp_mhead, fsp_mtail;
static uint32_t fsp_sent[512];
static unsigned fsp_sent_dwords;
static unsigned fsp_mbox_reads;
static uint32_t fsp_reply_error;   /* !=0 para probar el rechazo del FSP */

static void fake_fsp_reset(void)
{
    memset(fsp_emem, 0, sizeof(fsp_emem));
    memset(fsp_sent, 0, sizeof(fsp_sent));
    fsp_emem_ptr = fsp_qhead = fsp_qtail = fsp_mhead = fsp_mtail = 0;
    fsp_sent_dwords = fsp_mbox_reads = 0;
}

/* Escribir QUEUE_HEAD es el timbre: el FSP lee el mensaje y contesta. */
static void fake_fsp_consume(void)
{
    fsp_sent_dwords = fsp_qtail / 4u + 1u;
    if (fsp_sent_dwords > 512) fsp_sent_dwords = 512;
    memcpy(fsp_sent, fsp_emem, fsp_sent_dwords * 4u);
    fsp_qhead = fsp_qtail;            /* consumido */

    fsp_emem[0] = (1u << 31) | (1u << 30);                  /* MCTP SOM|EOM */
    fsp_emem[1] = (0x15u << 24) | (0x10deu << 8) | 0x7eu;   /* NVDM FSP_RESPONSE */
    fsp_emem[2] = 0;                                        /* taskId */
    fsp_emem[3] = 0x14u;                                    /* commandNvdmType = COT */
    fsp_emem[4] = fsp_reply_error;
    fsp_mhead = 0;
    fsp_mtail = 4u * 4u;                                    /* último DWORD escrito */
}

static uint32_t gsp_mmio_rd32(uint32_t off)
{
    switch (off) {
    case R_VRAM:  return FAKE_VRAM_MB;
    case R_THERM: return 0xffu;                 /* secure boot completo */
    case R_QHEAD: return fsp_qhead;
    case R_QTAIL: return fsp_qtail;
    case R_MHEAD: return fsp_mhead;
    case R_MTAIL: return fsp_mtail;
    case R_EMEMD: return fsp_emem_ptr < 512 ? fsp_emem[fsp_emem_ptr++] : 0;
    /* El bootrom tarda unas vueltas en dejar leer el falcon. */
    case R_MBOX0: return fsp_mbox_reads++ < 3 ? 0xbadf4100u : 0u;
    case R_MBOX1: return 0;
    case R_HWCFG2: return 0;                    /* lockdown liberado */
    case R_CPUCTL: return 0x180u;               /* RISC-V activo (bit 7) */
    default: return 0;
    }
}

/* En el banco la GPU nunca se cae del bus: el camino de recuperación por
 * configuración PCI solo tiene sentido contra hardware. */
static int gsp_mmio_alive(void) { return 1; }
static int gsp_mmio_pci_recover(void) { return 0; }

static void gsp_mmio_wr32(uint32_t off, uint32_t val)
{
    switch (off) {
    case R_EMEMC: fsp_emem_ptr = (val & 0xffffffu) / 4u; break;
    case R_EMEMD: if (fsp_emem_ptr < 512) fsp_emem[fsp_emem_ptr++] = val; break;
    case R_QTAIL: fsp_qtail = val; break;
    case R_QHEAD: fsp_qhead = val; fake_fsp_consume(); break;
    case R_MHEAD: fsp_mhead = val; break;
    case R_MTAIL: fsp_mtail = val; break;
    default: break;
    }
}

/* gsp_fw.h simulado. */
enum gsp_fw_kind { GSP_FW_BOOTLOADER = 0, GSP_FW_FMC, GSP_FW_UCODE, GSP_FW_BOOTER_LOAD,
                   GSP_FW_BOOTER_UNLOAD, GSP_FW_COUNT };
enum gsp_fw_chip { GSP_FW_CHIP_BLACKWELL = 0, GSP_FW_CHIP_AMPERE };
struct gsp_fw_blob { const char *path; unsigned char *data; unsigned long len;
                     unsigned gem_handle; int valid; };
static struct gsp_fw_blob g_blobs[GSP_FW_COUNT];
static const struct gsp_fw_blob *gsp_fw_get(enum gsp_fw_kind k)
{ return k < GSP_FW_COUNT && g_blobs[k].valid ? &g_blobs[k] : NULL; }

#include "gsp_dma_body.inc"
#include "gsp_rm_body.inc"
#include "gsp_wpr_body.inc"
#include "gsp_libos_body.inc"
#include "fmc_lx_body.inc"
#include "fsp_lx_body.inc"
#include "gsp_rpc_body.inc"

static int load_blob(enum gsp_fw_kind kind, const char *path)
{
    FILE *f = fopen(path, "rb");
    if (!f) { perror(path); return -1; }
    fseek(f, 0, SEEK_END);
    long len = ftell(f);
    fseek(f, 0, SEEK_SET);
    unsigned char *buf = malloc(len);
    if (fread(buf, 1, len, f) != (size_t)len) { perror("read"); return -1; }
    fclose(f);
    g_blobs[kind].data = buf;
    g_blobs[kind].len = len;
    g_blobs[kind].valid = 1;
    g_blobs[kind].path = path;
    printf("blob %s: %ld bytes\n", path, len);
    return 0;
}

/* Paso 3: imagen GSP-RM + radix3, en las dos familias. */
static int check_radix3(struct gsp_rm_fw *keep)
{
    const enum gsp_fw_chip chips[] = { GSP_FW_CHIP_BLACKWELL, GSP_FW_CHIP_AMPERE };

    for (unsigned c = 0; c < 2; c++) {
        struct gsp_rm_fw fw;
        if (gsp_rm_prepare(chips[c], &fw) != 0) {
            printf("FALLO: gsp_rm_prepare chip=%u\n", chips[c]);
            return -1;
        }
        unsigned long pages = (fw.img_len + 4095) / 4096;
        if (fw.rx3.mem[2].size != ((pages * 8 + 4095) & ~4095ul)) { printf("FALLO: tamaño hoja\n"); return -1; }
        if (fw.rx3.mem[1].size != 4096 || fw.rx3.mem[0].size != 4096) { printf("FALLO: tamaño l1/raíz\n"); return -1; }
        if ((uintptr_t)fw.img & 4095) { printf("FALLO: imagen no alineada\n"); return -1; }
        const uint64_t *l2 = fw.rx3.mem[2].va;
        for (unsigned long j = 0; j < pages; j++) {
            if (l2[j] != lx_virt_to_phys(fw.img + j * 4096)) { printf("FALLO: hoja %lu\n", j); return -1; }
        }
        /* Las entradas sobrantes de la hoja quedan a cero (página parcial). */
        for (unsigned long j = pages; j < fw.rx3.mem[2].size / 8; j++) {
            if (l2[j] != 0) { printf("FALLO: relleno hoja %lu no nulo\n", j); return -1; }
        }
        printf("OK: radix3 %lu páginas, todas las hojas cuadran\n", pages);
        if (c == 0)
            *keep = fw;          /* la de Blackwell alimenta el paso 4 */
        else
            gsp_rm_release(&fw);
    }
    return 0;
}

/* Paso 4: bootloader en sysmem + GspFwWprMeta. */
static int check_wpr(const struct gsp_rm_fw *rm, struct gsp_wpr *keep)
{
    struct gsp_wpr w;

    if (sizeof(struct gsp_wpr_meta) != 256) { printf("FALLO: meta no mide 256 B\n"); return -1; }
    if (gsp_wpr_prepare(rm, &w) != 0) { printf("FALLO: gsp_wpr_prepare\n"); return -1; }

    const struct gsp_wpr_meta *m = w.meta;
    if (w.fb_bytes != (uint64_t)FAKE_VRAM_MB << 20) { printf("FALLO: VRAM\n"); return -1; }
    /* 22 + 14 MiB + ALIGN(96 KiB * 12 GiB, 1 MiB) + 96 MiB = 134 MiB. */
    uint64_t want_heap = (22ull << 20) + (14ull << 20) + (2ull << 20) + (96ull << 20);
    if (m->gspFwHeapSize != want_heap) {
        printf("FALLO: heap %llu != %llu\n", (unsigned long long)m->gspFwHeapSize,
               (unsigned long long)want_heap);
        return -1;
    }
    /* Offsets del bootloader: los del blob real, y dentro de la imagen copiada. */
    if (m->bootloaderManifestOffset != 0x0 || m->bootloaderDataOffset != 0xa00 ||
        m->bootloaderCodeOffset != 0xb200) {
        printf("FALLO: offsets del bootloader inesperados\n");
        return -1;
    }
    if (m->bootloaderCodeOffset >= m->sizeOfBootloader) { printf("FALLO: code fuera\n"); return -1; }
    /* Todo lo que debe fijar el FMC sigue a cero — lo comprueba también
     * wpr_meta_verify(); aquí se repite por si esa comprobación se relaja. */
    if (m->gspFwWprStart || m->gspFwOffset || m->frtsOffset || m->fbSize) {
        printf("FALLO: campos que fija el FMC vienen escritos\n");
        return -1;
    }
    if (m->pmuReservedSize != 0x1820000u) { printf("FALLO: pmuReservedSize\n"); return -1; }
    printf("OK: WPR meta 256 B, heap %llu MiB, offsets del bootloader correctos\n",
           (unsigned long long)(m->gspFwHeapSize >> 20));

    *keep = w;   /* el paso 5 lo necesita vivo */
    return 0;
}

/* Paso 5: colas compartidas, logs, RMARGS, boot params y staging del FMC. */
static int check_libos(const struct gsp_wpr *wpr)
{
    struct gsp_libos lo;
    struct fmc_image img;
    struct fmc_staged staged;
    const struct gsp_fw_blob *fmc = gsp_fw_get(GSP_FW_FMC);

    if (gsp_libos_prepare(wpr, &lo) != 0) { printf("FALLO: gsp_libos_prepare\n"); return -1; }

    /* 128 páginas de colas + 1 de tabla, y la tabla se describe a sí misma. */
    if (lo.shm_ptes_nr != 129 || lo.shm_ptes_size != 4096) {
        printf("FALLO: PTEs de la shm (%u, %lu)\n", lo.shm_ptes_nr, lo.shm_ptes_size);
        return -1;
    }
    if (lo.shm.size != 4096 + 2 * 0x40000ul) { printf("FALLO: tamaño shm\n"); return -1; }
    if (lo.cmdq_offset != 4096 || lo.msgq_offset != 4096 + 0x40000ul) {
        printf("FALLO: offsets de las colas\n"); return -1;
    }
    /* Cada PTE de la shm describe su propia página. */
    const uint64_t *ptes = lo.shm.va;
    for (unsigned i = 0; i < lo.shm_ptes_nr; i++) {
        if (ptes[i] != lo.shm.phys + ((uint64_t)i << 12)) { printf("FALLO: PTE shm %u\n", i); return -1; }
    }
    /* "LOGINIT" empaquetado big-endian en un u64. */
    const struct gsp_libos_region *r = lo.libos.va;
    if (r[0].id8 != 0x4c4f47494e4954ull) {
        printf("FALLO: id8 LOGINIT = 0x%llx\n", (unsigned long long)r[0].id8);
        return -1;
    }
    if (r[3].pa != lo.rmargs.phys) { printf("FALLO: región RMARGS\n"); return -1; }
    const struct gsp_fmc_boot_params *p = lo.boot_params.va;
    if (p->bootGspRmParams.gspRmDescOffset != wpr->meta_phys ||
        p->gspRmParams.bootArgsOffset != lo.libos.phys) {
        printf("FALLO: boot params no enlazan\n"); return -1;
    }
    printf("OK: libos 4 regiones, colas 2x256 KiB (%u PTEs), boot params enlazados\n",
           lo.shm_ptes_nr);

    /* Staging del FMC: parseo del ELF firmado + copias a memoria DMA. */
    if (!fmc || fmc_lx_parse(fmc->data, fmc->len, &img) != 0) {
        printf("FALLO: fmc_lx_parse\n"); return -1;
    }
    if (fmc_lx_verify_sizes(&img) != 0) { printf("FALLO: tamaños COT gb20x\n"); return -1; }
    if (fmc_lx_stage(&img, &staged) != 0) { printf("FALLO: fmc_lx_stage\n"); return -1; }
    if (staged.img.size != img.image_len || staged.hash.size != 48 ||
        staged.pkey.size != 97 || staged.sig.size != 96) {
        printf("FALLO: tamaños stageados\n"); return -1;
    }
    if (memcmp(staged.img.va, img.image, img.image_len)) {
        printf("FALLO: la imagen FMC copiada no coincide\n"); return -1;
    }
    printf("OK: FMC stageado (imagen %lu B + 48/97/96)\n", staged.img.size);

    fmc_lx_stage_release(&staged);
    gsp_libos_release(&lo);
    if (lo.shm.va || lo.boot_params.va || staged.img.va) {
        printf("FALLO: release deja punteros\n"); return -1;
    }
    printf("OK: release limpio\n");
    return 0;
}

/* Deja un mensaje de GSP-RM en la página `idx` del anillo de la cola. */
static void fake_rpc_post(const struct gsp_libos *lo, unsigned idx, uint32_t fn,
                          uint32_t result)
{
    unsigned char *msgq = (unsigned char *)lo->shm.va + lo->msgq_offset;
    unsigned char *entry = msgq + 4096 + (unsigned long)idx * 4096;
    struct gsp_rpc_hdr *hdr = (struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));

    memset(entry, 0, 4096);
    hdr->length = sizeof(struct gsp_rpc_hdr);
    hdr->function = fn;
    hdr->rpc_result = result;
}

/* Recepción de RPCs de GSP-RM sobre las colas del paso 5. */
static int check_rpc(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: gsp_rpc_init\n"); return -1; }
    if (rpc.cnt != 63) { printf("FALLO: cnt=%u\n", rpc.cnt); return -1; }
    if (gsp_rpc_start(0) != 0) { printf("FALLO: gsp_rpc_start\n"); return -1; }

    /* Un evento cualquiera por delante del que esperamos: debe consumirlo. */
    fake_rpc_post(lo, 0, NV_VGPU_MSG_EVENT_UCODE_LIBOS_PRINT, 0);
    fake_rpc_post(lo, 1, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 0);
    msgq->tx.writePtr = 2;

    if (gsp_rpc_wait_event(&rpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 100) != 0) {
        printf("FALLO: no vio GSP_INIT_DONE\n");
        return -1;
    }
    if (*rpc.rptr != 2) { printf("FALLO: rptr=%u tras consumir 2\n", *rpc.rptr); return -1; }
    printf("OK: RPC consume el evento previo y ve GSP_INIT_DONE (rptr=%u)\n", *rpc.rptr);

    /* Un rpc_result distinto de cero tiene que salir como error, no colarse. */
    fake_rpc_post(lo, 2, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 0x55);
    msgq->tx.writePtr = 3;
    if (gsp_rpc_wait_event(&rpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 100) == 0) {
        printf("FALLO: un RPC con error se dio por bueno\n");
        return -1;
    }
    printf("OK: un RPC con rpc_result != 0 se detecta\n");

    /* Y sin mensajes, el tiempo se agota en vez de inventarse uno. */
    if (gsp_rpc_wait_event(&rpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 5) == 0) {
        printf("FALLO: dio por recibido un mensaje que no existe\n");
        return -1;
    }
    printf("OK: sin mensajes, expira\n");
    return 0;
}

/* Paso 6: el paquete COT, contra un FSP simulado. */
static int check_cot(const struct gsp_wpr *wpr)
{
    struct gsp_libos lo;
    struct fmc_image img;
    struct fmc_staged staged;
    const struct gsp_fw_blob *fmc = gsp_fw_get(GSP_FW_FMC);

    if (gsp_libos_prepare(wpr, &lo) != 0 || !fmc ||
        fmc_lx_parse(fmc->data, fmc->len, &img) != 0 ||
        fmc_lx_stage(&img, &staged) != 0) {
        printf("FALLO: preparando el payload del COT\n");
        return -1;
    }

    fake_fsp_reset();
    if (fsp_lx_boot_gsp_fmc(&staged, &lo, wpr) != 0) {
        printf("FALLO: fsp_lx_boot_gsp_fmc\n");
        return -1;
    }
    /* 868 B = 2 DWORD de cabeceras + 860 de payload COT. */
    if (fsp_sent_dwords != 868u / 4u) {
        printf("FALLO: se enviaron %u DWORDs, esperados %u\n", fsp_sent_dwords, 868u / 4u);
        return -1;
    }
    if (fsp_sent[0] != 0xc0000000u) { printf("FALLO: MCTP 0x%08x\n", fsp_sent[0]); return -1; }
    if (fsp_sent[1] != 0x1410de7eu) { printf("FALLO: NVDM 0x%08x\n", fsp_sent[1]); return -1; }

    const unsigned char *cot = (const unsigned char *)&fsp_sent[2];
    uint16_t version, size;
    uint64_t fmc_off, args_off, frts_off;
    uint32_t frts_size;
    memcpy(&version, cot + 0, 2);
    memcpy(&size, cot + 2, 2);
    memcpy(&fmc_off, cot + 4, 8);
    memcpy(&frts_off, cot + 24, 8);
    memcpy(&frts_size, cot + 32, 4);
    memcpy(&args_off, cot + 852, 8);

    if (version != 2 || size != 860) {
        printf("FALLO: COT v%u size %u (esperado v2/860)\n", version, size);
        return -1;
    }
    if (fmc_off != staged.img.phys) { printf("FALLO: gspFmcSysmemOffset\n"); return -1; }
    if (args_off != lo.boot_params.phys) { printf("FALLO: gspBootArgsSysmemOffset\n"); return -1; }
    if (frts_off != wpr->rsvd_size || frts_size != 0x100000u) {
        printf("FALLO: FRTS %llu/%u\n", (unsigned long long)frts_off, frts_size);
        return -1;
    }
    /* La cadena de firma va en su sitio y solo ocupa lo suyo: el resto de los
     * campos de 384 B (que gh100 sí llena) tiene que seguir a cero. */
    if (memcmp(cot + 36, staged.hash.va, 48) || memcmp(cot + 84, staged.pkey.va, 97) ||
        memcmp(cot + 468, staged.sig.va, 96)) {
        printf("FALLO: la cadena de firma no está en su offset\n");
        return -1;
    }
    for (unsigned i = 84 + 97; i < 468; i++) {
        if (cot[i]) { printf("FALLO: relleno de publicKey no nulo en %u\n", i); return -1; }
    }
    /* frtsSysmemOffset/Size se quedan a cero: la FRTS va en VRAM. */
    for (unsigned i = 12; i < 24; i++) {
        if (cot[i]) { printf("FALLO: frtsSysmem no nulo\n"); return -1; }
    }
    printf("OK: COT 868 B, cabeceras y firma en su sitio, FSP simulado lo acepta\n");

    /* Y que un rechazo del FSP se detecte en vez de darse por bueno. */
    fake_fsp_reset();
    fsp_reply_error = 0x1234u;
    if (fsp_lx_boot_gsp_fmc(&staged, &lo, wpr) == 0) {
        printf("FALLO: un COT rechazado se dio por bueno\n");
        return -1;
    }
    fsp_reply_error = 0;
    printf("OK: el rechazo del FSP se detecta\n");

    if (check_rpc(&lo) != 0)
        return -1;

    fmc_lx_stage_release(&staged);
    gsp_libos_release(&lo);
    return 0;
}

int main(int argc, char **argv)
{
    if (argc < 4) {
        fprintf(stderr, "uso: %s <gsp-…bin> <bootloader-…bin> <fmc-…bin>\n", argv[0]);
        return 2;
    }
    if (load_blob(GSP_FW_UCODE, argv[1]) || load_blob(GSP_FW_BOOTLOADER, argv[2]) ||
        load_blob(GSP_FW_FMC, argv[3]))
        return 2;

    struct gsp_rm_fw rm;
    struct gsp_wpr wpr;
    if (check_radix3(&rm) != 0)
        return 1;
    if (check_wpr(&rm, &wpr) != 0)
        return 1;
    if (check_libos(&wpr) != 0)
        return 1;
    if (check_cot(&wpr) != 0)
        return 1;
    gsp_wpr_release(&wpr);
    gsp_rm_release(&rm);
    return 0;
}
