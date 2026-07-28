/* G4 paso 1: recepción de mensajes de GSP-RM por la cola compartida.
 *
 * Con el GSP arrancado (G3b), GSP-RM habla con la CPU por las dos colas que montó
 * el paso 5. El primer mensaje que manda es `GSP_INIT_DONE`; verlo llegar prueba
 * de una vez el contrato entero de memoria compartida: la tabla de PTEs, los
 * offsets de las colas, los punteros cruzados de lectura/escritura y el formato
 * de los elementos.
 *
 * Esta parte **solo recibe**. Escribe únicamente el puntero de lectura, que vive
 * en nuestra propia memoria; de MMIO solo toca `app_version` antes de empezar.
 * El envío de RPCs (cmdq) es el siguiente escalón.
 *
 * Referencias upstream: `r535_gsp_msgq_{wait,peek,recv,get_entry}`,
 * `r535_gsp_msg_recv`, `r535_gsp_rpc_poll` (`nvkm/subdev/gsp/rm/r535/rpc.c`) y
 * `r535_gsp_init` (`rm/r535/gsp.c`).
 */
#ifndef GSP_RPC_H
#define GSP_RPC_H

#include "gsp_libos.h"

/* Cabecera del elemento de cola (`r535_gsp_msg`): metadatos de encolado. */
struct gsp_msg_elem {
    uint8_t auth_tag_buffer[16];
    uint8_t aad_buffer[16];
    uint32_t checksum;
    uint32_t sequence;
    uint32_t elem_count;
    uint32_t pad;
    /* data[] — detrás va la cabecera RPC */
};

/* Cabecera del RPC (`nvfw_gsp_rpc`). `length` incluye esta cabecera. */
struct gsp_rpc_hdr {
    uint32_t header_version;
    uint32_t signature;
    uint32_t length;
    uint32_t function;
    uint32_t rpc_result;
    uint32_t rpc_result_private;
    uint32_t sequence;
    uint32_t spare;
};

/* Eventos que manda GSP-RM. **Ojo: r535 y r570 renumeran a partir de 0x101c**
 * (en r535 el 0x101c es NVLINK_FAULT_UP; en r570, GSP_LOCKDOWN_NOTICE). Estos son
 * los de `rm/r570/nvrm/msgfn.h`, que es el que corresponde al firmware 570.144. */
#define NV_VGPU_MSG_EVENT_FIRST_EVENT           0x1000u
#define NV_VGPU_MSG_EVENT_GSP_INIT_DONE         0x1001u
#define NV_VGPU_MSG_EVENT_GSP_RUN_CPU_SEQUENCER 0x1002u
#define NV_VGPU_MSG_EVENT_OS_ERROR_LOG          0x1006u
#define NV_VGPU_MSG_EVENT_UCODE_LIBOS_PRINT     0x100cu
#define NV_VGPU_MSG_EVENT_GSP_LOCKDOWN_NOTICE   0x101cu
#define NV_VGPU_MSG_EVENT_GSP_POST_NOCAT_RECORD 0x1020u
#define NV_VGPU_MSG_EVENT_FECS_ERROR            0x1021u
#define NV_VGPU_MSG_EVENT_RECOVERY_ACTION       0x1022u

struct gsp_rpc {
    volatile uint32_t *rptr;   /* lo escribimos nosotros (rx de la cmdq) */
    volatile uint32_t *wptr;   /* lo escribe el GSP (tx de la msgq) */
    unsigned char *msgq;       /* base de la cola de mensajes */
    uint32_t cnt;              /* nº de entradas (páginas) del anillo */
    int ready;
};

/* Cablea los punteros del anillo a partir de la memoria compartida del paso 5. */
/* Nombre de un `NV_STATUS` de RM. Única tabla del port: la usan tanto el status
 * que viene dentro del wrapper de RM_ALLOC/RM_CONTROL como el `rpc_result` del
 * transporte. Nunca devuelve NULL; los desconocidos salen como "?". */
const char *nv_status_name(uint32_t st);

int gsp_rpc_init(const struct gsp_libos *lo, struct gsp_rpc *out);

/* Publica `app_version` y comprueba que el núcleo RISC-V está activo. */
int gsp_rpc_start(uint32_t app_version);

/* Consume mensajes hasta ver `fn` o agotar el tiempo. Los que llegan antes se
 * registran y se descartan. Devuelve 0 si llegó el esperado. */
int gsp_rpc_wait_event(struct gsp_rpc *rpc, uint32_t fn, unsigned timeout_ms);

/* Igual, pero copiando el payload del mensaje esperado a `out` (hasta `out_len`
 * bytes) — es la mitad receptora de una llamada síncrona a GSP-RM, que empareja
 * la respuesta **por `function`**, no por secuencia (`r535_gsp_msg_recv`).
 *
 * `payload_len` recibe el tamaño real del payload (puede ser mayor que `out_len`:
 * se copia lo que cabe) y `status` el `rpc_result` del mensaje. Ambos opcionales.
 * Devuelve 0 solo si llegó `fn` **y** su `rpc_result` es 0.
 *
 * A diferencia de `GSP_INIT_DONE` (32 B), las respuestas de RM pueden ocupar
 * varias páginas y **dar la vuelta al anillo**: la copia lo tiene en cuenta. */
int gsp_rpc_recv(struct gsp_rpc *rpc, uint32_t fn, void *out, uint32_t out_len,
                 uint32_t *payload_len, uint32_t *status, unsigned timeout_ms);

#endif
