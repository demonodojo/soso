/* G4 paso 2: envío de RPCs a GSP-RM por la cola de comandos.
 *
 * El orden importa y no es el obvio: `r535_gsp_oneinit` encola
 * `GSP_SET_SYSTEM_INFO` y `SET_REGISTRY` **antes de arrancar el GSP**. Quedan
 * esperando en la cmdq y GSP-RM las consume como parte de su propia
 * inicialización. Sin ellas arranca a ciegas: en la primera prueba en hardware
 * (2026-07-25) soltó cientos de `GSP_POST_NOCAT_RECORD` y devolvió
 * `GSP_INIT_DONE` con `NV_ERR_OPERATING_SYSTEM`.
 *
 * Referencias: `r535_gsp_cmdq_{get,push}`, `r535_gsp_rpc_{get,send}`
 * (`rm/r535/rpc.c`), `r570_gsp_set_system_info` (`rm/r570/gsp.c`) y
 * `r535_gsp_rpc_set_registry` (`rm/r535/gsp.c`).
 */
#include "gsp_cmdq.h"
#include "gsp_mmio.h"

/* Definidos en el shim (lxdde/shim/src/shims.c). */
void *memcpy(void *dst, const void *src, unsigned long n);
void *memset(void *dst, int c, unsigned long n);

#define NV_PGSP_FALCON      0x00110000u
/* Escribir aquí es el timbre de la cmdq (`nvkm_falcon_wr32(falcon, 0xc00, 0)`). */
#define NV_PGSP_QUEUE_KICK  (NV_PGSP_FALCON + 0xc00u)

#define GSP_MSG_HDR_SIZE  ((uint32_t)sizeof(struct gsp_msg_elem))
#define GSP_RPC_HDR_SIZE  ((uint32_t)sizeof(struct gsp_rpc_hdr))
#define GSP_MSG_MIN_SIZE  GSP_PAGE_SIZE

/* Cabecera de RPC saliente (`r535_gsp_rpc_get`). */
#define GSP_RPC_HEADER_VERSION 0x03000000u
#define GSP_RPC_SIGNATURE      (((uint32_t)'C' << 24) | ((uint32_t)'P' << 16) | \
                                ((uint32_t)'R' << 8) | (uint32_t)'V')

/* Funciones de `rm/r570/nvrm/rpcfn.h`. */
#define NV_VGPU_MSG_FUNCTION_GSP_SET_SYSTEM_INFO 72u
#define NV_VGPU_MSG_FUNCTION_SET_REGISTRY        73u

static uint32_t align_up_u32(uint32_t v, uint32_t a)
{
    return (v + a - 1u) & ~(a - 1u);
}

int gsp_cmdq_init(const struct gsp_libos *lo, struct gsp_cmdq *out)
{
    struct gsp_msgq_headers *cmdq;
    struct gsp_msgq_headers *msgq;

    if (!lo || !out || !lo->ready || !lo->shm.va) {
        return -1;
    }
    cmdq = (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->cmdq_offset);
    msgq = (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);

    /* Espejo de los punteros de la recepción: la cmdq escribe su `tx.writePtr`
     * y lee el `rx.readPtr` de la cola de mensajes. */
    out->wptr = &cmdq->tx.writePtr;
    out->rptr = &msgq->rx.readPtr;
    out->cmdq = (unsigned char *)cmdq;
    out->cnt = cmdq->tx.msgCount;
    out->seq = 0;
    if (!out->cnt) {
        return -1;
    }
    out->ready = 1;
    return 0;
}

/* Empuja un elemento ya montado en `buf` (cabecera de elemento + RPC + payload).
 * `rpc_len` es lo que ocupan la cabecera RPC y su payload. */
static int cmdq_push(struct gsp_cmdq *q, struct gsp_msg_elem *msg, uint32_t rpc_len)
{
    uint32_t len = align_up_u32(GSP_MSG_HDR_SIZE + rpc_len, GSP_PAGE_SIZE);
    uint32_t pages = len / GSP_PAGE_SIZE;
    uint32_t wptr = *q->wptr;
    uint64_t csum = 0;
    const uint64_t *p;
    const uint64_t *end;
    unsigned time;

    if (pages > q->cnt) {
        lx_printk("nouveau-lx: RPC de %u páginas no cabe en el anillo (%u)\n",
                  pages, q->cnt);
        return -1;
    }

    msg->pad = 0;
    msg->checksum = 0;
    msg->sequence = q->seq++;
    msg->elem_count = pages;

    /* XOR de todo el elemento en palabras de 64 bits, plegado a 32. */
    p = (const uint64_t *)msg;
    end = (const uint64_t *)((const unsigned char *)msg + len);
    while (p < end) {
        csum ^= *p++;
    }
    msg->checksum = (uint32_t)(csum >> 32) ^ (uint32_t)csum;

    /* Esperar hueco: el GSP libera páginas moviendo su puntero de lectura. */
    for (time = 4000; time > 0; time--) {
        uint32_t free = *q->rptr + q->cnt - wptr - 1u;
        if (free >= q->cnt) {
            free -= q->cnt;
        }
        if (free >= pages) {
            break;
        }
        if (!gsp_mmio_alive()) {
            lx_printk("nouveau-lx: la GPU se ha caído del bus encolando un RPC\n");
            return -1;
        }
        lx_mdelay(1);
    }
    if (time == 0) {
        lx_printk("nouveau-lx: cmdq llena, no cabe el RPC\n");
        return -1;
    }

    /* Copiar, partiendo si el anillo da la vuelta. */
    {
        uint32_t off = 0;
        uint32_t left = len;
        while (left) {
            uint32_t room = (q->cnt - wptr) * GSP_PAGE_SIZE;
            uint32_t size = left < room ? left : room;
            unsigned char *cqe = q->cmdq + GSP_PAGE_SIZE +
                                 (unsigned long)wptr * GSP_PAGE_SIZE;

            memcpy(cqe, (const unsigned char *)msg + off, size);
            wptr += size / GSP_PAGE_SIZE;
            if (wptr == q->cnt) {
                wptr = 0;
            }
            off += size;
            left -= size;
        }
    }

    /* El elemento tiene que estar completo antes de publicar el puntero, y el
     * puntero antes de tocar el timbre. */
    __asm__ __volatile__("sfence" ::: "memory");
    *q->wptr = wptr;
    __asm__ __volatile__("mfence" ::: "memory");

    gsp_mmio_wr32(NV_PGSP_QUEUE_KICK, 0);
    return 0;
}

/* Monta el elemento en `buf` y devuelve dónde va el payload. */
static void *rpc_prepare(unsigned char *buf, unsigned long buf_len, uint32_t fn,
                         uint32_t payload_size, uint32_t *rpc_len)
{
    struct gsp_rpc_hdr *rpc;

    *rpc_len = align_up_u32(GSP_RPC_HDR_SIZE + payload_size, (uint32_t)sizeof(uint64_t));
    if (GSP_MSG_HDR_SIZE + *rpc_len > buf_len) {
        return NULL;
    }
    memset(buf, 0, buf_len);

    rpc = (struct gsp_rpc_hdr *)(buf + GSP_MSG_HDR_SIZE);
    rpc->header_version = GSP_RPC_HEADER_VERSION;
    rpc->signature = GSP_RPC_SIGNATURE;
    rpc->function = fn;
    rpc->rpc_result = 0xffffffffu;
    rpc->rpc_result_private = 0xffffffffu;
    rpc->length = GSP_RPC_HDR_SIZE + payload_size;
    /* La `sequence` de la cabecera RPC va a cero también upstream: la que se
     * incrementa por mensaje es la del *elemento* de cola (`cmdq_push`), y una
     * respuesta se empareja por `function`, no por secuencia. */
    rpc->sequence = 0;
    return rpc + 1;
}

int gsp_cmdq_send(struct gsp_cmdq *q, uint32_t fn, const void *payload,
                  uint32_t payload_size)
{
    unsigned long buf_len;
    unsigned char *buf;
    void *dst;
    uint32_t rpc_len = 0;
    int ret;

    if (!q || !q->ready) {
        return -1;
    }
    buf_len = align_up_u32(GSP_MSG_HDR_SIZE + GSP_RPC_HDR_SIZE + payload_size,
                           GSP_MSG_MIN_SIZE);
    buf = lx_kzalloc(buf_len, GFP_KERNEL);
    if (!buf) {
        return -1;
    }
    dst = rpc_prepare(buf, buf_len, fn, payload_size, &rpc_len);
    if (!dst) {
        lx_kfree(buf);
        return -1;
    }
    if (payload && payload_size) {
        memcpy(dst, payload, payload_size);
    }
    ret = cmdq_push(q, (struct gsp_msg_elem *)buf, rpc_len);
    lx_kfree(buf);
    return ret;
}

int gsp_cmdq_call(struct gsp_cmdq *q, struct gsp_rpc *rpc, uint32_t fn,
                  const void *payload, uint32_t payload_size,
                  void *reply, uint32_t reply_len, uint32_t *reply_got,
                  uint32_t *status, unsigned timeout_ms)
{
    if (!q || !q->ready || !rpc || !rpc->ready) {
        return -1;
    }
    if (gsp_cmdq_send(q, fn, payload, payload_size) != 0) {
        return -1;
    }
    return gsp_rpc_recv(rpc, fn, reply, reply_len, reply_got, status, timeout_ms);
}

int gsp_cmdq_set_system_info(struct gsp_cmdq *q, const struct gsp_sysinfo *si)
{
    GspSystemInfo info;

    if (!si) {
        return -1;
    }
    memset(&info, 0, sizeof(info));

    info.gpuPhysAddr = si->bar0_phys;
    info.gpuPhysFbAddr = si->bar1_phys;
    info.gpuPhysInstAddr = si->bar3_phys;
    info.nvDomainBusDeviceFunc = si->bdf;
    info.maxUserVa = si->max_user_va;
    info.pciConfigMirrorBase = si->cfg_mirror_base;
    info.pciConfigMirrorSize = si->cfg_mirror_size;
    info.PCIDeviceID = ((uint32_t)si->device_id << 16) | si->vendor_id;
    info.PCISubDeviceID = ((uint32_t)si->subdevice_id << 16) | si->subvendor_id;
    info.PCIRevisionID = si->revision_id;
    /* `acpiMethodData` se queda a cero: sin ACPI, sus `bValid` valen 0 y GSP-RM
     * lo entiende como "este sistema no aporta esos métodos". Igual con
     * `bIsPrimary` — la dGPU no pinta la consola — y con
     * `bPreserveVideoMemoryAllocations`. */

    lx_printk("nouveau-lx: SET_SYSTEM_INFO bar0=0x%llx bar1=0x%llx bar3=0x%llx "
              "bdf=0x%llx dev=0x%08x (%u B)\n",
              (unsigned long long)info.gpuPhysAddr,
              (unsigned long long)info.gpuPhysFbAddr,
              (unsigned long long)info.gpuPhysInstAddr,
              (unsigned long long)info.nvDomainBusDeviceFunc,
              info.PCIDeviceID, (unsigned)sizeof(info));

    return gsp_cmdq_send(q, NV_VGPU_MSG_FUNCTION_GSP_SET_SYSTEM_INFO,
                         &info, (uint32_t)sizeof(info));
}

/* Las tres claves que pone nouveau (`r535_registry_entries`). */
static const struct {
    const char *key;
    uint32_t value;
} g_registry[] = {
    { "RMSecBusResetEnable", 1 },
    { "RMForcePcieConfigSave", 1 },
    { "RMDevidCheckIgnore", 1 },
};

#define REGISTRY_COUNT (sizeof(g_registry) / sizeof(g_registry[0]))

static unsigned long registry_key_len(const char *s)
{
    unsigned long n = 0;
    while (s[n]) {
        n++;
    }
    return n + 1;   /* con el NUL */
}

int gsp_cmdq_set_registry(struct gsp_cmdq *q)
{
    /* Tabla + entradas + los nombres detrás, todo contiguo. */
    unsigned char buf[512];
    PACKED_REGISTRY_TABLE *table = (PACKED_REGISTRY_TABLE *)buf;
    PACKED_REGISTRY_ENTRY *entries = (PACKED_REGISTRY_ENTRY *)(table + 1);
    unsigned long str_off = sizeof(*table) + REGISTRY_COUNT * sizeof(*entries);
    unsigned i;

    memset(buf, 0, sizeof(buf));
    table->numEntries = (uint32_t)REGISTRY_COUNT;

    for (i = 0; i < REGISTRY_COUNT; i++) {
        unsigned long klen = registry_key_len(g_registry[i].key);

        if (str_off + klen > sizeof(buf)) {
            lx_printk("nouveau-lx: registry no cabe en el buffer\n");
            return -1;
        }
        entries[i].type = REGISTRY_TABLE_ENTRY_TYPE_DWORD;
        entries[i].length = (uint32_t)sizeof(uint32_t);
        entries[i].data = g_registry[i].value;
        entries[i].nameOffset = (uint32_t)str_off;
        memcpy(buf + str_off, g_registry[i].key, klen);
        str_off += klen;
    }
    table->size = (uint32_t)str_off;

    lx_printk("nouveau-lx: SET_REGISTRY %u claves (%lu B)\n",
              (unsigned)REGISTRY_COUNT, str_off);

    return gsp_cmdq_send(q, NV_VGPU_MSG_FUNCTION_SET_REGISTRY, buf,
                         (uint32_t)str_off);
}
