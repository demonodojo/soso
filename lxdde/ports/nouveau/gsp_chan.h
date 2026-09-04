/* G4e (1/2): canal GPFIFO + USERD + pushbuffer.
 *
 * Reserva el canal vía GSP-RM (`AMPERE_CHANNEL_GPFIFO_A`), prepara los búferes
 * en sysmem coherente, los mapea en el vaspace externo de G4d y expone la
 * cola GPFIFO y el USERD para que gsp_ce encole trabajo.
 */
#ifndef GSP_CHAN_H
#define GSP_CHAN_H

#include "gsp_dma.h"
#include "gsp_rm_obj.h"
#include "gsp_vmm.h"
#include "gsp_vram.h"
#include "nvrm_r570.h"

/* En Blackwell el USERD no hace writeback de GPGet (lee 0). El progreso lo
 * lleva SW tras el semáforo CE/QMD (`ack_progress`); no se escribe USERD.GPGet
 * (nouveau / nvidia-push). USERD.GPPut va módulo ENTRIES en submit — escribir
 * el contador libre (4096…) cuelga el PBDMA. 4096×8 B. */
#define GSP_CHAN_GPFIFO_ENTRIES  4096u
#define GSP_CHAN_GPFIFO_SIZE     (GSP_CHAN_GPFIFO_ENTRIES * NVC56F_GP_ENTRY__SIZE)
#define GSP_CHAN_USERD_SIZE      4096u
#define GSP_CHAN_PB_SIZE         65536u
#define GSP_CHAN_NOTIFIER_SIZE   4096u
/* Tope de cordura para lo que RM conteste como tamaño del method buffer. */
#define GSP_CHAN_MTHDBUF_MAX     (1u << 20)

/* Sysmem mapeada en el vaspace, justo detrás de la página scratch de G4d.
 * Derivada de `GSP_VA_BASE` (gsp_vmm.h), que es donde está la razón de que la
 * base viva por debajo de 2^40: aquí dentro está el pushbuffer, y su VA va en una
 * entrada de GPFIFO, que sólo llega al bit 39. */
#define GSP_CHAN_VA_BASE         (GSP_VA_BASE + 0x20000000ull)
/* Ventana por canal: GPFIFO 32 KiB + USERD/PB/notifier 12 KiB ≈ 44 KiB; 64 KiB
 * de stride separa canales para que un desborde no pise al vecino. */
#define GSP_CHAN_VA_STRIDE       0x10000ull

struct gsp_chan {
    struct gsp_rm *rm;
    struct gsp_vmm *vmm;
    uint32_t handle;
    /* El chid que le PEDIMOS a RM. No se manda como número: viaja dentro de los
     * dos subcampos de USERD de `flags` (índice = chid % 8, página = chid / 8), que
     * es como lo hace `r535_chan_alloc`. Dos canales con el mismo chid piden el
     * mismo slot de USERD y el segundo se lleva un NO_MEMORY. El token de
     * `GET_WORK_SUBMIT_TOKEN` dice cuál acabó siendo, y contrastarlo con éste es
     * lo que prueba que el mecanismo es el que creemos. */
    uint32_t chid;
    /* `NV2080_ENGINE_TYPE_*` al que va atado. No es un detalle de configuración:
     * el motor decide qué objetos acepta RM sobre este canal (CE en COPY0,
     * compute en GR0), y equivocarlo se paga con un INVALID_CLASS que parece
     * hablar de la clase del objeto. */
    uint32_t engine;
    /* Clase con la que RM aceptó el canal. No es un `#define` porque depende
     * del chip y la dice su catálogo (`gsp_rm_class_pick`). */
    uint32_t cls;
    struct gsp_dma_buf gpfifo;
    struct gsp_dma_buf userd;
    struct gsp_dma_buf pushbuf;
    struct gsp_dma_buf notifier;
    /* Method buffer del CE. No es el pushbuffer —eso era el error—: es un búfer
     * aparte que RM usa para reinyectar métodos tras un fallo de página, y su
     * tamaño lo dicta RM, no nosotros. */
    struct gsp_dma_buf mthdbuf;
    uint32_t mthdbuf_size;
    uint64_t gpfifo_va;
    uint64_t userd_va;
    uint64_t pushbuf_va;
    uint64_t notifier_va;
    /* Bloque de instancia del canal, en VRAM. La CPU lo lee por PRAMIN
     * (`gsp_pramin_*`), no por BAR1. */
    uint64_t inst_addr;
    /* USERD en VRAM (paridad r535); GPPut vía PRAMIN en cada submit. */
    int userd_vram;
    unsigned gpput;
    /* Progreso del GPFIFO visto por SW. Desde VOLTA el USERD solo actualiza
     * GPGet a timer (compat) y en BLACKWELL_CHANNEL_GPFIFO_* el writeback
     * desapareció del todo (nouveau 862450a / Skeggs 2025): leer 0x88 siempre
     * da 0. El ack lo pone quien espera el semáforo CE/QMD —equivalente al
     * non-WFI sem release de upstream— y pb_rewind mira este valor, no el USERD. */
    unsigned gpget;
    unsigned pb_pos;
    /* Valor a escribir en el doorbell para patear ESTE canal. El RPC
     * GET_WORK_SUBMIT_TOKEN da runlist+chid; en gb20x hay que OR el bit 30
     * (RUNLIST_DOORBELL_ENABLE) — ver gsp_chan_doorbell_kick(). */
    uint32_t doorbell_token;
    uint32_t doorbell_kick;
    int doorbell_ok;
    int ready;
};

/* `rm`/`vmm` deben estar listos (G4c/G4d). `vaspace` = handle del vaspace.
 * `vram` hace falta para el bloque de instancia, que va en VRAM. `idx` numera el
 * canal: da su handle y su ventana de VAs, así que dos canales vivos no pueden
 * compartirlo. `engine` es el `NV2080_ENGINE_TYPE_*` al que se ata. */
int gsp_chan_init(struct gsp_rm *rm, struct gsp_vmm *vmm, struct gsp_vram *vram,
                  struct gsp_chan *c, uint32_t vaspace, unsigned idx,
                  uint32_t engine);

/* Reserva espacio en el pushbuffer (alineado a 4 B). Devuelve el offset o -1. */
int gsp_chan_pb_reserve(struct gsp_chan *c, unsigned bytes);

/* Vuelve al principio del pushbuffer para reutilizarlo. SOLO es válido cuando
 * `gpget == gpput` (progreso SW tras ack): reescribir métodos que el host
 * todavía no ha leído es corrupción silenciosa. Devuelve 0 si rebobinó. */
int gsp_chan_pb_rewind(struct gsp_chan *c);

/* Marca todo lo encolado como consumido (gpget SW). Llamar solo tras un wait
 * del semáforo CE/QMD que haya visto el payload. No toca USERD.GPGet. */
void gsp_chan_ack_progress(struct gsp_chan *c);

/* Encola un segmento del pushbuffer en el GPFIFO y publica Put/GPPut. */
int gsp_chan_submit(struct gsp_chan *c, unsigned pb_off, unsigned pb_len);

/* Vuelca USERD + progreso SW. En Blackwell el GPGet del USERD es cosmético. */
void gsp_chan_dump(struct gsp_chan *c, const char *why);

void gsp_chan_fini(struct gsp_chan *c);

#endif
