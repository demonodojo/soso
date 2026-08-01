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

/* Ventana de VAs para tensores residentes, separada de G4f/G5. */
#define G6_VA_BASE   (GSP_VA_BASE + 0x40000000ull)
#define G6_VA_LIMIT  (GSP_VA_BASE + 0x50000000ull) /* 256 MiB de ventana */

/* Búfer de rebote de las subidas: sysmem contigua, cacheada, con su propia VA.
 * 1 MiB = 256 páginas, una sola tabla hoja. El tamaño manda de verdad: la subida
 * es un LAUNCH_DMA y una espera de semáforo POR BÚFER, así que con 4 KiB un
 * tensor de 64 MiB eran 16384 idas y vueltas y con esto son 64. */
#define G6_BOUNCE_VA     (GSP_VA_BASE + 0x30400000ull)
#define G6_BOUNCE_BYTES  0x100000u

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

/* Libera la VA; devuelve 0 si ok, -1 si no existía. */
int gsp_buf_free(struct gsp_buf *b, uint64_t va);

/* Bytes repartibles aún sin reservar (total - used del pool). */
uint64_t gsp_buf_vram_free(const struct gsp_buf *b);

void gsp_buf_fini(struct gsp_buf *b);

#endif
