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

#define GSP_CHAN_GPFIFO_ENTRIES  64u
#define GSP_CHAN_GPFIFO_SIZE     4096u
#define GSP_CHAN_USERD_SIZE      4096u
#define GSP_CHAN_PB_SIZE         4096u
#define GSP_CHAN_NOTIFIER_SIZE   4096u

/* Sysmem mapeada en el vaspace, justo detrás de la página scratch de G4d. */
#define GSP_CHAN_VA_BASE         0x0000010020000000ull

struct gsp_chan {
    struct gsp_rm *rm;
    struct gsp_vmm *vmm;
    uint32_t handle;
    struct gsp_dma_buf gpfifo;
    struct gsp_dma_buf userd;
    struct gsp_dma_buf pushbuf;
    struct gsp_dma_buf notifier;
    uint64_t gpfifo_va;
    uint64_t userd_va;
    uint64_t pushbuf_va;
    uint64_t notifier_va;
    /* Bloque de instancia del canal, en VRAM. No lo tocamos nunca desde la CPU
     * (sin BAR1 no hay ventana): es RM quien lo usa, nosotros solo decimos
     * dónde está. Por eso es una dirección pelada y no un gsp_dma_buf. */
    uint64_t inst_addr;
    Nvc56fControl *userd_ctl;
    unsigned gpput;
    unsigned pb_pos;
    int ready;
};

/* `rm`/`vmm` deben estar listos (G4c/G4d). `vaspace` = handle del vaspace.
 * `vram` hace falta para el bloque de instancia, que va en VRAM. */
int gsp_chan_init(struct gsp_rm *rm, struct gsp_vmm *vmm, struct gsp_vram *vram,
                  struct gsp_chan *c, uint32_t vaspace);

/* Reserva espacio en el pushbuffer (alineado a 4 B). Devuelve el offset o -1. */
int gsp_chan_pb_reserve(struct gsp_chan *c, unsigned bytes);

/* Encola un segmento del pushbuffer en el GPFIFO y publica Put/GPPut. */
int gsp_chan_submit(struct gsp_chan *c, unsigned pb_off, unsigned pb_len);

void gsp_chan_fini(struct gsp_chan *c);

#endif
