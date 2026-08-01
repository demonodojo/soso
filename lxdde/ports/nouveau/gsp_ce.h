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

/* Longitud de línea de las copias multilínea. Es el tamaño de página porque es
 * el que usa `nve0_bo_move_copy` y el que garantiza que src y dst caen en
 * páginas enteras del vaspace. */
#define GSP_CE_LINE_BYTES  4096u

/* Sondeo con reloj fino antes de dormir por ticks, igual que el QMD. Con el tick
 * a 100 Hz, cada `lx_mdelay(1)` cuesta 10 ms: una subida de 64 MiB son 64 copias
 * y eso era más de medio segundo de puro dormir. */
#define GSP_CE_SPIN_US     2000u

/* Cuánto se escucha el anillo de mensajes cuando una espera vence. RM cuenta los
 * fallos de canal por evento (RC_TRIGGERED: tipo de excepción, chid y dirección
 * de la falta de MMU) y si nadie escucha se quedan en la cola: el ciclo del
 * 2026-07-30 se fue sin saber por qué murió el canal por no drenar aquí. */
#define GSP_CE_RC_DRAIN_MS  300u

struct gsp_ce {
    struct gsp_rm *rm;
    struct gsp_chan *chan;
    uint32_t handle;
    uint32_t cls;      /* la que aceptó RM; la elige el catálogo del chip */
    uint32_t seq;      /* payload del semáforo: uno por copia, monótono */
    uint32_t pending;  /* payload de la última copia encolada */
    int ready;
    /* Tras un timeout o pushbuffer irrecuperable: no reencolar. Un CE atascado
     * (gpget fijo) llena el log con "subida CE falló" por cada matvec si se
     * sigue intentando — G6 lo vio en silicio el 2026-07-30. */
    int stuck;
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

/* Escucha el anillo de mensajes de GSP-RM y registra lo que haya (RC_TRIGGERED,
 * NOCAT…). Sólo para llamar cuando algo ha vencido: nunca en medio de una
 * llamada síncrona, porque consumiría su respuesta. */
void gsp_ce_drain_events(struct gsp_ce *ce);

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
