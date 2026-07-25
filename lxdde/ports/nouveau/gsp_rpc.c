/* G4 paso 1: recepción por la cola de mensajes de GSP-RM. Ver gsp_rpc.h. */
#include "gsp_rpc.h"
#include "gsp_mmio.h"

/* Falcon del GSP: `nvkm_falcon_ctor(..., 0x110000, ...)`, `addr2` = 0x1000. */
#define NV_PGSP_FALCON         0x00110000u
#define NV_PFALCON_MAILBOX0    (NV_PGSP_FALCON + 0x040u)
#define NV_PFALCON_OS          (NV_PGSP_FALCON + 0x080u)   /* app_version */
#define NV_PRISCV_RISCV_CPUCTL (NV_PGSP_FALCON + 0x1000u + 0x388u)
#define CPUCTL_ACTIVE_STAT     (1u << 7)   /* `ga102_flcn_riscv_active` */

#define GSP_MSG_HDR_SIZE  ((uint32_t)sizeof(struct gsp_msg_elem))
#define GSP_RPC_HDR_SIZE  ((uint32_t)sizeof(struct gsp_rpc_hdr))

/* El firmware escribe estas estructuras: si el layout se desvía, leemos basura. */
typedef char msg_elem_size_check[sizeof(struct gsp_msg_elem) == 48 ? 1 : -1];
typedef char rpc_hdr_size_check[sizeof(struct gsp_rpc_hdr) == 32 ? 1 : -1];

/* La memoria compartida está mapeada sin caché, pero el orden entre leer wptr y
 * publicar rptr importa: el GSP mira ese puntero para reutilizar las páginas. */
static void gsp_rpc_barrier(void)
{
    __asm__ __volatile__("mfence" ::: "memory");
}

int gsp_rpc_init(const struct gsp_libos *lo, struct gsp_rpc *out)
{
    struct gsp_msgq_headers *cmdq;
    struct gsp_msgq_headers *msgq;

    if (!lo || !out || !lo->ready || !lo->shm.va) {
        return -1;
    }
    cmdq = (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->cmdq_offset);
    msgq = (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);

    /* Punteros cruzados, como en `r535_gsp_shared_init`: cada cola escribe su
     * `tx.writePtr` y lee el `rx.readPtr` de la otra. */
    out->wptr = &msgq->tx.writePtr;
    out->rptr = &cmdq->rx.readPtr;
    out->msgq = (unsigned char *)msgq;
    out->cnt = cmdq->tx.msgCount;
    if (!out->cnt) {
        lx_printk("nouveau-lx: RPC sin entradas en el anillo\n");
        return -1;
    }
    out->ready = 1;
    return 0;
}

int gsp_rpc_start(uint32_t app_version)
{
    uint32_t cpuctl;

    /* `r535_gsp_init`: publicar la versión del bootloader y exigir RISC-V vivo. */
    gsp_mmio_wr32(NV_PFALCON_OS, app_version);

    cpuctl = gsp_mmio_rd32(NV_PRISCV_RISCV_CPUCTL);
    if (!(cpuctl & CPUCTL_ACTIVE_STAT)) {
        lx_printk("nouveau-lx: el RISC-V del GSP no está activo (cpuctl=0x%08x)\n", cpuctl);
        return -1;
    }
    lx_printk("nouveau-lx: RISC-V activo (cpuctl=0x%08x), app_version=0x%08x\n",
              cpuctl, app_version);
    return 0;
}

/* Entradas ocupadas del anillo. */
static uint32_t msgq_used(const struct gsp_rpc *rpc, uint32_t rptr)
{
    uint32_t wptr = *rpc->wptr;
    uint32_t used = wptr + rpc->cnt - rptr;

    if (used >= rpc->cnt) {
        used -= rpc->cnt;
    }
    return used;
}

/* La primera página de la cola es la cabecera; las entradas empiezan detrás. */
static const struct gsp_msg_elem *msgq_entry(const struct gsp_rpc *rpc, uint32_t rptr)
{
    return (const struct gsp_msg_elem *)(rpc->msgq + GSP_PAGE_SIZE +
                                         (unsigned long)rptr * GSP_PAGE_SIZE);
}

static const char *rpc_event_name(uint32_t fn)
{
    switch (fn) {
    case NV_VGPU_MSG_EVENT_GSP_INIT_DONE:
        return "GSP_INIT_DONE";
    case NV_VGPU_MSG_EVENT_UCODE_LIBOS_PRINT:
        return "UCODE_LIBOS_PRINT";
    default:
        return "?";
    }
}

int gsp_rpc_wait_event(struct gsp_rpc *rpc, uint32_t fn, unsigned timeout_ms)
{
    unsigned seen = 0;

    if (!rpc || !rpc->ready) {
        return -1;
    }
    while (timeout_ms--) {
        uint32_t rptr = *rpc->rptr;

        /* Hace falta al menos la página que lleva las dos cabeceras. */
        if (msgq_used(rpc, rptr) == 0) {
            if (!gsp_mmio_alive()) {
                lx_printk("nouveau-lx: la GPU se ha caído del bus esperando RPCs\n");
                return -1;
            }
            lx_mdelay(1);
            continue;
        }

        {
            const struct gsp_msg_elem *elem = msgq_entry(rpc, rptr);
            const struct gsp_rpc_hdr *hdr = (const struct gsp_rpc_hdr *)(elem + 1);
            uint32_t length = hdr->length;
            uint32_t function = hdr->function;
            uint32_t result = hdr->rpc_result;
            uint32_t pages;

            if (length < GSP_RPC_HDR_SIZE) {
                lx_printk("nouveau-lx: RPC con longitud imposible (%u)\n", length);
                return -1;
            }
            seen++;
            lx_printk("nouveau-lx: RPC fn=0x%04x (%s) len=%u res=0x%x\n",
                      function, rpc_event_name(function), length, result);

            /* Avanzar el puntero de lectura tantas páginas como ocupe. */
            pages = (length + GSP_MSG_HDR_SIZE + GSP_PAGE_SIZE - 1) / GSP_PAGE_SIZE;
            rptr = (rptr + pages) % rpc->cnt;
            gsp_rpc_barrier();
            *rpc->rptr = rptr;

            if (result) {
                lx_printk("nouveau-lx: GSP-RM devuelve error 0x%x en fn=0x%04x\n",
                          result, function);
                return -1;
            }
            if (function == fn) {
                lx_printk("nouveau-lx: %s recibido tras %u mensaje(s)\n",
                          rpc_event_name(fn), seen);
                return 0;
            }
        }
    }
    lx_printk("nouveau-lx: no llegó fn=0x%04x (%u mensaje(s) vistos)\n", fn, seen);
    return -1;
}
