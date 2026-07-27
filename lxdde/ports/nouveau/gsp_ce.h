/* G4e (2/2): motor de copia (CE) sobre el canal GPFIFO.
 *
 * Reserva `AMPERE_DMA_COPY_A`, construye un pushbuffer con copia lineal virtual
 * y lo encola. En hardware la verificación es sysmem → VRAM → sysmem; en
 * hostcheck solo se valida el layout del PB y las peticiones RM.
 */
#ifndef GSP_CE_H
#define GSP_CE_H

#include "gsp_chan.h"

struct gsp_ce {
    struct gsp_rm *rm;
    struct gsp_chan *chan;
    uint32_t handle;
    int ready;
};

int gsp_ce_init(struct gsp_rm *rm, struct gsp_chan *chan, struct gsp_ce *ce);

/* Codifica una copia lineal virtual src→dst de `size` bytes en el pushbuffer.
 * Devuelve el offset del PB o -1. */
int gsp_ce_encode_copy(struct gsp_ce *ce, uint64_t dst_va, uint64_t src_va,
                       uint32_t size, unsigned *pb_off, unsigned *pb_len);

/* Copia de prueba G4e: scratch → VRAM → scratch y compara el patrón.
 * En hardware exige canal+CE vivos; devuelve 0 solo si el readback cuadra.
 * Sin GPU real devuelve -1 (no miente con on_gpu). */
int gsp_ce_selftest(struct gsp_ce *ce, uint64_t scratch_va, uint64_t vram_va,
                    void *scratch_cpu, uint32_t size);

void gsp_ce_fini(struct gsp_ce *ce);

#endif
