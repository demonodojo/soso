/* G4 paso 1: recepción por la cola de mensajes de GSP-RM. Ver gsp_rpc.h. */
#include "gsp_rpc.h"
#include "gsp_mmio.h"
#include "gsp_cpu_seq.h"
#include "nvrm_r570.h"   /* NV_VGPU_MSG_FUNCTION_*, para nombrar lo que mandamos */

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

    /* `r535_gsp_init`: publicar app_version; el juez es GSP_INIT_DONE (puede
     * pedir CORE_RESUME por msgq si RISC-V aún está en halt). */
    gsp_mmio_wr32(NV_PFALCON_OS, app_version);

    cpuctl = gsp_mmio_rd32(NV_PRISCV_RISCV_CPUCTL);
    if (cpuctl & CPUCTL_ACTIVE_STAT) {
        lx_printk("nouveau-lx: RISC-V activo (cpuctl=0x%08x), app_version=0x%08x\n",
                  cpuctl, app_version);
    } else {
        lx_printk("nouveau-lx: RISC-V inactivo (cpuctl=0x%08x), app_version=0x%08x "
                  "— poll RPC/msgq\n",
                  cpuctl, app_version);
    }
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
    case NV_VGPU_MSG_EVENT_RC_TRIGGERED:
        return "RC_TRIGGERED";
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

/* Nombres de las funciones que mandamos nosotros. El log solo sabía traducir
 * los eventos que llegan de GSP-RM, así que las respuestas a nuestras propias
 * llamadas salían como `fn=0x0041 (?)` — y averiguar que ese 0x41 era
 * GET_GSP_STATIC_INFO costó más de lo que debería. */
static const char *rpc_function_name(uint32_t fn)
{
    switch (fn) {
    case NV_VGPU_MSG_FUNCTION_FREE:
        return "FREE";
    case NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER:
        return "UNLOADING_GUEST_DRIVER";
    case NV_VGPU_MSG_FUNCTION_GET_GSP_STATIC_INFO:
        return "GET_GSP_STATIC_INFO";
    case NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL:
        return "GSP_RM_CONTROL";
    case NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC:
        return "GSP_RM_ALLOC";
    default:
        return rpc_event_name(fn);
    }
}

/* Nombres de `NV_STATUS`, verificados uno a uno contra `nvstatuscodes.h` de
 * open-gpu-kernel-modules (2026-07-28). Vivía en gsp_rm_obj.c, y aquí había una
 * segunda tabla más corta con los mismos códigos: el 0x22 (INVALID_CLASS) del
 * RM_ALLOC de compute salió impreso como "?" porque estaba en la otra. Una sola,
 * y las dos rutas —el status de dentro del wrapper y el `rpc_result` del
 * transporte— tiran de ella.
 *
 * Dos de los nombres que hubo aquí estaban inventados: el 0x2b no es
 * INVALID_CLASS sino INVALID_HEAP, y el 0x2f no es INVALID_OBJECT_PARENT sino
 * INVALID_LOCK_STATE. Los de verdad son 0x22 y 0x36. Un nombre falso en un
 * mensaje de error manda el diagnóstico al lado contrario. */
const char *nv_status_name(uint32_t st)
{
    switch (st) {
    case 0x00u: return "OK";
    case 0x1eu: return "INVALID_ADDRESS";
    case 0x1fu: return "INVALID_ARGUMENT";
    case 0x21u: return "INVALID_CHANNEL";
    case 0x22u: return "INVALID_CLASS";
    case 0x23u: return "INVALID_CLIENT";
    case 0x26u: return "INVALID_DEVICE";
    case 0x29u: return "INVALID_FLAGS";
    case 0x2bu: return "INVALID_HEAP";
    case 0x2fu: return "INVALID_LOCK_STATE";
    case 0x31u: return "INVALID_OBJECT";
    case 0x33u: return "INVALID_OBJECT_HANDLE";
    case 0x36u: return "INVALID_OBJECT_PARENT";
    case 0x37u: return "INVALID_OFFSET";
    case 0x3au: return "INVALID_PARAM_STRUCT";
    case 0x3bu: return "INVALID_PARAMETER";
    case 0x40u: return "INVALID_STATE";
    case 0x4fu: return "NO_FREE_FIFOS";
    case 0x51u: return "NO_MEMORY";
    case 0x55u: return "NOT_READY";
    case 0x56u: return "NOT_SUPPORTED";
    case 0x57u: return "OBJECT_NOT_FOUND";
    case 0x58u: return "OBJECT_TYPE_MISMATCH";
    case 0x59u: return "OPERATING_SYSTEM";
    case 0x65u: return "TIMEOUT";
    case 0x66u: return "TIMEOUT_RETRY";
    default:    return "?";
    }
}

/* Códigos de `rpc_result`. Hay DOS familias y confundirlas cuesta caro:
 *
 *  - por debajo de 0xff000000 es un NV_STATUS normal de RM (el 0x59 que salió
 *    en G4a, por ejemplo);
 *  - 0xff1000xx es `NV_VGPU_MSG_RESULT__RPC` (`rpc_headers.h` de
 *    open-gpu-kernel-modules): el mensaje ni llegó a RM, lo rechazó el
 *    transporte. Es un problema de cómo lo hemos construido nosotros, no de lo
 *    que hemos pedido.
 *
 * El 0xff100002 de G4d era del segundo grupo y el log lo enseñaba como "?". */
static const char *rpc_status_name(uint32_t status)
{
    switch (status) {
    case 0xff100001u: return "RPC_UNKNOWN_FUNCTION";
    case 0xff100002u: return "RPC_INVALID_MESSAGE_FORMAT";
    case 0xff100003u: return "RPC_HANDLE_NOT_FOUND";
    case 0xff100004u: return "RPC_HANDLE_EXISTS";
    case 0xff100005u: return "RPC_UNKNOWN_RM_ERROR";
    case 0xff100006u: return "RPC_UNKNOWN_VMIOP_ERROR";
    case 0xff100007u: return "RPC_RESERVED_HANDLE";
    /* Por debajo de 0xff000000 es un NV_STATUS y lo nombra la tabla de arriba.
     * Había DOS tablas —una aquí y otra en gsp_rm_obj.c— y el 0x22 del compute
     * salió como "?" porque estaba sólo en la otra (2026-07-28). Dos tablas de
     * lo mismo divergen siempre; ahora hay una. */
    default:    return nv_status_name(status);
    }
}

/* Cuántos NOCAT DISTINTOS se vuelcan por arranque. RM los suelta a cientos cuando
 * algo le va mal (G4a los vio en avalancha), así que volcarlos todos ahoga el
 * serie. Pero volcar "los dos primeros" tampoco valía: el 2026-07-28 los dos
 * primeros eran **el mismo aviso repetido** y los tres siguientes —entre ellos el
 * que acompañaba al NO_MEMORY del canal de GR0— se cayeron por el tope. Se
 * desduplica por contenido y así el tope cuenta avisos, no copias. */
#define GSP_NOCAT_DUMP_MAX   6u
#define GSP_NOCAT_DUMP_WORDS 24u

static unsigned g_nocat_dumped;
static uint32_t g_nocat_seen[GSP_NOCAT_DUMP_MAX];

/* Huella del registro **saltando la palabra 2**, que es un contador de tiempo
 * (0x0c133d60 y 0x27c14360 en dos copias del mismo aviso): con ella dentro, cada
 * copia sería única y la desduplicación no serviría de nada. FNV-1a sobre las
 * primeras 96 palabras, que es donde están el tipo, el motor y las cadenas. */
static uint32_t nocat_fingerprint(const unsigned char *p, uint32_t len)
{
    uint32_t h = 2166136261u;
    uint32_t i;
    uint32_t n = len < 96u ? len : 96u;

    for (i = 0; i < n; i++) {
        if (i >= 8u && i < 12u) {
            continue;
        }
        h = (h ^ (uint32_t)p[i]) * 16777619u;
    }
    return h;
}

/* 1 si este aviso no se ha visto todavía y queda hueco para volcarlo. */
static int nocat_is_new(const unsigned char *p, uint32_t len)
{
    uint32_t fp = nocat_fingerprint(p, len);
    unsigned i;

    for (i = 0; i < g_nocat_dumped; i++) {
        if (g_nocat_seen[i] == fp) {
            return 0;
        }
    }
    if (g_nocat_dumped >= GSP_NOCAT_DUMP_MAX) {
        return 0;
    }
    g_nocat_seen[g_nocat_dumped] = fp;
    return 1;
}
/* Estático y no en la pila: el registro son ~1,2 KiB y la pila del bring-up no
 * está para eso. Aquí sólo hay una fibra tocando el RPC. */
static unsigned char g_nocat_buf[1536];
static unsigned char g_cpu_seq_buf[4096];

static void cpu_seq_capture(const struct gsp_rpc *rpc, uint32_t rptr, uint32_t length)
{
    uint32_t plen = length - GSP_RPC_HDR_SIZE;

    if (plen > (uint32_t)sizeof(g_cpu_seq_buf)) {
        plen = (uint32_t)sizeof(g_cpu_seq_buf);
    }
    ring_copy(rpc, rptr, GSP_MSG_HDR_SIZE + GSP_RPC_HDR_SIZE, g_cpu_seq_buf, plen);
    (void)gsp_cpu_seq_run(g_cpu_seq_buf, plen);
}

/* El registro NOCAT es un `NV2080_NOCAT_JOURNAL_ENTRY` y su layout NO está en
 * nuestras cabeceras. Bautizar campos a ojo es cómo se acaba leyendo un offset
 * por otro (el enum de r535 apuntando a NVLink, o los tres nombres inventados de
 * `rm_status_hint`), así que esto no interpreta: vuelca las primeras palabras en
 * crudo y saca las cadenas, que se identifican solas — RM mete ahí el motor, el
 * código de error y la aserción con fichero y línea. */
static void nocat_dump(const unsigned char *p, uint32_t len)
{
    char run[65];
    unsigned n = 0;
    uint32_t i;
    uint32_t words = len / 4u;

    if (words > GSP_NOCAT_DUMP_WORDS) {
        words = GSP_NOCAT_DUMP_WORDS;
    }
    for (i = 0; i < words; i += 4u) {
        uint32_t w[4] = { 0, 0, 0, 0 };
        uint32_t j;

        for (j = 0; j < 4u && i + j < words; j++) {
            memcpy(&w[j], p + (i + j) * 4u, 4);
        }
        lx_printk("nouveau-lx: NOCAT[%u] +0x%03x: %08x %08x %08x %08x\n",
                  g_nocat_dumped, (unsigned)(i * 4u), w[0], w[1], w[2], w[3]);
    }

    /* Cadenas de 4 caracteres o más. Menos que eso es ruido binario que casa con
     * ASCII por casualidad y ensucia lo que sí es un mensaje. */
    for (i = 0; i <= len; i++) {
        int c = i < len ? p[i] : 0;

        if (c >= 0x20 && c < 0x7f) {
            if (n < sizeof(run) - 1u) {
                run[n++] = (char)c;
            }
            continue;
        }
        if (n >= 4u) {
            run[n] = '\0';
            lx_printk("nouveau-lx: NOCAT[%u] texto: %s\n", g_nocat_dumped, run);
        }
        n = 0;
    }
    g_nocat_dumped++;
}

/* Copia el registro mientras el mensaje sigue siendo nuestro (antes de publicar el
 * rptr) y lo vuelca si es un aviso que no habíamos visto. */
static void nocat_capture(const struct gsp_rpc *rpc, uint32_t rptr, uint32_t length)
{
    uint32_t plen = length - GSP_RPC_HDR_SIZE;

    if (g_nocat_dumped >= GSP_NOCAT_DUMP_MAX) {
        return;
    }
    if (plen > (uint32_t)sizeof(g_nocat_buf)) {
        plen = (uint32_t)sizeof(g_nocat_buf);
    }
    ring_copy(rpc, rptr, GSP_MSG_HDR_SIZE + GSP_RPC_HDR_SIZE, g_nocat_buf, plen);
    if (nocat_is_new(g_nocat_buf, plen)) {
        nocat_dump(g_nocat_buf, plen);
    }
}

static void flush_repeats(unsigned *repeats)
{
    if (*repeats) {
        lx_printk("nouveau-lx: ... y %u más iguales\n", *repeats);
        *repeats = 0;
    }
}

/* Códigos `ROBUST_CHANNEL_*` de `nverror.h` (OGKM 570.144). Solo los que puede
 * ver este bring-up; el resto sale como "?". */
static const char *rc_except_name(uint32_t type)
{
    switch (type) {
    case 13u: return "GR_EXCEPTION";
    case 31u: return "FIFO_ERROR_MMU_ERR_FLT";
    case 32u: return "PBDMA_ERROR";
    case 39u: return "CE0_ERROR";
    case 44u: return "GR_FAULT_DURING_CTXSW";
    case 69u: return "GR_CLASS_ERROR";
    case 79u: return "GPU_HAS_FALLEN_OFF_THE_BUS";
    default:  return "?";
    }
}

/* `NV_n_*` de dev_fault.h gb202 — lo que trae el RC en mmuFaultType. */
const char *mmu_fault_type_name(uint32_t type)
{
    switch (type) {
    case 0u:  return "PDE";
    case 1u:  return "PDE_SIZE";
    case 2u:  return "PTE";
    case 3u:  return "VA_LIMIT_VIOLATION";
    case 4u:  return "UNBOUND_INST_BLOCK";
    case 5u:  return "PRIV_VIOLATION";
    case 6u:  return "RO_VIOLATION";
    case 7u:  return "WO_VIOLATION";
    case 8u:  return "PITCH_MASK_VIOLATION";
    case 9u:  return "WORK_CREATION";
    case 10u: return "UNSUPPORTED_APERTURE";
    case 11u: return "CC_VIOLATION";
    case 12u: return "UNSUPPORTED_KIND";
    case 13u: return "REGION_VIOLATION";
    case 14u: return "POISONED";
    case 15u: return "ATOMIC_VIOLATION";
    default:  return "?";
    }
}

/* Nombre del tipo de registro del journal (`RMCD_RECORD_TYPE`, r570 rmcd.h). */
static const char *rcd_record_name(unsigned tipo)
{
    switch (tipo) {
    case 138u: return "BugCheck";
    case 139u: return "SwRmAssert";
    case 140u: return "GpuTimeout";
    case 141u: return "SwDbgBreakpoint";
    case 142u: return "BadRead";
    case 143u: return "SurpriseRemoval";
    case 144u: return "PowerState";
    case 145u: return "PrbErrorInfo";
    case 146u: return "PrbFullDump";
    case RMCD_RECORD_RCDIAGREPORT: return "RcDiagReport";
    case RMCD_RECORD_NOCATREPORT:  return "NocatReport";
    case 150u: return "DispState";
    default:   return "?";
    }
}

void gsp_rpc_rc_journal_log(const unsigned char *j, uint32_t hay, uint32_t total)
{
    NVCD_RECORD_hdr hdr;
    unsigned tipo;
    uint32_t off;
    unsigned mostradas = 0;

    if (!j || hay < sizeof(hdr)) {
        return;
    }
    memcpy(&hdr, j, sizeof(hdr));
    tipo = hdr.cRecordType;
    lx_printk("nouveau-lx: rc: journal %u B (llegan %u) — registro grupo=%u "
              "tipo=%u (%s) tam=%u\n", total, hay, hdr.cRecordGroup, tipo,
              rcd_record_name(tipo), hdr.wRecordSize);

    if (tipo != RMCD_RECORD_RCDIAGREPORT) {
        return;
    }
    /* El cuerpo del RcDiagReport son entradas `{offset, tag, value, attribute}`:
     * el volcado de registros que RM leyó cuando saltó la excepción. Empiezan
     * detrás de la cabecera común y del encabezado del propio registro; en vez
     * de fiarlo todo al padding exacto de dos structs con NvU16 y NvU32
     * mezclados, se BUSCA el principio: una entrada de verdad tiene un offset
     * que parece un registro (dentro de BAR0, no nulo) y esa comprobación es más
     * robusta que contar bytes a ciegas. */
    off = (uint32_t)(sizeof(RmRCCommonJournal_RECORD_hdr) +
                     sizeof(RmRcDiag_RECORD_hdr));
    off &= ~3u;
    for (; off + sizeof(RmRcDiagRecordEntry) <= hay && mostradas < 48u;
         off += (uint32_t)sizeof(RmRcDiagRecordEntry)) {
        RmRcDiagRecordEntry e;

        memcpy(&e, j + off, sizeof(e));
        if (!e.offset || e.offset >= 0x01000000u) {
            continue;   /* no parece un registro de BAR0 */
        }
        lx_printk("nouveau-lx: rc: reg 0x%06x = 0x%08x (tag=%u attr=0x%x)\n",
                  e.offset, e.value, e.tag, e.attribute);
        mostradas++;
    }
    if (!mostradas) {
        const uint32_t *w = (const uint32_t *)(const void *)j;
        unsigned i;

        lx_printk("nouveau-lx: rc: sin entradas reconocibles; crudo:\n");
        for (i = 0; i + 8u <= hay / 4u; i += 8u) {
            if (i >= 32u) {
                break;
            }
            lx_printk("nouveau-lx: rc: +0x%02x: %08x %08x %08x %08x  %08x %08x "
                      "%08x %08x\n", i * 4u, w[i], w[i + 1], w[i + 2], w[i + 3],
                      w[i + 4], w[i + 5], w[i + 6], w[i + 7]);
        }
    }
}

int gsp_rpc_rc_triggered_log(const void *payload, uint32_t len)
{
    const rpc_rc_triggered_v17_02 *msg = payload;
    uint64_t fault;

    if (!payload || len < sizeof(rpc_rc_triggered_v17_02)) {
        return -1;
    }
    fault = ((uint64_t)msg->mmuFaultAddrHi << 32) | (uint64_t)msg->mmuFaultAddrLo;
    lx_printk("nouveau-lx: rc: engn=%08x chid=%u type=%u (%s) scope=%u part=%u\n",
              msg->nv2080EngineType, msg->chid, msg->exceptType,
              rc_except_name(msg->exceptType), msg->scope,
              (unsigned)msg->partitionAttributionId);
    if (fault || msg->mmuFaultType) {
        lx_printk("nouveau-lx: rc: mmuFault=0x%llx type=%u (%s)\n",
                  (unsigned long long)fault, msg->mmuFaultType,
                  mmu_fault_type_name(msg->mmuFaultType));
    }
    /* Cabecera del journal de RC (2026-07-29): el subtipo exacto del error —qué
     * método/dato atragantó al PBDMA, o qué excepción levantó GR— viaja aquí y no
     * en los campos fijos.
     *
     * Ocho palabras no bastaron: con el GR_EXCEPTION de la GB205 (silicio,
     * 2026-08-01) la cabeza sale casi toda a cero y lo que importa está más
     * abajo. Se suben a 32 palabras (128 B) y se imprime en cuatro líneas de
     * ocho, con el offset delante para poder contarlas. El journal entero son
     * ~6,5 KiB: eso sí ahogaría el serie, y además su cola son registros
     * repetidos por motor. Las líneas a cero se saltan — en un volcado de 128 B
     * casi todo suele serlo, y lo que se busca es dónde deja de serlo. */
    if (msg->rcJournalBufferSize && len > sizeof(*msg)) {
        gsp_rpc_rc_journal_log(msg->rcJournalBuffer,
                               len - (uint32_t)sizeof(*msg),
                               msg->rcJournalBufferSize);
    } else if (msg->rcJournalBufferSize) {
        lx_printk("nouveau-lx: rc: journal %u B pero sólo llegaron %u B de "
                  "mensaje: la copia del anillo se quedó corta\n",
                  msg->rcJournalBufferSize, len);
    }
    return 0;
}

static void rc_capture(const struct gsp_rpc *rpc, uint32_t rptr, uint32_t length)
{
    /* Cabecera fija + 128 B del journal, que es donde va el subtipo del error.
     * 2 KiB: el `RcDiagReport` trae ~200 entradas de 16 B y con 512 B sólo
     * entraban doce. Las que faltan son las que pueden nombrar la unidad exacta
     * que levantó la excepción — el volcado de GR ya viene en el mensaje, sólo
     * había que quedárselo entero. */
    unsigned char buf[sizeof(rpc_rc_triggered_v17_02) + 2048u];
    uint32_t plen = length - GSP_RPC_HDR_SIZE;

    if (plen > (uint32_t)sizeof(buf)) {
        plen = (uint32_t)sizeof(buf);
    }
    ring_copy(rpc, rptr, GSP_MSG_HDR_SIZE + GSP_RPC_HDR_SIZE, buf, plen);
    gsp_rpc_rc_triggered_log(buf, plen);
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
                      hdr.function, rpc_function_name(hdr.function), hdr.length,
                      hdr.rpc_result);
        }

        /* El contenido de los NOCAT distintos: "2 registro(s) NOCAT antes del
         * fallo" no dice qué falló, y la causa del NO_MEMORY del canal de GR0 o
         * del CE que no arranca está aquí dentro. */
        if (hdr.function == NV_VGPU_MSG_EVENT_GSP_POST_NOCAT_RECORD) {
            nocat_capture(rpc, rptr, hdr.length);
        }
        if (hdr.function == NV_VGPU_MSG_EVENT_RC_TRIGGERED) {
            rc_capture(rpc, rptr, hdr.length);
        }
        if (hdr.function == NV_VGPU_MSG_EVENT_GSP_RUN_CPU_SEQUENCER) {
            cpu_seq_capture(rpc, rptr, hdr.length);
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
                      rpc_function_name(hdr.function));
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
                      rpc_function_name(fn), seen);
            return 0;
        }
    }
    flush_repeats(&repeats);
    lx_printk("nouveau-lx: no llegó fn=0x%04x (%u mensaje(s) vistos, %u NOCAT)\n",
              fn, seen, nocat);
    return -1;
}

unsigned gsp_rpc_drain(struct gsp_rpc *rpc, unsigned ms)
{
    unsigned seen = 0;
    unsigned nocat = 0;
    uint32_t last_fn = 0xffffffffu;
    unsigned repeats = 0;
    uint32_t ring_bytes;

    /* `ms == 0` aparte: el `while (ms--)` de abajo lo convertiría en 2^32 vueltas
     * por el envoltorio del unsigned. */
    if (!rpc || !rpc->ready || ms == 0u) {
        return 0;
    }
    ring_bytes = rpc->cnt * GSP_PAGE_SIZE;

    while (ms--) {
        uint32_t rptr = *rpc->rptr;
        struct gsp_rpc_hdr hdr;
        uint32_t pages;

        if (msgq_used(rpc, rptr) == 0) {
            if (!gsp_mmio_alive()) {
                break;
            }
            lx_mdelay(1);
            continue;
        }
        ring_copy(rpc, rptr, GSP_MSG_HDR_SIZE, &hdr, GSP_RPC_HDR_SIZE);
        if (hdr.length < GSP_RPC_HDR_SIZE ||
            hdr.length > ring_bytes - GSP_MSG_HDR_SIZE) {
            lx_printk("nouveau-lx: RPC con longitud imposible (%u) al vaciar\n",
                      hdr.length);
            break;
        }
        seen++;
        if (hdr.function == last_fn) {
            repeats++;
        } else {
            flush_repeats(&repeats);
            last_fn = hdr.function;
            lx_printk("nouveau-lx: RPC (sin pedir) fn=0x%04x (%s) len=%u res=0x%x\n",
                      hdr.function, rpc_function_name(hdr.function), hdr.length,
                      hdr.rpc_result);
        }
        if (hdr.function == NV_VGPU_MSG_EVENT_GSP_POST_NOCAT_RECORD) {
            nocat++;
            nocat_capture(rpc, rptr, hdr.length);
        }
        if (hdr.function == NV_VGPU_MSG_EVENT_RC_TRIGGERED) {
            rc_capture(rpc, rptr, hdr.length);
        }
        if (hdr.function == NV_VGPU_MSG_EVENT_GSP_RUN_CPU_SEQUENCER) {
            cpu_seq_capture(rpc, rptr, hdr.length);
        }

        pages = (hdr.length + GSP_MSG_HDR_SIZE + GSP_PAGE_SIZE - 1) / GSP_PAGE_SIZE;
        rptr = (rptr + pages) % rpc->cnt;
        gsp_rpc_barrier();
        *rpc->rptr = rptr;
    }
    flush_repeats(&repeats);
    if (seen) {
        lx_printk("nouveau-lx: %u mensaje(s) pendientes de GSP-RM (%u NOCAT)\n",
                  seen, nocat);
    } else {
        /* Que no haya nada NO es un detalle menor: significa que RM no se ha
         * quejado, así que el trabajo no llegó a fallar del lado de RM — no
         * arrancó. Eso apunta al doorbell o al canal, no a los métodos. */
        lx_printk("nouveau-lx: GSP-RM no tenía nada que decir (ni un evento)\n");
    }
    return seen;
}

int gsp_rpc_wait_event(struct gsp_rpc *rpc, uint32_t fn, unsigned timeout_ms)
{
    return gsp_rpc_recv(rpc, fn, NULL, 0, NULL, NULL, timeout_ms);
}

/* `r535_gsp_postinit` mínimo: habilitar interrupciones del falcon GSP. Sin MSI
 * en soso usamos polling, pero RM espera este bit tras el init. */
void gsp_rpc_postinit(void)
{
    gsp_mmio_wr32(NV_PGSP_FALCON + 0x4u, 0x40u);
    lx_printk("nouveau-lx: GSP postinit (0x110004=0x40)\n");
}
