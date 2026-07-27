/* G4e (2/2): motor de copia (CE) sobre el canal GPFIFO.
 *
 * Reserva `AMPERE_DMA_COPY_A`, construye un pushbuffer con copia lineal virtual
 * y lo encola. En hardware la verificación es sysmem → VRAM → sysmem; en
 * hostcheck solo se valida el layout del PB y las peticiones RM.
 */
#ifndef GSP_CE_H
#define GSP_CE_H

#include "gsp_chan.h"

/* Timeout de cada copia. Generoso: un CE que no responde en 2 s no va a
 * responder, y quedarse esperando durante el bring-up es peor que fallar. */
#define GSP_CE_WAIT_MS  2000u

struct gsp_ce {
    struct gsp_rm *rm;
    struct gsp_chan *chan;
    uint32_t handle;
    uint32_t seq;      /* payload del semáforo: uno por copia, monótono */
    uint32_t pending;  /* payload de la última copia encolada */
    int ready;
};

int gsp_ce_init(struct gsp_rm *rm, struct gsp_chan *chan, struct gsp_ce *ce);

/* Codifica una copia lineal virtual src→dst de `size` bytes en el pushbuffer.
 * Devuelve el offset del PB o -1. El semáforo de la copia lleva `++ce->seq`
 * como payload: con un payload fijo, la segunda copia no se distingue de la
 * primera y el `wait` pasa sin que el CE haya hecho nada. */
int gsp_ce_encode_copy(struct gsp_ce *ce, uint64_t dst_va, uint64_t src_va,
                       uint32_t size, unsigned *pb_off, unsigned *pb_len);

/* Espera a que el semáforo del canal alcance el payload de la última copia.
 * Devuelve 0 si llegó, -1 si venció el plazo. */
int gsp_ce_wait(struct gsp_ce *ce, unsigned ms);

/* encode + submit + wait. Devuelve 0 solo si el CE señalizó. */
int gsp_ce_copy_sync(struct gsp_ce *ce, uint64_t dst_va, uint64_t src_va,
                     uint32_t size, unsigned ms);

/* Copia de prueba G4e: scratch → VRAM → scratch y compara el patrón.
 * En hardware exige canal+CE vivos; devuelve 0 solo si el readback cuadra.
 * Sin GPU real devuelve -1 (no miente con on_gpu). */
int gsp_ce_selftest(struct gsp_ce *ce, uint64_t scratch_va, uint64_t vram_va,
                    void *scratch_cpu, uint32_t size);

void gsp_ce_fini(struct gsp_ce *ce);

#endif
