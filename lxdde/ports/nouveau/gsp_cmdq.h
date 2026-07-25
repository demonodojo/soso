/* G4 paso 2: envío por la cola de comandos. Ver gsp_cmdq.c. */
#ifndef GSP_CMDQ_H
#define GSP_CMDQ_H

#include "gsp_rpc.h"
#include "nvrm_r570.h"

struct gsp_cmdq {
    volatile uint32_t *wptr;   /* lo escribimos nosotros (tx de la cmdq) */
    volatile uint32_t *rptr;   /* lo escribe el GSP (rx de la msgq) */
    unsigned char *cmdq;
    uint32_t cnt;
    uint32_t seq;
    int ready;
};

/* Lo que GSP-RM necesita saber de la máquina. Se recoge en el bring-up, que es
 * quien tiene el `lx_pci_dev`. */
struct gsp_sysinfo {
    uint64_t bar0_phys;
    uint64_t bar1_phys;
    uint64_t bar3_phys;
    uint64_t bdf;            /* domain:bus:dev.fn empaquetado como pci_dev_id */
    uint64_t max_user_va;
    uint32_t cfg_mirror_base;
    uint32_t cfg_mirror_size;
    uint16_t vendor_id;
    uint16_t device_id;
    uint16_t subvendor_id;
    uint16_t subdevice_id;
    uint8_t revision_id;
};

int gsp_cmdq_init(const struct gsp_libos *lo, struct gsp_cmdq *out);

/* Encola un RPC y toca el timbre. No espera respuesta. */
int gsp_cmdq_send(struct gsp_cmdq *q, uint32_t fn, const void *payload,
                  uint32_t payload_size);

/* Llamada síncrona: encola `fn` y espera la respuesta, que GSP-RM devuelve con
 * el **mismo `function`** (`r535_gsp_rpc_send` con espera). Los eventos que
 * lleguen mientras se registran y se descartan.
 *
 * `reply`/`reply_len` es opcional; `reply_got` recibe el tamaño real del payload
 * de respuesta y `status` su `NV_STATUS`. Devuelve 0 solo si la respuesta llegó
 * con `rpc_result == 0`. */
int gsp_cmdq_call(struct gsp_cmdq *q, struct gsp_rpc *rpc, uint32_t fn,
                  const void *payload, uint32_t payload_size,
                  void *reply, uint32_t reply_len, uint32_t *reply_got,
                  uint32_t *status, unsigned timeout_ms);

/* Las dos RPCs que GSP-RM consume durante su init; hay que encolarlas **antes**
 * de arrancar el GSP. */
int gsp_cmdq_set_system_info(struct gsp_cmdq *q, const struct gsp_sysinfo *si);
int gsp_cmdq_set_registry(struct gsp_cmdq *q);

#endif
