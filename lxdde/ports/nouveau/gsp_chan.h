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
/* Tope de cordura para lo que RM conteste como tamaño del method buffer. */
#define GSP_CHAN_MTHDBUF_MAX     (1u << 20)

/* Sysmem mapeada en el vaspace, justo detrás de la página scratch de G4d.
 * Derivada de `GSP_VA_BASE` (gsp_vmm.h), que es donde está la razón de que la
 * base viva por debajo de 2^40: aquí dentro está el pushbuffer, y su VA va en una
 * entrada de GPFIFO, que sólo llega al bit 39. */
#define GSP_CHAN_VA_BASE         (GSP_VA_BASE + 0x20000000ull)
/* Cada canal se lleva su propia ventana. Sus cuatro búferes ocupan 16 KiB; el
 * hueco de 64 KiB deja sitio de sobra y hace que la VA diga de un vistazo de qué
 * canal es. Con dos canales apilados a 16 KiB, un desbordamiento del pushbuffer
 * de uno caería dentro del GPFIFO del otro sin que nada se quejara. */
#define GSP_CHAN_VA_STRIDE       0x10000ull

struct gsp_chan {
    struct gsp_rm *rm;
    struct gsp_vmm *vmm;
    uint32_t handle;
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
    /* Bloque de instancia del canal, en VRAM. No lo tocamos nunca desde la CPU
     * (sin BAR1 no hay ventana): es RM quien lo usa, nosotros solo decimos
     * dónde está. Por eso es una dirección pelada y no un gsp_dma_buf. */
    uint64_t inst_addr;
    Nvc56fControl *userd_ctl;
    unsigned gpput;
    unsigned pb_pos;
    /* Valor a escribir en el doorbell para patear ESTE canal, tal cual lo da RM
     * (`GET_WORK_SUBMIT_TOKEN`). El flag va aparte del valor porque un token de
     * 0 es perfectamente legítimo —runlist 0, chid 0— y no se puede usar el cero
     * como "no lo tengo". */
    uint32_t doorbell_token;
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

/* Vuelve al principio del pushbuffer para reutilizarlo. SOLO es válido cuando el
 * host ya consumió todo lo encolado (GPGet == GPPut): reescribir métodos que el
 * host todavía no ha leído es corrupción silenciosa del trabajo en vuelo, así
 * que si no se cumple esto falla en vez de rebobinar. Devuelve 0 si rebobinó. */
int gsp_chan_pb_rewind(struct gsp_chan *c);

/* Encola un segmento del pushbuffer en el GPFIFO y publica Put/GPPut. */
int gsp_chan_submit(struct gsp_chan *c, unsigned pb_off, unsigned pb_len);

/* Vuelca el estado del canal visto desde el USERD. Diagnóstico, no control: lo
 * llama quien se queda esperando algo que no llega. Lo importante es `GPGet`,
 * que dice si el host recogió el trabajo o ni se enteró — ver el comentario de
 * la implementación. */
void gsp_chan_dump(struct gsp_chan *c, const char *why);

void gsp_chan_fini(struct gsp_chan *c);

#endif
