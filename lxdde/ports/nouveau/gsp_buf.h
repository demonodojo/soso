/* G6: buffers de usuario en VRAM (pesos residentes).
 *
 * Sin BAR1 la CPU no escribe VRAM: se reserva física con `gsp_vram_alloc`, se
 * mapea en el vaspace del canal y las subidas van por CE desde sysmem. Las
 * liberaciones devuelven el bloque a una free-list sobre el bump allocator. */
#ifndef GSP_BUF_H
#define GSP_BUF_H

#include "gsp_ce.h"
#include "gsp_vmm.h"
#include "gsp_vram.h"

/* Ventana de VAs para tensores residentes, separada de G4f/G5 **y del contexto
 * de GR**, que es lo que no estaba y costó tres días.
 *
 * AVERÍA (2026-08-02): esto estaba en `GSP_VA_BASE + 0x40000000`, que es
 * exactamente `GSP_GRCTX_VA_BASE`, y la ventana del grctx mide 1 GiB
 * (`GSP_GRCTX_VA_SIZE`). O sea que los pesos residentes se mapeaban ENCIMA de
 * los búferes de contexto de GR que la promoción acababa de colocar ahí (en el
 * log salían en `va=0x8047260000` y compañía, dentro de este rango).
 *
 * El síntoma no se parecía en nada a la causa: el matvec con pesos en VRAM
 * levantaba un `GR_EXCEPTION` **sin falta de MMU** —las VAs estaban mapeadas, sí:
 * a las páginas equivocadas— y el volcado de registros que RM adjunta al RC
 * apuntaba al CTXCTL (`0x400100` bit 19 = `gf100_gr_ctxctl_isr` en nouveau), o
 * sea a FECS salvando y restaurando un contexto que ya no estaba. La ruta
 * escalonada seguía funcionando porque no toca esta ventana.
 *
 * Detrás del grctx, no delante: el attribute CB pide alineación a su propio
 * tamaño redondeado a potencia de dos y esa ventana puede crecer con el chip. */
#define G6_VA_BASE   (GSP_VA_BASE + 0x80000000ull)
#define G6_VA_LIMIT  (GSP_VA_BASE + 0x90000000ull) /* 256 MiB de ventana */

/* Búfer de rebote de las subidas: sysmem contigua, cacheada, con su propia VA.
 * 1 MiB = 256 páginas, una sola tabla hoja. El tamaño manda de verdad: la subida
 * es un LAUNCH_DMA y una espera de semáforo POR BÚFER, así que con 4 KiB un
 * tensor de 64 MiB eran 16384 idas y vueltas y con esto son 64. */
#define G6_BOUNCE_VA     (GSP_VA_BASE + 0x30400000ull)
#define G6_BOUNCE_BYTES  0x100000u

/* Ventana para el ORIGEN de las subidas por DMA: aquí se mapean las páginas del
 * propio búfer del proceso y el CE lee de ellas, sin copia intermedia. Se
 * remapea en cada lote (`gsp_vmm_map` ya invalida la TLB de la MMU al terminar,
 * y no hay `unmap` que hacer).
 *
 * El hueco: el rebote acaba en +0x30500000 y el grctx empieza en +0x40000000,
 * así que 16 MiB en +0x31000000 quedan lejos de los dos. NO usar la zona de
 * +0x40000000: ahí está el grctx y pisarlo da un GR_EXCEPTION sin falta de MMU
 * (ver la avería del 2026-08-02 arriba).
 *
 * 16 MiB de lote es un equilibrio: la lista de físicas que el kernel prepara son
 * 8 bytes por página (4096 entradas), y cada lote es UN LAUNCH_DMA con una sola
 * espera de semáforo — contra los 44 que salían subiendo de MiB en MiB. */
#define G6_SRC_VA        (GSP_VA_BASE + 0x31000000ull)
#define G6_SRC_MAX       (16ull * 1024ull * 1024ull)

struct gsp_buf {
    struct gsp_vram *vram;
    struct gsp_vmm *vmm;
    struct gsp_ce *ce;
    uint64_t scratch_va;
    void *scratch_cpu;
    unsigned scratch_bytes;
    uint64_t va_next;
    int ready;
};

struct gsp_buf_slot {
    uint64_t va;
    uint64_t phys;
    uint64_t size;
    int in_use;
};

#define G6_MAX_SLOTS 64u

int gsp_buf_init(struct gsp_buf *b, struct gsp_vram *vram, struct gsp_vmm *vmm,
                 struct gsp_ce *ce, uint64_t scratch_va, void *scratch_cpu,
                 unsigned scratch_bytes);

/* Reserva VRAM+VA; devuelve la VA del tensor o 0. */
uint64_t gsp_buf_alloc(struct gsp_buf *b, uint64_t size);

/* Copia `size` bytes desde `src` (sysmem CPU) a la VA residente. */
int gsp_buf_upload(struct gsp_buf *b, uint64_t va, const void *src, uint64_t size);

/* Igual, pero a `offset` bytes del principio del búfer. Existe para que el
 * kernel pueda subir un tensor a trozos: sin offset tenía que copiarlo ENTERO a
 * un temporal de su heap antes de llamar aquí (44 MiB por subida en TinyLlama),
 * porque esta función sólo aceptaba la VA base del slot. `offset` ha de ser
 * múltiplo de página, como `size` salvo el rabo final. */
int gsp_buf_upload_at(struct gsp_buf *b, uint64_t va, uint64_t offset,
                      const void *src, uint64_t size);

/* Sube SIN copia de CPU: `phys` son las físicas de las páginas del origen (en
 * orden), se mapean en `G6_SRC_VA` y el CE copia de ahí a la VRAM del slot.
 *
 * `src_off` (0..4095) es lo que le falta al origen para empezar en frontera de
 * página: el CE lee de `G6_SRC_VA + src_off` y las páginas dadas han de cubrir
 * `src_off + size`. **No es un lujo**: el payload de un shard `.som` empieza en el
 * byte 64 del fichero, así que el puntero de cualquier tensor mapeado llega en
 * +64, y mientras esto exigió alineación de página el camino sin copias no se
 * disparó ni una vez con pesos de verdad (2026-08-17 → 2026-08-17).
 *
 * El DESTINO sí tiene que estar alineado a página, y `size` ser múltiplo de página
 * salvo un único rabo final: es lo probado en silicio de la copia multilínea del
 * CE (`gsp_ce_encode_copy`). Si el llamante no puede cumplirlo, tiene
 * `gsp_buf_upload_at`, que rebota por sysmem propia.
 *
 * Devuelve 0, o -1 sin haber tocado el CE si algo no encaja (y entonces el
 * llamante puede rebotar). */
int gsp_buf_upload_dma(struct gsp_buf *b, uint64_t va, uint64_t offset,
                       const uint64_t *phys, unsigned npages, unsigned src_off,
                       uint64_t size);

/* Libera la VA; devuelve 0 si ok, -1 si no existía. */
int gsp_buf_free(struct gsp_buf *b, uint64_t va);

/* Bytes repartibles aún sin reservar (total - used del pool). */
uint64_t gsp_buf_vram_free(const struct gsp_buf *b);

void gsp_buf_fini(struct gsp_buf *b);

#endif
