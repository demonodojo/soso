/* G3b paso 5: libos boot args + colas compartidas. Ver gsp_libos.h. */
#include "gsp_libos.h"

/* Definidos en el shim (lxdde/shim/src/shims.c). */
void *memcpy(void *dst, const void *src, unsigned long n);
void *memset(void *dst, int c, unsigned long n);

/* El firmware lee estas estructuras tal cual: si el layout se desvía, lee basura.
 * Los tamaños salen del ABI de r570 (ver gsp_libos.h). */
typedef char libos_region_size_check[sizeof(struct gsp_libos_region) == 32 ? 1 : -1];
typedef char msgq_tx_size_check[sizeof(struct gsp_msgq_tx_header) == 32 ? 1 : -1];
typedef char msgq_rx_off_check[offsetof(struct gsp_msgq_headers, rx.readPtr) == 32 ? 1 : -1];
typedef char rmargs_dmem_check[offsetof(struct gsp_arguments_cached, bDmemStack) == 48 ? 1 : -1];
typedef char rmargs_prof_check[offsetof(struct gsp_arguments_cached, profilerArgs) == 56 ? 1 : -1];
typedef char boot_params_rm_check[offsetof(struct gsp_fmc_boot_params, gspRmParams) == 40 ? 1 : -1];
typedef char boot_params_size_check[sizeof(struct gsp_fmc_boot_params) == 80 ? 1 : -1];

#define LIBOS_MEMORY_REGION_CONTIGUOUS  1u
#define LIBOS_MEMORY_REGION_LOC_SYSMEM  1u

/* Tamaños de `r535_gsp_shared_init` y `r535_gsp_libos_init`. */
#define GSP_QUEUE_SIZE   0x40000ul   /* comandos y mensajes, cada uno */
#define GSP_LOG_SIZE     0x10000ul   /* LOGINIT / LOGINTR / LOGRM */
#define GSP_LIBOS_SIZE   0x1000ul
#define GSP_RMARGS_SIZE  0x1000ul

#define GSP_LIBOS_REGIONS 4u

/* `r535_gsp_libos_id8`: el nombre empaquetado en un u64, big-endian. */
static uint64_t libos_id8(const char *name)
{
    uint64_t id = 0;
    unsigned i;

    for (i = 0; i < 8u && name[i]; i++) {
        id = (id << 8) | (unsigned char)name[i];
    }
    return id;
}

/* `create_pte_array`: aunque el búfer sea contiguo, GSP-RM espera la lista de
 * físicas de cada página de 4 KiB. Va dentro del propio búfer, tras el "put
 * pointer" de la primera palabra. */
static void create_pte_array(uint64_t *ptes, uint64_t addr, unsigned long size)
{
    unsigned long pages = (size + GSP_PAGE_SIZE - 1) / GSP_PAGE_SIZE;
    unsigned long i;

    for (i = 0; i < pages; i++) {
        ptes[i] = addr + (i << GSP_PAGE_SHIFT);
    }
}

/* `r535_gsp_shared_init`: un bloque contiguo con la tabla de PTEs delante y las
 * dos colas detrás. La cola de comandos lleva las cabeceras; la de mensajes las
 * suyas propias, y cada una apunta al `readPtr` de la otra. */
static int shared_init(struct gsp_libos *lo)
{
    struct gsp_msgq_headers *cmdq;
    uint64_t *ptes;
    unsigned i;
    unsigned long total;

    lo->shm_ptes_nr = (unsigned)((GSP_QUEUE_SIZE + GSP_QUEUE_SIZE) >> GSP_PAGE_SHIFT);
    /* Las propias páginas de la tabla también se describen a sí mismas. */
    lo->shm_ptes_nr += (unsigned)((lo->shm_ptes_nr * sizeof(uint64_t) + GSP_PAGE_SIZE - 1) /
                                  GSP_PAGE_SIZE);
    lo->shm_ptes_size = (lo->shm_ptes_nr * sizeof(uint64_t) + GSP_PAGE_SIZE - 1) &
                        ~(GSP_PAGE_SIZE - 1);

    total = lo->shm_ptes_size + GSP_QUEUE_SIZE + GSP_QUEUE_SIZE;
    if (gsp_dma_alloc(&lo->shm, total, "colas GSP") != 0) {
        return -1;
    }
    lo->cmdq_offset = lo->shm_ptes_size;
    lo->msgq_offset = lo->cmdq_offset + GSP_QUEUE_SIZE;

    ptes = lo->shm.va;
    for (i = 0; i < lo->shm_ptes_nr; i++) {
        ptes[i] = lo->shm.phys + ((uint64_t)i << GSP_PAGE_SHIFT);
    }

    cmdq = (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->cmdq_offset);
    cmdq->tx.version = 0;
    cmdq->tx.size = (uint32_t)GSP_QUEUE_SIZE;
    cmdq->tx.entryOff = (uint32_t)GSP_PAGE_SIZE;
    cmdq->tx.msgSize = (uint32_t)GSP_PAGE_SIZE;
    cmdq->tx.msgCount = (cmdq->tx.size - cmdq->tx.entryOff) / cmdq->tx.msgSize;
    cmdq->tx.writePtr = 0;
    cmdq->tx.flags = 1;
    cmdq->tx.rxHdrOff = (uint32_t)offsetof(struct gsp_msgq_headers, rx.readPtr);
    /* La cola de mensajes la rellena el GSP; nosotros solo la dejamos a cero. */
    return 0;
}

/* `r570_gsp_set_rmargs`. */
static void set_rmargs(struct gsp_libos *lo)
{
    struct gsp_arguments_cached *args = lo->rmargs.va;

    args->messageQueueInitArguments.sharedMemPhysAddr = lo->shm.phys;
    args->messageQueueInitArguments.pageTableEntryCount = lo->shm_ptes_nr;
    args->messageQueueInitArguments.cmdQueueOffset = lo->cmdq_offset;
    args->messageQueueInitArguments.statQueueOffset = lo->msgq_offset;

    args->srInitArguments.oldLevel = 0;
    args->srInitArguments.flags = 0;
    args->srInitArguments.bInPMTransition = 0;

    args->bDmemStack = 1;   /* r570; en r535 este campo no existe */
}

static void region_set(struct gsp_libos_region *r, const char *name,
                       const struct gsp_dma_buf *b)
{
    r->id8 = libos_id8(name);
    r->pa = b->phys;
    r->size = b->size;
    r->kind = LIBOS_MEMORY_REGION_CONTIGUOUS;
    r->loc = LIBOS_MEMORY_REGION_LOC_SYSMEM;
}

/* `gh100_gsp_init`: los parámetros que el FSP entrega al FMC. */
static void boot_params_set(struct gsp_libos *lo, const struct gsp_wpr *wpr)
{
    struct gsp_fmc_boot_params *p = lo->boot_params.va;

    p->bootGspRmParams.target = GSP_DMA_TARGET_COHERENT_SYSTEM;
    p->bootGspRmParams.gspRmDescOffset = wpr->meta_phys;
    p->bootGspRmParams.gspRmDescSize = (uint32_t)sizeof(struct gsp_wpr_meta);
    p->bootGspRmParams.bIsGspRmBoot = 1;

    p->gspRmParams.target = GSP_DMA_TARGET_NONCOHERENT_SYSTEM;
    p->gspRmParams.bootArgsOffset = lo->libos.phys;
}

/* Releer lo escrito antes de que el FMC lo siga: aquí un puntero mal puesto no
 * da error, cuelga el arranque del GSP sin decir por qué. */
static int libos_verify(const struct gsp_libos *lo, const struct gsp_wpr *wpr)
{
    const struct gsp_libos_region *r = lo->libos.va;
    const struct gsp_arguments_cached *args = lo->rmargs.va;
    const struct gsp_fmc_boot_params *p = lo->boot_params.va;
    const struct gsp_msgq_headers *cmdq =
        (const struct gsp_msgq_headers *)((const unsigned char *)lo->shm.va + lo->cmdq_offset);
    const struct gsp_dma_buf *bufs[GSP_LIBOS_REGIONS];
    const char *names[GSP_LIBOS_REGIONS] = { "LOGINIT", "LOGINTR", "LOGRM", "RMARGS" };
    unsigned i;

    bufs[0] = &lo->loginit;
    bufs[1] = &lo->logintr;
    bufs[2] = &lo->logrm;
    bufs[3] = &lo->rmargs;

    for (i = 0; i < GSP_LIBOS_REGIONS; i++) {
        if (r[i].id8 != libos_id8(names[i]) || r[i].pa != bufs[i]->phys ||
            r[i].size != bufs[i]->size || r[i].kind != LIBOS_MEMORY_REGION_CONTIGUOUS ||
            r[i].loc != LIBOS_MEMORY_REGION_LOC_SYSMEM) {
            lx_printk("nouveau-lx: libos región %u (%s) mal escrita\n", i, names[i]);
            return -1;
        }
        if (r[i].pa & (GSP_PAGE_SIZE - 1)) {
            lx_printk("nouveau-lx: libos región %u sin alinear\n", i);
            return -1;
        }
    }
    /* La lista de PTEs de cada log empieza tras el put pointer y su primera
     * entrada tiene que ser la propia base del búfer. */
    for (i = 0; i < 3u; i++) {
        const uint64_t *ptes = (const uint64_t *)((const unsigned char *)bufs[i]->va +
                                                  sizeof(uint64_t));
        unsigned long pages = bufs[i]->size / GSP_PAGE_SIZE;
        unsigned long j;
        for (j = 0; j < pages; j++) {
            if (ptes[j] != bufs[i]->phys + ((uint64_t)j << GSP_PAGE_SHIFT)) {
                lx_printk("nouveau-lx: libos %s PTE %lu incorrecta\n", names[i], j);
                return -1;
            }
        }
    }
    if (args->messageQueueInitArguments.sharedMemPhysAddr != lo->shm.phys ||
        args->messageQueueInitArguments.pageTableEntryCount != lo->shm_ptes_nr ||
        args->messageQueueInitArguments.cmdQueueOffset != lo->cmdq_offset ||
        args->messageQueueInitArguments.statQueueOffset != lo->msgq_offset ||
        !args->bDmemStack) {
        lx_printk("nouveau-lx: RMARGS no cuadra con la memoria compartida\n");
        return -1;
    }
    if (cmdq->tx.size != GSP_QUEUE_SIZE || cmdq->tx.msgSize != GSP_PAGE_SIZE ||
        cmdq->tx.entryOff != GSP_PAGE_SIZE || cmdq->tx.flags != 1 ||
        cmdq->tx.msgCount != (GSP_QUEUE_SIZE - GSP_PAGE_SIZE) / GSP_PAGE_SIZE ||
        cmdq->tx.rxHdrOff != 32u) {
        lx_printk("nouveau-lx: cabecera de la cola de comandos mal formada\n");
        return -1;
    }
    if (p->bootGspRmParams.gspRmDescOffset != wpr->meta_phys ||
        p->bootGspRmParams.gspRmDescSize != sizeof(struct gsp_wpr_meta) ||
        p->bootGspRmParams.target != GSP_DMA_TARGET_COHERENT_SYSTEM ||
        !p->bootGspRmParams.bIsGspRmBoot ||
        p->gspRmParams.bootArgsOffset != lo->libos.phys ||
        p->gspRmParams.target != GSP_DMA_TARGET_NONCOHERENT_SYSTEM) {
        lx_printk("nouveau-lx: boot params no enlazan WPR meta y libos\n");
        return -1;
    }
    /* El WPR lo talla el FMC: si aquí hubiera carveout, sería inventado. */
    if (p->bootGspRmParams.wprCarveoutOffset || p->bootGspRmParams.wprCarveoutSize) {
        lx_printk("nouveau-lx: boot params con carveout que fija el FMC\n");
        return -1;
    }
    return 0;
}

int gsp_libos_prepare(const struct gsp_wpr *wpr, struct gsp_libos *out)
{
    struct gsp_libos_region *regions;

    if (!out || !wpr || !wpr->ready) {
        return -1;
    }
    memset(out, 0, sizeof(*out));

    if (shared_init(out) != 0 ||
        gsp_dma_alloc(&out->libos, GSP_LIBOS_SIZE, "libos") != 0 ||
        gsp_dma_alloc(&out->loginit, GSP_LOG_SIZE, "LOGINIT") != 0 ||
        gsp_dma_alloc(&out->logintr, GSP_LOG_SIZE, "LOGINTR") != 0 ||
        gsp_dma_alloc(&out->logrm, GSP_LOG_SIZE, "LOGRM") != 0 ||
        gsp_dma_alloc(&out->rmargs, GSP_RMARGS_SIZE, "RMARGS") != 0 ||
        gsp_dma_alloc(&out->boot_params, sizeof(struct gsp_fmc_boot_params), "boot params") != 0) {
        gsp_libos_release(out);
        return -1;
    }
    /* Las 4 regiones tienen que caber en la página de boot args. */
    if (GSP_LIBOS_REGIONS * sizeof(struct gsp_libos_region) > out->libos.size) {
        gsp_libos_release(out);
        return -1;
    }

    create_pte_array((uint64_t *)((unsigned char *)out->loginit.va + sizeof(uint64_t)),
                     out->loginit.phys, out->loginit.size);
    create_pte_array((uint64_t *)((unsigned char *)out->logintr.va + sizeof(uint64_t)),
                     out->logintr.phys, out->logintr.size);
    create_pte_array((uint64_t *)((unsigned char *)out->logrm.va + sizeof(uint64_t)),
                     out->logrm.phys, out->logrm.size);

    set_rmargs(out);

    regions = out->libos.va;
    region_set(&regions[0], "LOGINIT", &out->loginit);
    region_set(&regions[1], "LOGINTR", &out->logintr);
    region_set(&regions[2], "LOGRM", &out->logrm);
    region_set(&regions[3], "RMARGS", &out->rmargs);

    boot_params_set(out, wpr);

    if (libos_verify(out, wpr) != 0) {
        gsp_libos_release(out);
        return -1;
    }

    out->ready = 1;
    lx_printk("nouveau-lx: colas GSP shm=0x%llx (%lu KiB, %u PTEs) cmdq=+0x%lx msgq=+0x%lx %u msgs\n",
              (unsigned long long)out->shm.phys, out->shm.size / 1024u, out->shm_ptes_nr,
              out->cmdq_offset, out->msgq_offset,
              (unsigned)((GSP_QUEUE_SIZE - GSP_PAGE_SIZE) / GSP_PAGE_SIZE));
    lx_printk("nouveau-lx: libos verificado @0x%llx 4 regiones (log 3x%lu KiB) rmargs=0x%llx\n",
              (unsigned long long)out->libos.phys, (unsigned long)(GSP_LOG_SIZE / 1024u),
              (unsigned long long)out->rmargs.phys);
    lx_printk("nouveau-lx: boot params @0x%llx → wpr meta 0x%llx + libos 0x%llx\n",
              (unsigned long long)out->boot_params.phys,
              (unsigned long long)wpr->meta_phys,
              (unsigned long long)out->libos.phys);
    return 0;
}

void gsp_libos_release(struct gsp_libos *lo)
{
    if (!lo) {
        return;
    }
    gsp_dma_free(&lo->boot_params);
    gsp_dma_free(&lo->rmargs);
    gsp_dma_free(&lo->logrm);
    gsp_dma_free(&lo->logintr);
    gsp_dma_free(&lo->loginit);
    gsp_dma_free(&lo->libos);
    gsp_dma_free(&lo->shm);
    memset(lo, 0, sizeof(*lo));
}
