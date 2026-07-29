/* G4f: contexto de GR — consulta, reserva, mapeo y promoción.
 *
 * Un canal de GR no ejecuta nada hasta que su contexto está promocionado. El
 * driver le pregunta a RM los tamaños de los búferes de contexto del gráfico, los
 * reserva en VRAM, los mapea en el vaspace del canal y se los entrega con
 * `NV2080_CTRL_CMD_GPU_PROMOTE_CTX`. Sin esto, el objeto de compute se reserva sin
 * queja y el primer QMD no puede correr.
 *
 * Referencias (leídas el 2026-07-28, no de memoria): `r535_gr_oneinit`,
 * `r535_gr_get_ctxbuf_info` y `r535_gr_promote_ctx` de
 * `nvkm/subdev/gsp/rm/r535/gr.c`, con los layouts en `nvrm_r570.h`.
 *
 * DOS JUEGOS DE NÚMEROS QUE NO COINCIDEN. La consulta devuelve un array indexado
 * por *id de propiedad de contexto* (`NV0080_CTX_PROP_*`, 0x00…0x19) y la
 * promoción habla de *bufferId* (`..._BUFFER_ID_*`, 0…12). Ni son el mismo número
 * ni están en el mismo orden: el mapa de `gsp_grctx.c` traduce, y usar un id por
 * el otro es un búfer del tamaño de otro sin un solo mensaje de error.
 */
#ifndef GSP_GRCTX_H
#define GSP_GRCTX_H

#include "gsp_chan.h"
#include "gsp_vmm.h"
#include "gsp_vram.h"

/* Ocho propiedades del mapa de upstream, más el duplicado del mapa de acceso
 * privilegiado (que se promociona dos veces con bufferId distinto). */
#define GSP_GRCTX_MAX 10u

/* Base de las VAs de los búferes de contexto. Detrás de la sysmem del compute
 * (`G4F_DATA_VA` = GSP_VA_BASE + 0x30000000) y con 256 MiB de hueco: el
 * attribute CB pide alineación a su propio tamaño redondeado a potencia de dos, que
 * en un chip grande son decenas de MiB. */
#define GSP_GRCTX_VA_BASE (GSP_VA_BASE + 0x40000000ull)
/* Tope de la ventana. Que exista es lo que convierte "no cabe" en un error con
 * nombre en vez de en un mapeo encima de otra cosa. */
#define GSP_GRCTX_VA_SIZE 0x40000000ull

struct gsp_grctx_buf {
    uint32_t buffer_id;    /* `NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_*` */
    uint32_t prop_id;      /* `NV0080_CTX_PROP_*`, de dónde salió el tamaño */
    uint64_t size;         /* ya con el ajuste del principal */
    uint64_t align;        /* alineación de la VA */
    uint8_t page_shift;    /* 12, 16 o 21 según el tamaño */
    uint8_t global;        /* compartido entre canales (upstream lo reusa) */
    uint8_t init;          /* RM lo inicializa: hay que darle la física */
    uint8_t ro;            /* se mapea de sólo lectura */
    /* Rellenados al reservar */
    uint64_t phys;         /* VRAM */
    uint64_t va;           /* 0 si no se mapea (el PRIV_ACCESS_MAP propio) */
    uint8_t nonmapped;
};

struct gsp_grctx {
    struct gsp_grctx_buf buf[GSP_GRCTX_MAX];
    unsigned nr;
    uint64_t va_next;
    uint64_t vram_bytes;   /* lo que se llevó de VRAM, para el log */
    int planned;
    int promoted;
};

/* Traduce la respuesta de `GET_CONTEXT_BUFFERS_INFO` en la lista de búferes con su
 * tamaño, página y alineación. Función pura: ni reserva ni habla con RM, así que la
 * aritmética —que es donde están las trampas— se prueba sin GPU. `engine_idx` es el
 * índice del motor de gráficos dentro de `engineContextBuffersInfo` (0 = GR0).
 * Devuelve el número de búferes, o -1. */
int gsp_grctx_plan(const NV2080_CTRL_INTERNAL_STATIC_GR_GET_CONTEXT_BUFFERS_INFO_PARAMS *info,
                   unsigned engine_idx, struct gsp_grctx *ctx);

/* Pide los tamaños a RM (sobre el subdevice) y llama a `gsp_grctx_plan`. */
int gsp_grctx_query(struct gsp_rm *rm, unsigned engine_idx, struct gsp_grctx *ctx);

/* Reserva cada búfer en VRAM, lo mapea en el vaspace y promociona el contexto del
 * canal `chan`. Devuelve 0 si RM aceptó la promoción. */
int gsp_grctx_promote(struct gsp_rm *rm, struct gsp_vmm *vmm, struct gsp_vram *vram,
                      struct gsp_chan *chan, struct gsp_grctx *ctx);

#endif
