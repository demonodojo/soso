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

/* El apagado quita el bus master; aquí solo se cuenta que lo pida. */
static unsigned pci_master_cleared;
static void lx_pci_clear_master(struct lx_pci_dev *d) { (void)d; pci_master_cleared++; }
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
static int fake_unload_pending;    /* !=0 → MAILBOX0 acaba dando 0x80000000 */

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
    /* El bootrom tarda unas vueltas en dejar leer el falcon. Tras el aviso de
     * descarga, en cambio, MAILBOX0 tiene que acabar valiendo 0x80000000: el
     * banco lo simula con unas vueltas de retardo para que el sondeo de
     * `gsp_fini` se ejercite de verdad y no acierte en la primera lectura. */
    case R_MBOX0:
        if (fake_unload_pending) {
            return fsp_mbox_reads++ < 3 ? 0u : 0x80000000u;
        }
        return fsp_mbox_reads++ < 3 ? 0xbadf4100u : 0u;
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

#include "nvrm_r570.h"

#include "gsp_dma_body.inc"
#include "gsp_rm_body.inc"
#include "gsp_wpr_body.inc"
#include "gsp_libos_body.inc"
#include "fmc_lx_body.inc"
#include "fsp_lx_body.inc"
#include "gsp_rpc_body.inc"
#include "gsp_cmdq_body.inc"
#include "gsp_rm_obj_body.inc"
#include "gsp_vram_body.inc"
#include "gsp_vmm_body.inc"
#include "gsp_chan_body.inc"
#include "gsp_ce_body.inc"
#include "gsp_compute_body.inc"
#include "gsp_fini_body.inc"

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

/* Deja un mensaje de GSP-RM en la página `idx` del anillo, con `plen` bytes de
 * payload detrás de las cabeceras. Escribe dando la vuelta al anillo igual que
 * lo haría el firmware, para poder ejercitar ese caso. */
static void fake_rpc_post_payload(const struct gsp_libos *lo, unsigned idx,
                                  uint32_t fn, uint32_t result,
                                  const unsigned char *payload, uint32_t plen)
{
    unsigned char *entries = (unsigned char *)lo->shm.va + lo->msgq_offset + 4096;
    unsigned long ring = 63ul * 4096ul;
    unsigned long pos = (unsigned long)idx * 4096ul;
    struct gsp_rpc_hdr hdr;
    unsigned char zero[4096];
    uint32_t total = (uint32_t)sizeof(struct gsp_msg_elem) + sizeof(hdr) + plen;
    uint32_t pages = (total + 4095u) / 4096u;
    unsigned p;
    unsigned long w;
    uint32_t i;

    memset(zero, 0, sizeof(zero));
    for (p = 0; p < pages; p++) {
        memcpy(entries + ((pos + (unsigned long)p * 4096ul) % ring), zero, 4096);
    }

    memset(&hdr, 0, sizeof(hdr));
    hdr.length = (uint32_t)sizeof(hdr) + plen;
    hdr.function = fn;
    hdr.rpc_result = result;

    /* Byte a byte para no tener que partir cada memcpy en el borde del anillo. */
    w = (pos + sizeof(struct gsp_msg_elem)) % ring;
    for (i = 0; i < sizeof(hdr); i++) {
        entries[(w + i) % ring] = ((const unsigned char *)&hdr)[i];
    }
    w = (w + sizeof(hdr)) % ring;
    for (i = 0; i < plen; i++) {
        entries[(w + i) % ring] = payload ? payload[i] : 0;
    }
}

static void fake_rpc_post(const struct gsp_libos *lo, unsigned idx, uint32_t fn,
                          uint32_t result)
{
    fake_rpc_post_payload(lo, idx, fn, result, NULL, 0);
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

/* Llamada síncrona (G4b): payload de respuesta y anillo circular.
 *
 * Estos dos casos no los tocaba nada: `GSP_INIT_DONE` mide 32 B y no lleva
 * payload, así que ni la copia ni el borde del anillo se ejercitaban. Una
 * respuesta de RM sí puede ocupar varias páginas y envolver. */
static int check_rpc_sync(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_cmdq q;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);
    unsigned char pattern[6000];
    unsigned char got[6000];
    uint32_t plen = 0;
    uint32_t status = 0xdeadbeefu;
    uint32_t i;

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: rpc_init (sync)\n"); return -1; }
    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: cmdq_init (sync)\n"); return -1; }

    for (i = 0; i < sizeof(pattern); i++) {
        pattern[i] = (unsigned char)(i * 7u + 3u);
    }

    /* 1) Payload corto, sin envolver: tiene que llegar entero. */
    *rpc.rptr = 10;
    fake_rpc_post_payload(lo, 10, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 0, pattern, 64);
    msgq->tx.writePtr = 11;
    memset(got, 0, sizeof(got));
    if (gsp_rpc_recv(&rpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, got, sizeof(got),
                     &plen, &status, 100) != 0) {
        printf("FALLO: recv con payload\n");
        return -1;
    }
    if (plen != 64 || status != 0 || memcmp(got, pattern, 64) != 0) {
        printf("FALLO: payload corto (len=%u status=0x%x)\n", plen, status);
        return -1;
    }
    printf("OK: recv copia el payload (%u B) y expone el status\n", plen);

    /* 2) El caso que importa: mensaje de 2 páginas que empieza en la última
     * entrada del anillo y continúa en la primera. */
    *rpc.rptr = 62;
    fake_rpc_post_payload(lo, 62, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 0, pattern,
                          (uint32_t)sizeof(pattern));
    msgq->tx.writePtr = 1;
    memset(got, 0, sizeof(got));
    if (gsp_rpc_recv(&rpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, got, sizeof(got),
                     &plen, &status, 100) != 0) {
        printf("FALLO: recv de un mensaje que envuelve\n");
        return -1;
    }
    if (plen != sizeof(pattern) || memcmp(got, pattern, sizeof(pattern)) != 0) {
        printf("FALLO: payload envuelto corrupto (len=%u)\n", plen);
        for (i = 0; i < sizeof(pattern); i++) {
            if (got[i] != pattern[i]) {
                printf("       primer byte mal: %u (0x%02x != 0x%02x)\n",
                       i, got[i], pattern[i]);
                break;
            }
        }
        return -1;
    }
    if (*rpc.rptr != 1) { printf("FALLO: rptr=%u tras envolver\n", *rpc.rptr); return -1; }
    printf("OK: mensaje de 2 páginas que envuelve el anillo, %u B intactos (rptr=%u)\n",
           plen, *rpc.rptr);

    /* 3) La llamada completa: encola y consume su respuesta. */
    fake_rpc_post_payload(lo, 1, 72u, 0, pattern, 128);
    msgq->tx.writePtr = 2;
    memset(got, 0, sizeof(got));
    if (gsp_cmdq_call(&q, &rpc, 72u, NULL, 0, got, sizeof(got), &plen,
                      &status, 100) != 0) {
        printf("FALLO: gsp_cmdq_call\n");
        return -1;
    }
    if (plen != 128 || memcmp(got, pattern, 128) != 0) {
        printf("FALLO: respuesta de la llamada (len=%u)\n", plen);
        return -1;
    }
    printf("OK: gsp_cmdq_call encola y recoge su respuesta (%u B)\n", plen);

    /* 4) Un NV_STATUS de error tiene que salir como fallo, pero legible. */
    *rpc.rptr = 5;
    fake_rpc_post_payload(lo, 5, 72u, 0x56u, NULL, 0);
    msgq->tx.writePtr = 6;
    status = 0;
    if (gsp_cmdq_call(&q, &rpc, 72u, NULL, 0, NULL, 0, NULL, &status, 100) == 0) {
        printf("FALLO: una respuesta con NV_STATUS != 0 se dio por buena\n");
        return -1;
    }
    if (status != 0x56u) {
        printf("FALLO: status no propagado (0x%x)\n", status);
        return -1;
    }
    printf("OK: NV_STATUS de error se propaga (0x%x = NOT_SUPPORTED)\n", status);
    return 0;
}

/* G4c: la cadena cliente → device → subdevice.
 *
 * El banco es de un solo hilo, así que no puede "reaccionar" a cada push: se
 * pre-encolan las tres respuestas y las tres llamadas las consumen en orden. Lo
 * que de verdad se valida son las **peticiones** que quedan en la cmdq, que es
 * lo que verá RM: clase, padre y tamaño de parámetros de cada una. */
static int check_rm_objects(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_cmdq q;
    struct gsp_rm rm;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);
    unsigned char *cmdq_base = (unsigned char *)lo->shm.va + lo->cmdq_offset;
    rpc_gsp_rm_alloc ok;
    uint32_t base;
    uint32_t wptr0;
    unsigned i;
    /* Lo que debe pedir, en orden: clase, padre y tamaño de params. */
    const struct { uint32_t cls; uint32_t parent; uint32_t psize; const char *name; } want[3] = {
        { NV01_ROOT,        NVKM_RM_CLIENT(0), (uint32_t)sizeof(NV0000_ALLOC_PARAMETERS), "cliente" },
        { NV01_DEVICE_0,    NVKM_RM_CLIENT(0), (uint32_t)sizeof(NV0080_ALLOC_PARAMETERS), "device" },
        { NV20_SUBDEVICE_0, NVKM_RM_DEVICE,    (uint32_t)sizeof(NV2080_ALLOC_PARAMETERS), "subdevice" },
    };

    printf("sizeof rpc_gsp_rm_alloc=%zu rm_control=%zu NV0000=%zu NV0080=%zu NV2080=%zu\n",
           sizeof(rpc_gsp_rm_alloc), sizeof(rpc_gsp_rm_control),
           sizeof(NV0000_ALLOC_PARAMETERS), sizeof(NV0080_ALLOC_PARAMETERS),
           sizeof(NV2080_ALLOC_PARAMETERS));

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: rpc_init (rm)\n"); return -1; }
    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: cmdq_init (rm)\n"); return -1; }

    /* Tres respuestas OK seguidas, a partir de donde esté el puntero. */
    memset(&ok, 0, sizeof(ok));
    ok.status = 0;
    base = *rpc.rptr;
    for (i = 0; i < 3; i++) {
        fake_rpc_post_payload(lo, (base + i) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                              0, (const unsigned char *)&ok, (uint32_t)sizeof(ok));
    }
    msgq->tx.writePtr = (base + 3) % 63;

    wptr0 = *q.wptr;
    if (gsp_rm_init(&q, &rpc, &rm) != 0) {
        printf("FALLO: gsp_rm_init\n");
        return -1;
    }
    if (!rm.ready || rm.client != NVKM_RM_CLIENT(0) || rm.device != NVKM_RM_DEVICE ||
        rm.subdevice != NVKM_RM_SUBDEVICE) {
        printf("FALLO: handles cli=0x%08x dev=0x%08x sub=0x%08x\n",
               rm.client, rm.device, rm.subdevice);
        return -1;
    }

    /* Releer las tres peticiones encoladas y comprobarlas una a una. */
    for (i = 0; i < 3; i++) {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + i) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_alloc *a = (const rpc_gsp_rm_alloc *)(hdr + 1);

        if (hdr->function != NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC) {
            printf("FALLO: %s con function=%u\n", want[i].name, hdr->function);
            return -1;
        }
        if (a->hClass != want[i].cls) {
            printf("FALLO: %s hClass=0x%x (esperaba 0x%x)\n",
                   want[i].name, a->hClass, want[i].cls);
            return -1;
        }
        if (a->hParent != want[i].parent) {
            printf("FALLO: %s hParent=0x%08x (esperaba 0x%08x)\n",
                   want[i].name, a->hParent, want[i].parent);
            return -1;
        }
        if (a->paramsSize != want[i].psize) {
            printf("FALLO: %s paramsSize=%u (esperaba %u)\n",
                   want[i].name, a->paramsSize, want[i].psize);
            return -1;
        }
        if (a->hClient != NVKM_RM_CLIENT(0)) {
            printf("FALLO: %s hClient=0x%08x\n", want[i].name, a->hClient);
            return -1;
        }
        if (hdr->length != sizeof(*hdr) + sizeof(*a) + want[i].psize) {
            printf("FALLO: %s length=%u\n", want[i].name, hdr->length);
            return -1;
        }
    }
    /* El cliente lleva su propio handle dentro de los parámetros y processID=~0. */
    {
        const unsigned char *entry = cmdq_base + 4096 + (unsigned long)(wptr0 % 63) * 4096;
        const rpc_gsp_rm_alloc *a = (const rpc_gsp_rm_alloc *)
            (entry + sizeof(struct gsp_msg_elem) + sizeof(struct gsp_rpc_hdr));
        const NV0000_ALLOC_PARAMETERS *p = (const NV0000_ALLOC_PARAMETERS *)(a + 1);

        if (p->hClient != NVKM_RM_CLIENT(0) || p->processID != 0xffffffffu) {
            printf("FALLO: params del cliente hClient=0x%08x pid=0x%x\n",
                   p->hClient, p->processID);
            return -1;
        }
    }
    printf("OK: cadena RM cliente→device→subdevice (clases 0x0/0x80/0x2080, "
           "params %u/%u/%u B)\n", want[0].psize, want[1].psize, want[2].psize);

    /* Y un NV_STATUS dentro del wrapper tiene que salir como fallo — es el que
     * llega con el RPC impecable y un error de RM dentro. */
    memset(&ok, 0, sizeof(ok));
    ok.status = 0x2bu;   /* INVALID_CLASS */
    base = *rpc.rptr;
    fake_rpc_post_payload(lo, base % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC, 0,
                          (const unsigned char *)&ok, (uint32_t)sizeof(ok));
    msgq->tx.writePtr = (base + 1) % 63;
    {
        uint32_t st = 0;
        NV2080_ALLOC_PARAMETERS sub;

        memset(&sub, 0, sizeof(sub));
        if (gsp_rm_alloc(&rm, rm.device, 0x5d1d0001u, NV20_SUBDEVICE_0, &sub,
                         (uint32_t)sizeof(sub), &st) == 0) {
            printf("FALLO: un NV_STATUS de RM se dio por bueno\n");
            return -1;
        }
        if (st != 0x2bu) {
            printf("FALLO: NV_STATUS del wrapper no propagado (0x%x)\n", st);
            return -1;
        }
    }
    printf("OK: el NV_STATUS de dentro del wrapper se detecta (0x2b = INVALID_CLASS)\n");

    /* G4d: GET_GSP_STATIC_INFO. Se fabrica una respuesta con VRAM y nombre
     * conocidos y se comprueba que los campos se leen de donde deben — que es
     * justo lo que no se puede dar por hecho en una transcripción a mano. */
    {
        GspStaticConfigInfo *fake = calloc(1, sizeof(*fake));
        struct gsp_static_info si;
        const uint64_t vram = 12227ull * 1024ull * 1024ull;

        if (!fake) { printf("FALLO: sin memoria\n"); return -1; }
        printf("sizeof(GspStaticConfigInfo) = %zu\n", sizeof(*fake));

        fake->fb_length = vram;
        fake->l2_cache_size = 32u * 1024u;
        fake->bar1PdeBase = 0x1234000ull;
        fake->bar2PdeBase = 0x5678000ull;
        fake->hInternalClient = 0xc1d00001u;
        fake->hInternalDevice = 0xde1d0001u;
        fake->hInternalSubdevice = 0x5d1d0001u;
        memcpy(fake->gpuNameString, "NVIDIA GeForce RTX 5070", 24);
        /* Dos regiones: una utilizable y una reservada que debe descartarse. */
        fake->fbRegionInfoParams.numFBRegions = 2;
        fake->fbRegionInfoParams.fbRegion[0].base = 0;
        fake->fbRegionInfoParams.fbRegion[0].limit = (2048ull << 20) - 1;
        fake->fbRegionInfoParams.fbRegion[1].base = 2048ull << 20;
        fake->fbRegionInfoParams.fbRegion[1].limit = (2049ull << 20) - 1;
        fake->fbRegionInfoParams.fbRegion[1].reserved = 1;

        base = *rpc.rptr;
        fake_rpc_post_payload(lo, base % 63, NV_VGPU_MSG_FUNCTION_GET_GSP_STATIC_INFO,
                              0, (const unsigned char *)fake, (uint32_t)sizeof(*fake));
        msgq->tx.writePtr = (base + ((sizeof(*fake) + 80 + 4095) / 4096)) % 63;

        wptr0 = *q.wptr;
        if (gsp_static_info_get(&rm, vram, &si) != 0) {
            printf("FALLO: gsp_static_info_get\n");
            free(fake);
            return -1;
        }

        /* La petición NO puede ir pelada. Upstream la manda con el struct
         * entero de payload (`nvkm_gsp_rpc_rd(gsp, fn, sizeof(*rpc))` →
         * `rpc->length = 32 + 1656`), y RM lo usa de hueco para la respuesta.
         * Mandándola con length=32 el transporte la rechaza con
         * `0xff100002 = RPC_INVALID_MESSAGE_FORMAT`, que es lo que pasó en el
         * HW el 2026-07-25 y dejó G4d bloqueado. */
        {
            const unsigned char *entry = cmdq_base + 4096 +
                                         (unsigned long)(wptr0 % 63) * 4096;
            const struct gsp_rpc_hdr *hdr =
                (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
            uint32_t want = (uint32_t)(sizeof(*hdr) + sizeof(*fake));

            if (hdr->function != NV_VGPU_MSG_FUNCTION_GET_GSP_STATIC_INFO) {
                printf("FALLO: static info pedido con function=%u\n", hdr->function);
                free(fake);
                return -1;
            }
            if (hdr->length != want) {
                printf("FALLO: static info pedido con length=%u, esperaba %u "
                       "(pelado = 0xff100002 RPC_INVALID_MESSAGE_FORMAT)\n",
                       hdr->length, want);
                free(fake);
                return -1;
            }
            printf("OK: GET_GSP_STATIC_INFO se pide con %u B (cabecera + hueco "
                   "para la respuesta), no pelado\n", hdr->length);
        }
        if (si.fb_length != vram || si.region_nr != 1 ||
            si.usable_bytes != (2048ull << 20) ||
            si.bar1_pde_base != 0x1234000ull || si.bar2_pde_base != 0x5678000ull ||
            si.internal_client != 0xc1d00001u || si.internal_subdevice != 0x5d1d0001u ||
            strcmp(si.name, "NVIDIA GeForce RTX 5070") != 0) {
            printf("FALLO: campos mal leídos — vram=0x%llx regs=%u usable=0x%llx "
                   "bar1=0x%llx cli=0x%08x name='%s'\n",
                   (unsigned long long)si.fb_length, si.region_nr,
                   (unsigned long long)si.usable_bytes,
                   (unsigned long long)si.bar1_pde_base, si.internal_client, si.name);
            free(fake);
            return -1;
        }
        printf("OK: static info — '%s' %llu MiB, 1 de 2 regiones utilizable, "
               "handles y bases de PDE en su offset\n",
               si.name, (unsigned long long)(si.fb_length >> 20));

        /* Y el contraste: si la VRAM no cuadra, hay que rechazarlo, no seguir. */
        fake->fb_length = vram + 4096;
        base = *rpc.rptr;
        fake_rpc_post_payload(lo, base % 63, NV_VGPU_MSG_FUNCTION_GET_GSP_STATIC_INFO,
                              0, (const unsigned char *)fake, (uint32_t)sizeof(*fake));
        msgq->tx.writePtr = (base + ((sizeof(*fake) + 80 + 4095) / 4096)) % 63;
        if (gsp_static_info_get(&rm, vram, &si) == 0) {
            printf("FALLO: una fb_length que no cuadra se dio por buena\n");
            free(fake);
            return -1;
        }
        printf("OK: fb_length que no cuadra con la VRAM conocida se rechaza\n");
        free(fake);
    }
    return 0;
}

/* Envío por la cmdq: las dos RPCs que GSP-RM consume durante su init. */
/* G4: el apagado (`gsp_fini`).
 *
 * Lo que se valida son las **peticiones** que quedan en la cmdq, que es lo que
 * vería GSP-RM: tres FREE en orden inverso al de la reserva y el aviso de
 * descarga. Importa especialmente el número de función: `FREE` valía 27 en este
 * árbol (= `DMA_FILL_PTE_MEM`), y con el 27 esta prueba fallaría en el primer
 * `hdr->function`. Y el bus master tiene que quedar quitado pase lo que pase,
 * que es lo único que de verdad protegía al host el día del cuelgue. */
/* G4d (2/2): el vaspace, el directorio de páginas y la codificación VER3.
 *
 * Lo que aporta este banco sobre el `gsp_vmm_translate` que ya corre en el
 * arranque: `translate` recorre las tablas con las mismas rutinas que las
 * escribió, así que un PTE mal codificado —el bit que sobra, la apertura del
 * PDE puesta con los valores del PTE— le cuadraría igual. Aquí los valores
 * están escritos a mano, sacados de dev_mmu.h campo a campo. */
#define VMM_T_VA        0x0000010000000000ull   /* la misma que usa el bring-up */
#define VMM_T_VRAM_SZ   (2ull * 1024ull * 1024ull)
#define VMM_T_VRAM_PA   0x0000000240000000ull   /* 9 GiB, alineado a 2 MiB */

/* PTE de 4 KiB: VALID(bit 0) | APERTURE(2:1) | PCF(7:3) | KIND(11:8) | ADDR(51:12).
 * VRAM   → aper 0, pcf 0x10 (REGULAR_RW_ATOMIC_CACHED_ACD)   → 0x81
 * sysmem → aper 2, pcf 0x11 (REGULAR_RW_ATOMIC_UNCACHED_ACD) → 0x8d */
#define VMM_T_PTE_VRAM_LOW   0x81ull
#define VMM_T_PTE_SYS_LOW    0x8dull
/* PDE: **bit 0 a cero** (ahí vive IS_PTE, no VALID) | APERTURE(2:1) | PCF(5:3).
 * sysmem coherente → aper 2, pcf 1 (VALID_UNCACHED_ATS_ALLOWED) → 0xc */
#define VMM_T_PDE_SYS_LOW    0x0cull
#define VMM_T_ADDR_MASK      0x000ffffffffff000ull

static int check_vmm(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_cmdq q;
    struct gsp_vmm v;
    struct gsp_vram pool;
    struct gsp_static_info si;
    struct gsp_dma_buf scratch;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);
    unsigned char *cmdq_base = (unsigned char *)lo->shm.va + lo->cmdq_offset;
    rpc_gsp_rm_alloc alloc_ok;
    unsigned char ctrl_ok[sizeof(rpc_gsp_rm_control)];
    uint32_t base, wptr0;
    uint64_t root_phys, phys = 0, pte = 0;
    unsigned i;

    printf("sizeof NV_VASPACE_ALLOCATION_PARAMETERS=%zu SET_PAGE_DIRECTORY=%zu\n",
           sizeof(NV_VASPACE_ALLOCATION_PARAMETERS),
           sizeof(NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS));

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: rpc_init (vmm)\n"); return -1; }
    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: cmdq_init (vmm)\n"); return -1; }

    /* Cuatro RM_ALLOC (cliente, device, subdevice, vaspace) y un RM_CONTROL. */
    memset(&alloc_ok, 0, sizeof(alloc_ok));
    memset(ctrl_ok, 0, sizeof(ctrl_ok));
    base = *rpc.rptr;
    for (i = 0; i < 4; i++) {
        fake_rpc_post_payload(lo, (base + i) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                              0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    }
    fake_rpc_post_payload(lo, (base + 4) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    msgq->tx.writePtr = (base + 5) % 63;

    wptr0 = *q.wptr;
    if (gsp_vmm_init(&q, &rpc, &v) != 0) {
        printf("FALLO: gsp_vmm_init\n");
        return -1;
    }
    if (!v.ready || !v.bound || v.vaspace != NVKM_RM_VASPACE) {
        printf("FALLO: vmm ready=%d bound=%d vaspace=0x%08x\n",
               v.ready, v.bound, v.vaspace);
        return -1;
    }
    /* Cliente propio, como en upstream — y el device puede repetir handle
     * porque los handles se cuentan por cliente. */
    if (v.rm.client != NVKM_RM_CLIENT(1) || v.rm.device != NVKM_RM_DEVICE) {
        printf("FALLO: el vaspace no cuelga de un cliente propio "
               "(cli=0x%08x dev=0x%08x)\n", v.rm.client, v.rm.device);
        return -1;
    }

    /* La cuarta petición encolada es la del vaspace. */
    {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 3) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_alloc *a = (const rpc_gsp_rm_alloc *)(hdr + 1);
        const NV_VASPACE_ALLOCATION_PARAMETERS *p =
            (const NV_VASPACE_ALLOCATION_PARAMETERS *)(a + 1);

        if (a->hClass != FERMI_VASPACE_A || a->hObject != NVKM_RM_VASPACE ||
            a->hParent != NVKM_RM_DEVICE || a->hClient != NVKM_RM_CLIENT(1)) {
            printf("FALLO: vaspace cls=0x%x obj=0x%08x padre=0x%08x cli=0x%08x\n",
                   a->hClass, a->hObject, a->hParent, a->hClient);
            return -1;
        }
        if (a->paramsSize != sizeof(*p)) {
            printf("FALLO: vaspace paramsSize=%u (esperaba %zu)\n",
                   a->paramsSize, sizeof(*p));
            return -1;
        }
        if (p->index != NV_VASPACE_ALLOCATION_INDEX_GPU_NEW ||
            p->flags != NV_VASPACE_ALLOCATION_FLAGS_IS_EXTERNALLY_OWNED) {
            printf("FALLO: vaspace index=%u flags=0x%x (esperaba 0/%u — sin "
                   "EXTERNALLY_OWNED las tablas las querría construir RM)\n",
                   p->index, p->flags,
                   NV_VASPACE_ALLOCATION_FLAGS_IS_EXTERNALLY_OWNED);
            return -1;
        }
        if (p->vaSize || p->vaBase || p->bigPageSize) {
            printf("FALLO: vaspace con geometría puesta (vaSize=%llu base=%llu "
                   "big=%u); upstream los deja a cero\n",
                   (unsigned long long)p->vaSize, (unsigned long long)p->vaBase,
                   p->bigPageSize);
            return -1;
        }
    }
    printf("OK: FERMI_VASPACE_A (0x%04x) externo, cliente propio 0x%08x\n",
           FERMI_VASPACE_A, NVKM_RM_CLIENT(1));

    /* Y la quinta, el control que entrega el directorio. */
    root_phys = v.pt[0].mem.phys;
    {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 4) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_control *c = (const rpc_gsp_rm_control *)(hdr + 1);
        const NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS *p =
            (const NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS *)(c + 1);

        if (hdr->function != NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL ||
            c->cmd != NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY) {
            printf("FALLO: SET_PAGE_DIRECTORY fn=%u cmd=0x%08x\n",
                   hdr->function, c->cmd);
            return -1;
        }
        if (c->hObject != NVKM_RM_DEVICE) {
            printf("FALLO: SET_PAGE_DIRECTORY sobre 0x%08x (va sobre el device)\n",
                   c->hObject);
            return -1;
        }
        if (p->physAddress != root_phys) {
            printf("FALLO: directorio en 0x%llx, la raíz está en 0x%llx\n",
                   (unsigned long long)p->physAddress,
                   (unsigned long long)root_phys);
            return -1;
        }
        if (p->numEntries != 2) {
            printf("FALLO: numEntries=%u; la raíz VER3 indexa con UN bit de la "
                   "VA, son 2 entradas y no 512\n", p->numEntries);
            return -1;
        }
        if (p->flags != NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_FLAGS_APERTURE_SYSMEM_COH) {
            printf("FALLO: aperture=%u; nuestro directorio vive en sysmem, no en "
                   "VRAM como el de upstream\n", p->flags);
            return -1;
        }
        if (p->hVASpace != NVKM_RM_VASPACE) {
            printf("FALLO: hVASpace=0x%08x\n", p->hVASpace);
            return -1;
        }
    }
    printf("OK: SET_PAGE_DIRECTORY raíz=0x%llx 2 entradas en sysmem coherente\n",
           (unsigned long long)root_phys);

    /* Mapear lo mismo que el bring-up y releerlo. */
    if (gsp_dma_alloc(&scratch, 4096, "scratch") != 0) {
        printf("FALLO: scratch\n");
        return -1;
    }
    if (gsp_vmm_map(&v, VMM_T_VA, VMM_T_VRAM_PA, VMM_T_VRAM_SZ, GSP_VMM_VRAM) != 0 ||
        gsp_vmm_map(&v, VMM_T_VA + VMM_T_VRAM_SZ, scratch.phys, 4096,
                    GSP_VMM_SYSMEM) != 0) {
        printf("FALLO: gsp_vmm_map\n");
        return -1;
    }
    if (v.pages_mapped != 512 + 1) {
        printf("FALLO: %u páginas mapeadas (esperaba 513)\n", v.pages_mapped);
        return -1;
    }
    /* Raíz + 4 niveles intermedios + 2 hojas: la segunda hoja aparece porque la
     * página de sysmem cae justo en el siguiente tramo de 2 MiB. */
    if (v.pt_nr != 7) {
        printf("FALLO: %u tablas (esperaba 7: raíz, 4 niveles y 2 hojas)\n", v.pt_nr);
        return -1;
    }

    for (i = 0; i < 512; i++) {
        uint64_t at = VMM_T_VA + (uint64_t)i * 4096ull;

        if (gsp_vmm_translate(&v, at, &phys, &pte) != 0) {
            printf("FALLO: VA 0x%llx no traduce\n", (unsigned long long)at);
            return -1;
        }
        if (phys != VMM_T_VRAM_PA + (uint64_t)i * 4096ull) {
            printf("FALLO: VA 0x%llx → 0x%llx\n", (unsigned long long)at,
                   (unsigned long long)phys);
            return -1;
        }
        if ((pte & ~VMM_T_ADDR_MASK) != VMM_T_PTE_VRAM_LOW) {
            printf("FALLO: PTE de VRAM con banderas 0x%llx (esperaba 0x%llx)\n",
                   (unsigned long long)(pte & ~VMM_T_ADDR_MASK),
                   (unsigned long long)VMM_T_PTE_VRAM_LOW);
            return -1;
        }
    }
    printf("OK: 512 PTEs de VRAM, banderas 0x%llx (VALID, aper 0, PCF 0x10)\n",
           (unsigned long long)VMM_T_PTE_VRAM_LOW);

    if (gsp_vmm_translate(&v, VMM_T_VA + VMM_T_VRAM_SZ, &phys, &pte) != 0 ||
        phys != scratch.phys ||
        (pte & ~VMM_T_ADDR_MASK) != VMM_T_PTE_SYS_LOW) {
        printf("FALLO: PTE de sysmem 0x%llx → 0x%llx\n",
               (unsigned long long)pte, (unsigned long long)phys);
        return -1;
    }
    printf("OK: PTE de sysmem, banderas 0x%llx (aper 2, PCF 0x11)\n",
           (unsigned long long)VMM_T_PTE_SYS_LOW);

    /* Lo que no está mapeado tiene que fallar; si no, el recorrido no prueba nada. */
    if (gsp_vmm_translate(&v, VMM_T_VA + VMM_T_VRAM_SZ + 4096ull, &phys, &pte) == 0) {
        printf("FALLO: una VA sin mapear traduce a 0x%llx\n",
               (unsigned long long)phys);
        return -1;
    }
    if (gsp_vmm_translate(&v, VMM_T_VA - 4096ull, &phys, &pte) == 0) {
        printf("FALLO: la VA justo antes del mapeo traduce a 0x%llx\n",
               (unsigned long long)phys);
        return -1;
    }
    printf("OK: fuera del mapeo no traduce (ni por arriba ni por abajo)\n");

    /* Los PDE, uno a uno y en crudo. El de nivel 1 es doble: la mitad de páginas
     * grandes tiene que quedarse a cero. */
    for (i = 0; i < v.pt_nr; i++) {
        const struct gsp_vmm_pt *pt = &v.pt[i];
        const uint64_t *raw = (const uint64_t *)pt->mem.va;
        uint32_t idx;
        uint64_t e;

        if (pt->level == 0) {
            continue;
        }
        idx = lvl_index(pt->level, VMM_T_VA);
        if (pt->level == 1) {
            if (raw[(unsigned long)idx * 2u] != 0) {
                printf("FALLO: la mitad de páginas grandes de la PDE doble no "
                       "está a cero (0x%llx)\n",
                       (unsigned long long)raw[(unsigned long)idx * 2u]);
                return -1;
            }
            e = raw[(unsigned long)idx * 2u + 1u];
        } else {
            e = raw[idx];
        }
        if (e & 1ull) {
            printf("FALLO: PDE de nivel %u con el bit 0 puesto — eso es IS_PTE, "
                   "no VALID: la MMU lo leería como traducción final\n", pt->level);
            return -1;
        }
        if ((e & ~VMM_T_ADDR_MASK) != VMM_T_PDE_SYS_LOW) {
            printf("FALLO: PDE de nivel %u con banderas 0x%llx (esperaba 0x%llx)\n",
                   pt->level, (unsigned long long)(e & ~VMM_T_ADDR_MASK),
                   (unsigned long long)VMM_T_PDE_SYS_LOW);
            return -1;
        }
        if ((e & VMM_T_ADDR_MASK) == 0) {
            printf("FALLO: PDE de nivel %u sin dirección\n", pt->level);
            return -1;
        }
    }
    printf("OK: PDEs con bit 0 a cero, aper 2 y PCF 1; la mitad grande vacía\n");

    /* Y el reparto de VRAM: regiones que no valen fuera, y no dar dos veces lo
     * mismo. Va con una static info a mano porque la de verdad la trae RM. */
    memset(&si, 0, sizeof(si));
    si.ready = 1;
    si.region_nr = 2;
    si.region[0].base = 0;                      /* base 0: inservible a propósito */
    si.region[0].size = 0x100000ull;
    si.region[1].base = 0x200000ull;
    si.region[1].size = 0x300000ull;            /* 3 MiB */
    if (gsp_vram_init(&pool, &si) != 0 || pool.region_nr != 1 ||
        pool.total != 0x300000ull) {
        printf("FALLO: gsp_vram_init con una región inservible (nr=%u total=%llu)\n",
               pool.region_nr, (unsigned long long)pool.total);
        return -1;
    }
    {
        uint64_t a = gsp_vram_alloc(&pool, 0x100000ull, 0x100000ull);
        uint64_t b = gsp_vram_alloc(&pool, 0x100000ull, 0x100000ull);

        if (a != 0x200000ull || b != 0x300000ull || a == b) {
            printf("FALLO: reparto de VRAM a=0x%llx b=0x%llx\n",
                   (unsigned long long)a, (unsigned long long)b);
            return -1;
        }
        if (gsp_vram_alloc(&pool, 0x300000ull, 4096) != 0) {
            printf("FALLO: se repartió VRAM que no cabía\n");
            return -1;
        }
    }
    printf("OK: reparto de VRAM (descarta la región base 0 y no repite bloque)\n");

    /* El desmontaje: UNSET_PAGE_DIRECTORY y los cuatro FREE (vaspace, subdevice,
     * device, cliente). Se le dan respuestas para que no gaste los plazos. */
    base = *rpc.rptr;
    fake_rpc_post_payload(lo, base % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL, 0,
                          ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    for (i = 0; i < 4; i++) {
        fake_rpc_post_payload(lo, (base + 1 + i) % 63, NV_VGPU_MSG_FUNCTION_FREE,
                              0, NULL, 0);
    }
    msgq->tx.writePtr = (base + 5) % 63;

    wptr0 = *q.wptr;
    gsp_dma_free(&scratch);
    gsp_vmm_fini(&v);
    if (v.ready || v.bound || v.pt_nr != 0) {
        printf("FALLO: tras el fini quedan ready=%d bound=%d tablas=%u\n",
               v.ready, v.bound, v.pt_nr);
        return -1;
    }
    {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)(wptr0 % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_control *c = (const rpc_gsp_rm_control *)(hdr + 1);

        if (c->cmd != NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY) {
            printf("FALLO: el fini no quita el directorio (cmd=0x%08x)\n", c->cmd);
            return -1;
        }
    }
    printf("OK: el fini quita el directorio antes de soltar las tablas\n");
    return 0;
}

static int check_g4e_chan_ce(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_cmdq q;
    struct gsp_vmm v;
    struct gsp_chan chan;
    struct gsp_ce ce;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);
    unsigned char *cmdq_base = (unsigned char *)lo->shm.va + lo->cmdq_offset;
    rpc_gsp_rm_alloc alloc_ok;
    unsigned char ctrl_ok[sizeof(rpc_gsp_rm_control)];
    uint32_t base, wptr0;
    unsigned pb_off = 0, pb_len = 0;
    unsigned i;
    const uint32_t test_pat = 0x300u;

    printf("sizeof NV_CHANNEL_ALLOC_PARAMS=%zu Nvc56fControl=%zu\n",
           sizeof(NV_CHANNEL_ALLOC_PARAMS), sizeof(Nvc56fControl));

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: rpc_init (g4e)\n"); return -1; }
    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: cmdq_init (g4e)\n"); return -1; }

    memset(&alloc_ok, 0, sizeof(alloc_ok));
    memset(ctrl_ok, 0, sizeof(ctrl_ok));
    base = *rpc.rptr;
    for (i = 0; i < 4; i++) {
        fake_rpc_post_payload(lo, (base + i) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                              0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    }
    fake_rpc_post_payload(lo, (base + 4) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    /* Canal + CE + compute. */
    fake_rpc_post_payload(lo, (base + 5) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                          0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    fake_rpc_post_payload(lo, (base + 6) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                          0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    fake_rpc_post_payload(lo, (base + 7) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                          0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    msgq->tx.writePtr = (base + 8) % 63;

    wptr0 = *q.wptr;
    if (gsp_vmm_init(&q, &rpc, &v) != 0) {
        printf("FALLO: gsp_vmm_init (g4e)\n");
        return -1;
    }
    if (gsp_chan_init(&v.rm, &v, &chan, v.vaspace) != 0) {
        printf("FALLO: gsp_chan_init\n");
        return -1;
    }

    {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 5) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_alloc *a = (const rpc_gsp_rm_alloc *)(hdr + 1);
        const NV_CHANNEL_ALLOC_PARAMS *p =
            (const NV_CHANNEL_ALLOC_PARAMS *)(a + 1);

        if (a->hClass != AMPERE_CHANNEL_GPFIFO_A ||
            a->hObject != NVKM_RM_CHAN(0) ||
            a->hParent != NVKM_RM_DEVICE) {
            printf("FALLO: canal cls=0x%x obj=0x%08x padre=0x%08x\n",
                   a->hClass, a->hObject, a->hParent);
            return -1;
        }
        if (a->paramsSize != sizeof(*p)) {
            printf("FALLO: canal paramsSize=%u\n", a->paramsSize);
            return -1;
        }
        if (p->gpFifoEntries != GSP_CHAN_GPFIFO_ENTRIES ||
            p->hVASpace != v.vaspace ||
            p->engineType != NV2080_ENGINE_TYPE_COPY0) {
            printf("FALLO: canal entries=%u vaspace=0x%08x engine=%u\n",
                   p->gpFifoEntries, p->hVASpace, p->engineType);
            return -1;
        }
        if (p->userdMem.addressSpace != NV_ADDRESS_SPACE_SYSMEM_COHERENT) {
            printf("FALLO: userdMem aper=%u\n", p->userdMem.addressSpace);
            return -1;
        }
    }
    printf("OK: AMPERE_CHANNEL_GPFIFO_A alloc params\n");

    if (chan.userd_ctl->GPPut != 0 || chan.gpput != 0) {
        printf("FALLO: USERD/GPPut no arrancan en cero\n");
        return -1;
    }

    if (gsp_ce_init(&v.rm, &chan, &ce) != 0) {
        printf("FALLO: gsp_ce_init\n");
        return -1;
    }

    {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 6) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_alloc *a = (const rpc_gsp_rm_alloc *)(hdr + 1);

        if (a->hClass != AMPERE_DMA_COPY_A || a->hObject != NVKM_RM_CE0 ||
            a->hParent != NVKM_RM_CHAN(0)) {
            printf("FALLO: CE cls=0x%x obj=0x%08x padre=0x%08x\n",
                   a->hClass, a->hObject, a->hParent);
            return -1;
        }
    }
    printf("OK: AMPERE_DMA_COPY_A colgado del canal\n");

    if (gsp_ce_encode_copy(&ce, GSP_CHAN_VA_BASE + 8192ull,
                           GSP_CHAN_VA_BASE + 12288ull, 4096,
                           &pb_off, &pb_len) != 0 || pb_len < 32) {
        printf("FALLO: gsp_ce_encode_copy\n");
        return -1;
    }
    {
        const uint32_t *pb = (const uint32_t *)((const unsigned char *)chan.pushbuf.va + pb_off);
        unsigned found_launch = 0;

        for (i = 0; i < pb_len / 4; i++) {
            if (pb[i] == (NVC6B5_LAUNCH_DMA_DATA_TRANSFER_TYPE_NON_PIPELINED |
                          NVC6B5_LAUNCH_DMA_FLUSH_ENABLE_TRUE |
                          NVC6B5_LAUNCH_DMA_SRC_TYPE_VIRTUAL |
                          NVC6B5_LAUNCH_DMA_DST_TYPE_VIRTUAL |
                          NVC6B5_LAUNCH_DMA_SRC_MEMORY_LAYOUT_PITCH |
                          NVC6B5_LAUNCH_DMA_DST_MEMORY_LAYOUT_PITCH |
                          NVC6B5_LAUNCH_DMA_SEMAPHORE_TYPE_RELEASE_ONE_WORD)) {
                found_launch = 1;
                break;
            }
        }
        if (!found_launch) {
            printf("FALLO: pushbuffer sin LAUNCH_DMA esperado\n");
            return -1;
        }
    }
    printf("OK: pushbuffer CE (%u B) con LAUNCH_DMA\n", pb_len);

    if (gsp_chan_submit(&chan, pb_off, pb_len) != 0) {
        printf("FALLO: gsp_chan_submit\n");
        return -1;
    }
    if (chan.gpput != 1 || chan.userd_ctl->GPPut != 1) {
        printf("FALLO: GPPut=%u gpput=%u\n", chan.userd_ctl->GPPut, chan.gpput);
        return -1;
    }
    {
        const uint32_t *ring = (const uint32_t *)chan.gpfifo.va;

        if ((ring[0] & 1u) != NVC56F_GP_ENTRY0_FETCH_UNCONDITIONAL) {
            printf("FALLO: GPFIFO entry0 fetch\n");
            return -1;
        }
        if (((ring[1] >> 10) & 0x1fffffu) != ((pb_len + 3u) / 4u)) {
            printf("FALLO: GPFIFO length en words\n");
            return -1;
        }
    }
    printf("OK: GPFIFO entry + USERD GPPut\n");

    /* --- G4f: compute + QMD inline --- */
    {
        struct gsp_compute cp;
        GspQmdV05 qmd;
        unsigned qmd_off = 0, qmd_len = 0;
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 7) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_alloc *a = (const rpc_gsp_rm_alloc *)(hdr + 1);

        if (gsp_saxpy_sass_len < 16) {
            printf("FALLO: gsp_saxpy_sass_len=%u\n", gsp_saxpy_sass_len);
            return -1;
        }
        printf("OK: saxpy SASS blob %u B\n", gsp_saxpy_sass_len);

        if (gsp_compute_init(&v.rm, &chan, &cp) != 0) {
            printf("FALLO: gsp_compute_init\n");
            return -1;
        }
        if (a->hClass != BLACKWELL_COMPUTE_A || a->hObject != NVKM_RM_COMPUTE0 ||
            a->hParent != NVKM_RM_CHAN(0)) {
            printf("FALLO: compute cls=0x%x obj=0x%08x padre=0x%08x\n",
                   a->hClass, a->hObject, a->hParent);
            return -1;
        }
        printf("OK: BLACKWELL_COMPUTE_A colgado del canal\n");

        gsp_compute_fill_saxpy_qmd(&qmd, G4F_SASS_VA, 4);
        chan.pb_pos = 0;
        if (gsp_compute_encode_qmd(&cp, &qmd, &qmd_off, &qmd_len) != 0 ||
            qmd_len < 64) {
            printf("FALLO: gsp_compute_encode_qmd\n");
            return -1;
        }
        {
            const uint32_t *pb = (const uint32_t *)((const unsigned char *)chan.pushbuf.va + qmd_off);
            unsigned found_qmd_ver = 0;
            unsigned found_inline = 0;
            unsigned j;

            for (j = 0; j < qmd_len / 4; j++) {
                if (pb[j] == (GSP_QMD_VERSION_CURRENT | (GSP_QMD_VERSION_CURRENT << 16))) {
                    found_qmd_ver = 1;
                }
                if (pb[j] == qmd.words[0]) {
                    found_inline = 1;
                }
            }
            if (!found_qmd_ver || !found_inline) {
                printf("FALLO: pushbuffer compute sin QMD/version\n");
                return -1;
            }
        }
        printf("OK: pushbuffer compute QMD inline (%u B)\n", qmd_len);
        gsp_compute_fini(&cp);
    }

    /* Teardown G4e/G4f antes del vmm_fini de check_vmm (este test es autónomo). */
    base = *rpc.rptr;
    fake_rpc_post_payload(lo, base % 63, NV_VGPU_MSG_FUNCTION_FREE, 0, NULL, 0);
    fake_rpc_post_payload(lo, (base + 1) % 63, NV_VGPU_MSG_FUNCTION_FREE, 0, NULL, 0);
    fake_rpc_post_payload(lo, (base + 2) % 63, NV_VGPU_MSG_FUNCTION_FREE, 0, NULL, 0);
    fake_rpc_post_payload(lo, (base + 3) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    for (i = 0; i < 5; i++) {
        fake_rpc_post_payload(lo, (base + 4 + i) % 63, NV_VGPU_MSG_FUNCTION_FREE,
                              0, NULL, 0);
    }
    msgq->tx.writePtr = (base + 9) % 63;

    gsp_ce_fini(&ce);
    gsp_chan_fini(&chan);
    gsp_vmm_fini(&v);
    (void)test_pat;
    printf("OK: fini CE → compute → canal → vaspace\n");
    return 0;
}

static int check_fini(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_cmdq q;
    struct gsp_rm rm;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);
    unsigned char *cmdq_base = (unsigned char *)lo->shm.va + lo->cmdq_offset;
    /* Puntero no-NULL que nunca se desreferencia: el stub de clear_master lo ignora. */
    struct lx_pci_dev *fake_pdev = (struct lx_pci_dev *)(void *)&rpc;
    uint32_t base, wptr0;
    unsigned i;
    /* Orden inverso al de la reserva: el hijo antes que el padre. */
    const uint32_t want_free[3] = { NVKM_RM_SUBDEVICE, NVKM_RM_DEVICE, NVKM_RM_CLIENT(0) };

    printf("sizeof rpc_unloading_guest_driver=%zu\n",
           sizeof(rpc_unloading_guest_driver_v1F_07));

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: rpc_init (fini)\n"); return -1; }
    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: cmdq_init (fini)\n"); return -1; }

    memset(&rm, 0, sizeof(rm));
    rm.q = &q;
    rm.rpc = &rpc;
    rm.client = NVKM_RM_CLIENT(0);
    rm.device = NVKM_RM_DEVICE;
    rm.subdevice = NVKM_RM_SUBDEVICE;
    rm.ready = 1;

    /* Cuatro respuestas OK: tres FREE y el unload, en el orden en que se piden. */
    base = *rpc.rptr;
    for (i = 0; i < 3; i++) {
        fake_rpc_post_payload(lo, (base + i) % 63, NV_VGPU_MSG_FUNCTION_FREE, 0, NULL, 0);
    }
    fake_rpc_post_payload(lo, (base + 3) % 63,
                          NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER, 0, NULL, 0);
    msgq->tx.writePtr = (base + 4) % 63;

    fake_unload_pending = 1;
    fsp_mbox_reads = 0;
    pci_master_cleared = 0;
    wptr0 = *q.wptr;

    if (gsp_fini(&rm, &q, &rpc, fake_pdev) != 0) {
        printf("FALLO: gsp_fini con todo respondiendo bien\n");
        return -1;
    }
    fake_unload_pending = 0;

    /* Las tres peticiones de FREE. */
    for (i = 0; i < 3; i++) {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + i) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const uint32_t *p = (const uint32_t *)(hdr + 1);

        if (hdr->function != NV_VGPU_MSG_FUNCTION_FREE) {
            printf("FALLO: free %u con function=%u (esperaba %u; 27 era "
                   "DMA_FILL_PTE_MEM)\n", i, hdr->function,
                   NV_VGPU_MSG_FUNCTION_FREE);
            return -1;
        }
        if (hdr->length != sizeof(*hdr) + 16u) {
            printf("FALLO: free %u length=%u\n", i, hdr->length);
            return -1;
        }
        if (p[0] != NVKM_RM_CLIENT(0) || p[1] != 0 || p[2] != want_free[i] || p[3] != 0) {
            printf("FALLO: free %u params {0x%08x,0x%08x,0x%08x,0x%08x}, "
                   "esperaba objeto 0x%08x\n", i, p[0], p[1], p[2], p[3], want_free[i]);
            return -1;
        }
    }
    printf("OK: FREE (fn=%u) de subdevice, device y cliente, en ese orden\n",
           NV_VGPU_MSG_FUNCTION_FREE);

    /* Y el aviso de descarga: 8 B a cero, que es el unload que no es suspensión. */
    {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 3) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_unloading_guest_driver_v1F_07 *u =
            (const rpc_unloading_guest_driver_v1F_07 *)(hdr + 1);

        if (hdr->function != NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER) {
            printf("FALLO: unload con function=%u (esperaba %u)\n",
                   hdr->function, NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER);
            return -1;
        }
        if (hdr->length != sizeof(*hdr) + sizeof(*u)) {
            printf("FALLO: unload length=%u (esperaba %zu)\n",
                   hdr->length, sizeof(*hdr) + sizeof(*u));
            return -1;
        }
        if (u->bInPMTransition != 0 || u->bGc6Entering != 0 || u->newLevel != 0) {
            printf("FALLO: unload pm=%u gc6=%u level=%u (los tres a cero si no "
                   "es suspensión)\n", u->bInPMTransition, u->bGc6Entering, u->newLevel);
            return -1;
        }
    }
    printf("OK: UNLOADING_GUEST_DRIVER (fn=%u) con %zu B a cero\n",
           NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER,
           sizeof(rpc_unloading_guest_driver_v1F_07));

    if (pci_master_cleared != 1) {
        printf("FALLO: bus master quitado %u veces\n", pci_master_cleared);
        return -1;
    }
    if (rm.ready) {
        printf("FALLO: los objetos de RM siguen marcados como vivos\n");
        return -1;
    }
    printf("OK: bus master quitado y objetos de RM marcados como muertos\n");

    /* El caso que importa para el host: aunque no haya nada vivo con lo que
     * negociar, el DMA se corta igual. Es exactamente el escenario del cuelgue
     * del 2026-07-25, donde el apagado ordenado no era posible. */
    {
        struct gsp_rm dead;
        struct gsp_cmdq dead_q;
        struct gsp_rpc dead_rpc;

        memset(&dead, 0, sizeof(dead));
        memset(&dead_q, 0, sizeof(dead_q));
        memset(&dead_rpc, 0, sizeof(dead_rpc));
        pci_master_cleared = 0;
        if (gsp_fini(&dead, &dead_q, &dead_rpc, fake_pdev) == 0) {
            printf("FALLO: un apagado sin RPC vivo se dio por bueno\n");
            return -1;
        }
        if (pci_master_cleared != 1) {
            printf("FALLO: sin RPC vivo NO se quitó el bus master — "
                   "es justo cuando más falta hace\n");
            return -1;
        }
        printf("OK: sin RPC vivo el apagado falla pero corta el DMA igual\n");
    }

    /* Y sin pci_dev no hay forma de cortarlo: tiene que decirlo, no fingir. */
    if (gsp_fini(&rm, &q, &rpc, NULL) == 0) {
        printf("FALLO: un apagado sin pci_dev se dio por bueno\n");
        return -1;
    }
    printf("OK: sin pci_dev el apagado se declara fallido\n");
    return 0;
}

static int check_cmdq(const struct gsp_libos *lo)
{
    struct gsp_cmdq q;
    struct gsp_sysinfo si;
    const struct gsp_msg_elem *elem;
    const struct gsp_rpc_hdr *rpc;
    unsigned char *cmdq_base = (unsigned char *)lo->shm.va + lo->cmdq_offset;

    /* GspSystemInfo es el contrato con el firmware: si el tamaño baila, RM lee
     * los campos desplazados. Con el layout de r570 y alineación natural salen
     * 928 B; se imprime para que un cambio de header se note aquí. */
    printf("sizeof(GspSystemInfo) = %zu\n", sizeof(GspSystemInfo));
    if (sizeof(GspSystemInfo) != 928) { printf("FALLO: tamaño de GspSystemInfo\n"); return -1; }

    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: gsp_cmdq_init\n"); return -1; }
    if (q.cnt != 63) { printf("FALLO: cnt=%u\n", q.cnt); return -1; }
    if (*q.wptr != 0) { printf("FALLO: wptr inicial\n"); return -1; }

    memset(&si, 0, sizeof(si));
    si.bar0_phys = 0xf0000000ull;
    si.bar1_phys = 0xe0000000ull;
    si.bar3_phys = 0xd0000000ull;
    si.bdf = 0x100;
    si.vendor_id = 0x10de;
    si.device_id = 0x2f18;
    si.revision_id = 0xa1;

    if (gsp_cmdq_set_system_info(&q, &si) != 0) { printf("FALLO: set_system_info\n"); return -1; }

    /* El elemento va detrás de la primera página (la cabecera de la cola). */
    elem = (const struct gsp_msg_elem *)(cmdq_base + 4096);
    rpc = (const struct gsp_rpc_hdr *)(elem + 1);
    if (rpc->header_version != 0x03000000u) { printf("FALLO: header_version\n"); return -1; }
    if (rpc->signature != 0x43505256u) { printf("FALLO: signature 0x%08x\n", rpc->signature); return -1; }
    if (rpc->function != 72u) { printf("FALLO: function %u\n", rpc->function); return -1; }
    if (rpc->length != sizeof(struct gsp_rpc_hdr) + sizeof(GspSystemInfo)) {
        printf("FALLO: length %u\n", rpc->length);
        return -1;
    }
    /* El payload tiene que haber llegado entero. */
    {
        const GspSystemInfo *info = (const GspSystemInfo *)(rpc + 1);
        if (info->gpuPhysAddr != si.bar0_phys || info->gpuPhysFbAddr != si.bar1_phys ||
            info->PCIDeviceID != (((uint32_t)0x2f18 << 16) | 0x10de)) {
            printf("FALLO: el payload no cuadra\n");
            return -1;
        }
    }
    /* Checksum: XOR de todo el elemento en u64, plegado; con el campo ya puesto
     * el resultado tiene que dar cero. */
    {
        uint32_t len = (48u + rpc->length + 4095u) & ~4095u;
        const uint64_t *p = (const uint64_t *)elem;
        uint64_t csum = 0;
        for (uint32_t i = 0; i < len / 8u; i++) csum ^= p[i];
        if (((uint32_t)(csum >> 32) ^ (uint32_t)csum) != 0) {
            printf("FALLO: checksum no cierra\n");
            return -1;
        }
    }
    if (elem->elem_count != 1 || elem->sequence != 0) {
        printf("FALLO: elem_count=%u sequence=%u\n", elem->elem_count, elem->sequence);
        return -1;
    }
    if (*q.wptr != 1) { printf("FALLO: wptr=%u tras un RPC\n", *q.wptr); return -1; }
    printf("OK: SET_SYSTEM_INFO encolado (1 página, checksum cierra)\n");

    if (gsp_cmdq_set_registry(&q) != 0) { printf("FALLO: set_registry\n"); return -1; }
    {
        const struct gsp_msg_elem *e2 = (const struct gsp_msg_elem *)(cmdq_base + 4096 + 4096);
        const struct gsp_rpc_hdr *r2 = (const struct gsp_rpc_hdr *)(e2 + 1);
        const PACKED_REGISTRY_TABLE *t = (const PACKED_REGISTRY_TABLE *)(r2 + 1);
        const PACKED_REGISTRY_ENTRY *en = (const PACKED_REGISTRY_ENTRY *)(t + 1);
        const char *name0 = (const char *)t + en[0].nameOffset;

        if (r2->function != 73u) { printf("FALLO: fn registry %u\n", r2->function); return -1; }
        if (t->numEntries != 3) { printf("FALLO: numEntries %u\n", t->numEntries); return -1; }
        if (en[0].type != 1 || en[0].data != 1) { printf("FALLO: entrada 0\n"); return -1; }
        if (strcmp(name0, "RMSecBusResetEnable")) {
            printf("FALLO: nombre 0 = %s\n", name0);
            return -1;
        }
        if (e2->sequence != 1) { printf("FALLO: sequence no avanza\n"); return -1; }
        printf("OK: SET_REGISTRY encolado (3 claves, nombres en su offset)\n");
    }
    if (*q.wptr != 2) { printf("FALLO: wptr=%u tras dos RPCs\n", *q.wptr); return -1; }
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
    if (check_cmdq(&lo) != 0)
        return -1;
    if (check_rpc_sync(&lo) != 0)
        return -1;
    if (check_rm_objects(&lo) != 0)
        return -1;
    if (check_vmm(&lo) != 0)
        return -1;
    if (check_g4e_chan_ce(&lo) != 0)
        return -1;
    if (check_fini(&lo) != 0)
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
