/* G3b paso 5: libos boot args — lo último que hay que construir antes del COT.
 *
 * El GSP-FMC recibe un `GSP_FMC_BOOT_PARAMS` que apunta a dos cosas: el WPR meta
 * (paso 4) y los *boot args* de libos, que son una página con un array de regiones
 * de memoria. Esas regiones son los tres búferes de log (LOGINIT/LOGINTR/LOGRM) y
 * los argumentos de RM (RMARGS), que a su vez describen las colas de mensajes
 * compartidas por las que hablarán CPU y GSP.
 *
 * Referencias upstream: `r535_gsp_libos_init`, `r535_gsp_shared_init` y
 * `r570_gsp_set_rmargs` (`nvkm/subdev/gsp/rm/{r535,r570}/gsp.c`), estructuras de
 * `rm/r570/nvrm/gsp.h`. Ojo: r535 y r570 **no** comparten layout de
 * `GSP_ARGUMENTS_CACHED` ni de `MESSAGE_QUEUE_INIT_ARGUMENTS` (r570 quita los
 * campos `lockless*` y añade `bDmemStack`). Aquí va el de r570, que es el que
 * corresponde al firmware 570.144.
 */
#ifndef GSP_LIBOS_H
#define GSP_LIBOS_H

#include "gsp_dma.h"
#include "gsp_wpr.h"

/* --- Estructuras de firmware (r570) ---------------------------------------- */

/* LibosMemoryRegionInitArgument */
struct gsp_libos_region {
    uint64_t id8;      /* nombre en 8 chars big-endian: "LOGINIT", … */
    uint64_t pa;
    uint64_t size;
    uint8_t kind;      /* LIBOS_MEMORY_REGION_CONTIGUOUS = 1 */
    uint8_t loc;       /* LIBOS_MEMORY_REGION_LOC_SYSMEM = 1 */
};

/* msgqTxHeader / msgqRxHeader, al principio de cada cola. */
struct gsp_msgq_tx_header {
    uint32_t version;
    uint32_t size;
    uint32_t msgSize;
    uint32_t msgCount;
    uint32_t writePtr;
    uint32_t flags;
    uint32_t rxHdrOff;
    uint32_t entryOff;
};

struct gsp_msgq_rx_header {
    uint32_t readPtr;
};

struct gsp_msgq_headers {
    struct gsp_msgq_tx_header tx;
    struct gsp_msgq_rx_header rx;
};

/* MESSAGE_QUEUE_INIT_ARGUMENTS (r570: sin los campos lockless de r535). */
struct gsp_msgq_init_args {
    uint64_t sharedMemPhysAddr;
    uint32_t pageTableEntryCount;
    uint64_t cmdQueueOffset;
    uint64_t statQueueOffset;
};

struct gsp_sr_init_args {
    uint32_t oldLevel;
    uint32_t flags;
    uint8_t bInPMTransition;
};

/* GSP_ARGUMENTS_CACHED (r570). */
struct gsp_arguments_cached {
    struct gsp_msgq_init_args messageQueueInitArguments;
    struct gsp_sr_init_args srInitArguments;
    uint32_t gpuInstance;
    uint8_t bDmemStack;
    struct {
        uint64_t pa;
        uint64_t size;
    } profilerArgs;
};

/* GSP_DMA_TARGET */
#define GSP_DMA_TARGET_LOCAL_FB            0u
#define GSP_DMA_TARGET_COHERENT_SYSTEM     1u
#define GSP_DMA_TARGET_NONCOHERENT_SYSTEM  2u

struct gsp_fmc_init_params {
    uint32_t regkeys;
};

struct gsp_acr_boot_gsp_rm_params {
    uint32_t target;
    uint32_t gspRmDescSize;
    uint64_t gspRmDescOffset;
    uint64_t wprCarveoutOffset;
    uint32_t wprCarveoutSize;
    uint8_t bIsGspRmBoot;
};

struct gsp_rm_params {
    uint32_t target;
    uint64_t bootArgsOffset;
};

struct gsp_spdm_params {
    uint32_t target;
    uint64_t payloadBufferOffset;
    uint32_t payloadBufferSize;
};

/* GSP_FMC_BOOT_PARAMS: el payload al que apuntará el COT. */
struct gsp_fmc_boot_params {
    struct gsp_fmc_init_params initParams;
    struct gsp_acr_boot_gsp_rm_params bootGspRmParams;
    struct gsp_rm_params gspRmParams;
    struct gsp_spdm_params gspSpdmParams;
};

/* --- Estado del paso 5 ------------------------------------------------------ */

struct gsp_libos {
    /* Memoria compartida: tabla de PTEs + cola de comandos + cola de mensajes,
     * todo en un único bloque físicamente contiguo. */
    struct gsp_dma_buf shm;
    unsigned shm_ptes_nr;
    unsigned long shm_ptes_size;
    unsigned long cmdq_offset;
    unsigned long msgq_offset;

    struct gsp_dma_buf libos;    /* array de 4 regiones */
    struct gsp_dma_buf loginit;
    struct gsp_dma_buf logintr;
    struct gsp_dma_buf logrm;
    struct gsp_dma_buf rmargs;
    struct gsp_dma_buf boot_params;

    int ready;
};

/* Construye colas, búferes de log, RMARGS, el array de regiones y el
 * `GSP_FMC_BOOT_PARAMS` que los enlaza con el WPR meta. Todo en memoria: no lee
 * ni escribe un solo registro. */
int gsp_libos_prepare(const struct gsp_wpr *wpr, struct gsp_libos *out);

void gsp_libos_release(struct gsp_libos *lo);

#endif
