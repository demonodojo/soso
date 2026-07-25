/* G4 paso 1: recepción por la cola de mensajes de GSP-RM. Ver gsp_rpc.h. */
#include "gsp_rpc.h"
#include "gsp_mmio.h"

/* Definido en el shim (lxdde/shim/src/shims.c). */
void *memcpy(void *dst, const void *src, unsigned long n);

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

/* La primera página de la cola es la cabecera; las entradas empiezan detrás.
 *
 * Copia `n` bytes del mensaje que empieza en la entrada `rptr`, desde el offset
 * `off` contado en el propio mensaje. El área de entradas es un anillo, así que
 * un mensaje de varias páginas puede continuar en la entrada 0: con un solo
 * `memcpy` leeríamos fuera del búfer. `GSP_INIT_DONE` (32 B) nunca lo destapó. */
static void ring_copy(const struct gsp_rpc *rpc, uint32_t rptr, uint32_t off,
                      void *dst, uint32_t n)
{
    unsigned char *d = (unsigned char *)dst;
    unsigned long ring = (unsigned long)rpc->cnt * GSP_PAGE_SIZE;
    unsigned long pos = ((unsigned long)rptr * GSP_PAGE_SIZE + off) % ring;

    while (n) {
        unsigned long chunk = ring - pos;

        if (chunk > n) {
            chunk = n;
        }
        memcpy(d, rpc->msgq + GSP_PAGE_SIZE + pos, chunk);
        d += chunk;
        pos = (pos + chunk) % ring;
        n -= (uint32_t)chunk;
    }
}

static const char *rpc_event_name(uint32_t fn)
{
    switch (fn) {
    case NV_VGPU_MSG_EVENT_GSP_INIT_DONE:
        return "GSP_INIT_DONE";
    case NV_VGPU_MSG_EVENT_GSP_RUN_CPU_SEQUENCER:
        return "GSP_RUN_CPU_SEQUENCER";
    case NV_VGPU_MSG_EVENT_OS_ERROR_LOG:
        return "OS_ERROR_LOG";
    case NV_VGPU_MSG_EVENT_UCODE_LIBOS_PRINT:
        return "UCODE_LIBOS_PRINT";
    case NV_VGPU_MSG_EVENT_GSP_LOCKDOWN_NOTICE:
        return "GSP_LOCKDOWN_NOTICE";
    case NV_VGPU_MSG_EVENT_GSP_POST_NOCAT_RECORD:
        return "GSP_POST_NOCAT_RECORD";
    case NV_VGPU_MSG_EVENT_FECS_ERROR:
        return "FECS_ERROR";
    case NV_VGPU_MSG_EVENT_RECOVERY_ACTION:
        return "RECOVERY_ACTION";
    default:
        return "?";
    }
}

/* Unos pocos códigos de NV_STATUS, los que se ven en el arranque. */
static const char *rpc_status_name(uint32_t status)
{
    switch (status) {
    case 0x51u: return "NO_MEMORY";
    case 0x55u: return "NOT_READY";
    case 0x56u: return "NOT_SUPPORTED";
    case 0x59u: return "OPERATING_SYSTEM";
    case 0x65u: return "TIMEOUT";
    case 0x66u: return "TIMEOUT_RETRY";
    default:    return "?";
    }
}

static void flush_repeats(unsigned *repeats)
{
    if (*repeats) {
        lx_printk("nouveau-lx: ... y %u más iguales\n", *repeats);
        *repeats = 0;
    }
}

int gsp_rpc_recv(struct gsp_rpc *rpc, uint32_t fn, void *out, uint32_t out_len,
                 uint32_t *payload_len, uint32_t *status, unsigned timeout_ms)
{
    unsigned seen = 0;
    unsigned nocat = 0;
    /* GSP-RM puede mandar cientos de eventos iguales seguidos (los registros
     * NOCAT cuando algo le va mal); imprimirlos uno a uno ahoga el serie. */
    uint32_t last_fn = 0xffffffffu;
    unsigned repeats = 0;
    uint32_t ring_bytes;

    if (!rpc || !rpc->ready) {
        return -1;
    }
    ring_bytes = rpc->cnt * GSP_PAGE_SIZE;

    while (timeout_ms--) {
        uint32_t rptr = *rpc->rptr;
        struct gsp_rpc_hdr hdr;
        uint32_t pages;
        int matched;

        /* Hace falta al menos la página que lleva las dos cabeceras. */
        if (msgq_used(rpc, rptr) == 0) {
            if (!gsp_mmio_alive()) {
                lx_printk("nouveau-lx: la GPU se ha caído del bus esperando RPCs\n");
                return -1;
            }
            lx_mdelay(1);
            continue;
        }

        /* La cabecera RPC nunca da la vuelta: el mensaje empieza en una página y
         * las dos cabeceras suman 80 B. El payload sí puede. */
        ring_copy(rpc, rptr, GSP_MSG_HDR_SIZE, &hdr, GSP_RPC_HDR_SIZE);

        if (hdr.length < GSP_RPC_HDR_SIZE ||
            hdr.length > ring_bytes - GSP_MSG_HDR_SIZE) {
            lx_printk("nouveau-lx: RPC con longitud imposible (%u)\n", hdr.length);
            return -1;
        }
        seen++;
        if (hdr.function == NV_VGPU_MSG_EVENT_GSP_POST_NOCAT_RECORD) {
            nocat++;
        }
        if (hdr.function == last_fn) {
            repeats++;
        } else {
            flush_repeats(&repeats);
            last_fn = hdr.function;
            lx_printk("nouveau-lx: RPC fn=0x%04x (%s) len=%u res=0x%x\n",
                      hdr.function, rpc_event_name(hdr.function), hdr.length,
                      hdr.rpc_result);
        }

        /* Copiar ANTES de mover el puntero: en cuanto lo publicamos, el GSP
         * puede reutilizar esas páginas. */
        matched = hdr.function == fn;
        if (matched) {
            uint32_t plen = hdr.length - GSP_RPC_HDR_SIZE;

            if (out && out_len) {
                ring_copy(rpc, rptr, GSP_MSG_HDR_SIZE + GSP_RPC_HDR_SIZE, out,
                          plen < out_len ? plen : out_len);
            }
            if (payload_len) {
                *payload_len = plen;
            }
            if (status) {
                *status = hdr.rpc_result;
            }
        }

        /* Avanzar el puntero de lectura tantas páginas como ocupe. */
        pages = (hdr.length + GSP_MSG_HDR_SIZE + GSP_PAGE_SIZE - 1) / GSP_PAGE_SIZE;
        rptr = (rptr + pages) % rpc->cnt;
        gsp_rpc_barrier();
        *rpc->rptr = rptr;

        /* Solo se vacía el contador al salir: si se hiciera aquí en cada
         * vuelta, cada repetición imprimiría su propia línea y no
         * agruparíamos nada. */
        if (hdr.rpc_result) {
            flush_repeats(&repeats);
            lx_printk("nouveau-lx: GSP-RM devuelve %s (0x%x) en fn=0x%04x (%s)\n",
                      rpc_status_name(hdr.rpc_result), hdr.rpc_result, hdr.function,
                      rpc_event_name(hdr.function));
            if (nocat) {
                /* `SET_SYSTEM_INFO`/`SET_REGISTRY` ya se encolan antes de
                 * arrancar (G4a). Si aun así llueven NOCAT, el problema está en
                 * su *contenido* —una apertura o un id mal puestos—, no en su
                 * ausencia. */
                lx_printk("nouveau-lx: %u registro(s) NOCAT antes del fallo — "
                          "revisa el contenido de SET_SYSTEM_INFO\n", nocat);
            }
            return -1;
        }
        if (matched) {
            flush_repeats(&repeats);
            lx_printk("nouveau-lx: %s recibido tras %u mensaje(s)\n",
                      rpc_event_name(fn), seen);
            return 0;
        }
    }
    flush_repeats(&repeats);
    lx_printk("nouveau-lx: no llegó fn=0x%04x (%u mensaje(s) vistos, %u NOCAT)\n",
              fn, seen, nocat);
    return -1;
}

int gsp_rpc_wait_event(struct gsp_rpc *rpc, uint32_t fn, unsigned timeout_ms)
{
    return gsp_rpc_recv(rpc, fn, NULL, 0, NULL, NULL, timeout_ms);
}
