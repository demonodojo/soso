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

/* Eventos que manda GSP-RM (`nvrm/msgfn.h`). */
#define NV_VGPU_MSG_EVENT_FIRST_EVENT        0x1000u
#define NV_VGPU_MSG_EVENT_GSP_INIT_DONE      0x1001u
#define NV_VGPU_MSG_EVENT_UCODE_LIBOS_PRINT  0x100cu

struct gsp_rpc {
    volatile uint32_t *rptr;   /* lo escribimos nosotros (rx de la cmdq) */
    volatile uint32_t *wptr;   /* lo escribe el GSP (tx de la msgq) */
    unsigned char *msgq;       /* base de la cola de mensajes */
    uint32_t cnt;              /* nº de entradas (páginas) del anillo */
    int ready;
};

/* Cablea los punteros del anillo a partir de la memoria compartida del paso 5. */
int gsp_rpc_init(const struct gsp_libos *lo, struct gsp_rpc *out);

/* Publica `app_version` y comprueba que el núcleo RISC-V está activo. */
int gsp_rpc_start(uint32_t app_version);

/* Consume mensajes hasta ver `fn` o agotar el tiempo. Los que llegan antes se
 * registran y se descartan. Devuelve 0 si llegó el esperado. */
int gsp_rpc_wait_event(struct gsp_rpc *rpc, uint32_t fn, unsigned timeout_ms);

#endif
