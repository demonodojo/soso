/* G3b paso 6: COT al FSP. Ver fsp_lx.h — aquí empiezan las escrituras MMIO. */
#include "fsp_lx.h"
#include "gsp_mmio.h"

/* Definidos en el shim (lxdde/shim/src/shims.c). */
void *memcpy(void *dst, const void *src, unsigned long n);
void *memset(void *dst, int c, unsigned long n);

/* --- Registros -------------------------------------------------------------
 * Todos dentro de los 16 MiB de BAR0 que mapea el bring-up. Las colas del FSP
 * ya estaban verificadas en HW por `fmc_lx_fsp_probe` (secure boot = 0xff). */
#define NV_PFSP_QUEUE_HEAD0   0x008f2c00u
#define NV_PFSP_QUEUE_TAIL0   0x008f2c04u
#define NV_PFSP_MSGQ_HEAD0    0x008f2c80u
#define NV_PFSP_MSGQ_TAIL0    0x008f2c84u

/* Falcon del FSP: base 0x8f2000 (`nvkm_falcon_ctor(..., "fsp", 0x8f2000, ...)`).
 * EMEM se escribe por el par puerto/dato con autoincremento, igual que
 * `gp102_flcn_emem_pio`: bit 24 = autoinc de escritura, bit 25 = de lectura. */
#define NV_PFSP_FALCON        0x008f2000u
#define NV_PFALCON_EMEMC0     (NV_PFSP_FALCON + 0xac0u)
#define NV_PFALCON_EMEMD0     (NV_PFSP_FALCON + 0xac4u)
#define EMEMC_AINCW           (1u << 24)
#define EMEMC_AINCR           (1u << 25)

/* Falcon del GSP: base 0x110000 (`nvkm_falcon_ctor(..., 0x110000, ...)`). */
#define NV_PGSP_FALCON        0x00110000u
#define NV_PFALCON_MAILBOX0   (NV_PGSP_FALCON + 0x040u)
#define NV_PFALCON_MAILBOX1   (NV_PGSP_FALCON + 0x044u)
#define NV_PFALCON_HWCFG2     (NV_PGSP_FALCON + 0x0f4u)
#define HWCFG2_RISCV_BR_PRIV_LOCKDOWN (1u << 13)
/* Bloque RISC-V del falcon: `falcon->addr2` = 0x1000 en `ga102_gsp_flcn`. */
#define NV_PRISCV_RISCV_CPUCTL (NV_PGSP_FALCON + 0x1000u + 0x388u)
#define CPUCTL_HALTED         (1u << 4)

#define NV_THERM_I2CS_SCRATCH_GB202 0x00ad00bcu
#define FSP_BOOT_COMPLETE_SUCCESS   0x000000ffu

/* --- Protocolo MCTP/NVDM ---------------------------------------------------- */
#define MCTP_HEADER_SOM               (1u << 31)
#define MCTP_HEADER_EOM               (1u << 30)

#define MCTP_MSG_HEADER_TYPE_VENDOR_PCI 0x7eu   /* bits 6:0 */
#define MCTP_MSG_HEADER_VENDOR_ID_NV    0x10deu /* bits 23:8 */
#define NVDM_TYPE_COT                   0x14u   /* bits 31:24 */
#define NVDM_TYPE_FSP_RESPONSE          0x15u

#define NVDM_HEADER(type) \
    (((uint32_t)(type) << 24) | ((uint32_t)MCTP_MSG_HEADER_VENDOR_ID_NV << 8) | \
     (uint32_t)MCTP_MSG_HEADER_TYPE_VENDOR_PCI)

#define NVDM_HEADER_TYPE(h)      (((h) >> 24) & 0xffu)
#define NVDM_HEADER_VENDOR(h)    (((h) >> 8) & 0xffffu)
#define NVDM_HEADER_MSG_TYPE(h)  ((h) & 0x7fu)

/* Los tamaños de la cadena de firma son los de gb202 (gh100 usa 384/384), pero
 * los campos del mensaje miden lo mismo en las dos: se copia lo que hay y el
 * resto queda a cero. */
#define COT_VERSION_GB202 2u
#define COT_HASH_SIZE     48u
#define COT_PKEY_SIZE     97u
#define COT_SIG_SIZE      96u
#define COT_FRTS_VIDMEM_SIZE 0x100000u

struct nvdm_payload_cot {
    uint16_t version;
    uint16_t size;
    uint64_t gspFmcSysmemOffset;
    uint64_t frtsSysmemOffset;
    uint32_t frtsSysmemSize;
    /* Ojo: este es un offset desde el FINAL de la VRAM. */
    uint64_t frtsVidmemOffset;
    uint32_t frtsVidmemSize;
    uint32_t hash384[12];
    uint32_t publicKey[96];
    uint32_t signature[96];
    uint64_t gspBootArgsSysmemOffset;
} __attribute__((packed));

struct fsp_cot_msg {
    uint32_t mctp_header;
    uint32_t nvdm_header;
    struct nvdm_payload_cot cot;
};

struct nvdm_command_response {
    uint32_t taskId;
    uint32_t commandNvdmType;
    uint32_t errorCode;
} __attribute__((packed));

struct fsp_reply {
    uint32_t mctp_header;
    uint32_t nvdm_header;
    struct nvdm_command_response response;
};

/* El FSP lee estos bytes tal cual; un desajuste de layout manda un COT inválido. */
typedef char cot_size_check[sizeof(struct nvdm_payload_cot) == 860 ? 1 : -1];
typedef char msg_size_check[sizeof(struct fsp_cot_msg) == 868 ? 1 : -1];
typedef char reply_size_check[sizeof(struct fsp_reply) == 20 ? 1 : -1];

/* --- EMEM ------------------------------------------------------------------ */

static void emem_write(const uint32_t *data, unsigned dwords)
{
    unsigned i;

    gsp_mmio_wr32(NV_PFALCON_EMEMC0, EMEMC_AINCW);   /* offset 0, autoincremento */
    for (i = 0; i < dwords; i++) {
        gsp_mmio_wr32(NV_PFALCON_EMEMD0, data[i]);
    }
}

static void emem_read(uint32_t *data, unsigned dwords)
{
    unsigned i;

    gsp_mmio_wr32(NV_PFALCON_EMEMC0, EMEMC_AINCR);
    for (i = 0; i < dwords; i++) {
        data[i] = gsp_mmio_rd32(NV_PFALCON_EMEMD0);
    }
}

static void log_gsp_state(const char *what);

/* Estado de la cola de mensajes del FSP. TAIL apunta al último DWORD escrito.
 *
 * Devuelve 1 con datos (`*bytes`), 0 vacía y -1 si la GPU no contesta. Esa
 * tercera respuesta es el motivo de existir de esta función: con la tarjeta
 * fuera del bus los dos registros se leen 0xffffffff, `head == tail` se cumple
 * al pie de la letra, y devolver 0 —"cola vacía"— es como el cuelgue del
 * 2026-07-27 acabó anunciándose por serie como "respuesta del FSP de tamaño
 * raro (0)" en vez de decir que la tarjeta se había caído del bus. Es la misma
 * trampa que gsp_mmio_poll_ready() ya documenta: un all-ones no es un registro,
 * es silencio. */
static int fsp_poll(uint32_t *bytes)
{
    uint32_t head = gsp_mmio_rd32(NV_PFSP_MSGQ_HEAD0);
    uint32_t tail = gsp_mmio_rd32(NV_PFSP_MSGQ_TAIL0);

    if (bytes) {
        *bytes = 0;
    }
    if (head == 0xffffffffu && tail == 0xffffffffu) {
        return -1;
    }
    if (head == tail) {
        return 0;
    }
    if (bytes) {
        *bytes = (tail - head) + (uint32_t)sizeof(uint32_t);
    }
    return 1;
}

/* Un -1 de fsp_poll() puede ser un reset de función, que deja la tarjeta en el
 * bus pero sin decode de memoria y de eso se vuelve. Mismo criterio que el bucle
 * de arranque del FMC. Devuelve 1 si el MMIO ha vuelto. */
static int fsp_gone_recovered(const char *cuando)
{
    if (gsp_mmio_pci_recover() == 0 && gsp_mmio_alive()) {
        lx_printk("nouveau-lx: el MMIO ha vuelto tras reactivar el decode (%s)\n", cuando);
        return 1;
    }
    lx_printk("nouveau-lx: la GPU se ha caído del bus %s\n", cuando);
    log_gsp_state("fuera del bus");
    return 0;
}

static int fsp_send(const void *packet, uint32_t packet_size)
{
    unsigned time = 1000;

    if (packet_size % sizeof(uint32_t)) {
        return -1;
    }
    /* Que no quede un mensaje anterior sin consumir. */
    for (;;) {
        uint32_t head = gsp_mmio_rd32(NV_PFSP_QUEUE_HEAD0);
        uint32_t tail = gsp_mmio_rd32(NV_PFSP_QUEUE_TAIL0);
        if (head == tail) {
            break;
        }
        if (time-- == 0) {
            lx_printk("nouveau-lx: FSP con la cola ocupada (head=0x%x tail=0x%x)\n",
                      head, tail);
            return -1;
        }
        lx_mdelay(1);
    }

    emem_write(packet, packet_size / (uint32_t)sizeof(uint32_t));

    /* TAIL apunta al último DWORD escrito; escribir HEAD dispara el mensaje. */
    gsp_mmio_wr32(NV_PFSP_QUEUE_TAIL0, packet_size - (uint32_t)sizeof(uint32_t));
    gsp_mmio_wr32(NV_PFSP_QUEUE_HEAD0, 0);
    return 0;
}

static int fsp_wait_reply(unsigned timeout_ms)
{
    while (timeout_ms--) {
        int st = fsp_poll(NULL);

        if (st > 0) {
            return 0;
        }
        if (st < 0 && !fsp_gone_recovered("esperando la respuesta al COT")) {
            return -1;
        }
        lx_mdelay(1);
    }
    lx_printk("nouveau-lx: FSP no contesta al COT\n");
    return -1;
}

static int fsp_recv(struct fsp_reply *reply)
{
    uint32_t packet_size = 0;
    int st = fsp_poll(&packet_size);

    if (st < 0) {
        /* Aquí llegó el cuelgue del 2026-07-27: wait_reply había visto cola (no
         * imprimió "FSP no contesta"), y una vuelta después head==tail==all-ones.
         * Es decir, la tarjeta murió entre las dos lecturas, no que el FSP
         * contestara raro. */
        (void)fsp_gone_recovered("antes de leer la respuesta al COT");
        return -1;
    }
    if (st == 0) {
        lx_printk("nouveau-lx: el FSP anunció respuesta y dejó la cola vacía\n");
        return -1;
    }
    if ((packet_size % 4u) || packet_size > sizeof(*reply)) {
        lx_printk("nouveau-lx: respuesta del FSP de tamaño raro (%u)\n", packet_size);
        return -1;
    }
    emem_read((uint32_t *)reply, packet_size / 4u);

    gsp_mmio_wr32(NV_PFSP_MSGQ_TAIL0, 0);
    gsp_mmio_wr32(NV_PFSP_MSGQ_HEAD0, 0);
    return (int)packet_size;
}

static int fsp_check_reply(const struct fsp_reply *r)
{
    if (!(r->mctp_header & MCTP_HEADER_SOM) || !(r->mctp_header & MCTP_HEADER_EOM)) {
        lx_printk("nouveau-lx: cabecera MCTP inesperada: 0x%08x\n", r->mctp_header);
        return -1;
    }
    if (NVDM_HEADER_MSG_TYPE(r->nvdm_header) != MCTP_MSG_HEADER_TYPE_VENDOR_PCI ||
        NVDM_HEADER_VENDOR(r->nvdm_header) != MCTP_MSG_HEADER_VENDOR_ID_NV ||
        NVDM_HEADER_TYPE(r->nvdm_header) != NVDM_TYPE_FSP_RESPONSE) {
        lx_printk("nouveau-lx: cabecera NVDM inesperada: 0x%08x\n", r->nvdm_header);
        return -1;
    }
    if (r->response.commandNvdmType != NVDM_TYPE_COT) {
        lx_printk("nouveau-lx: el FSP responde al NVDM 0x%02x, no al COT\n",
                  r->response.commandNvdmType);
        return -1;
    }
    if (r->response.errorCode) {
        lx_printk("nouveau-lx: el FSP rechaza el COT (error 0x%08x)\n",
                  r->response.errorCode);
        return -1;
    }
    return 0;
}

/* `gh100_gsp_lockdown_released`. Devuelve 1 cuando hay veredicto (bueno o malo),
 * 0 mientras el bootrom siga trabajando. */
static int lockdown_released(uint64_t args_addr, uint32_t *mbox0)
{
    uint32_t hwcfg2;

    *mbox0 = gsp_mmio_rd32(NV_PFALCON_MAILBOX0);

    /* 0xbadf41xx = el bootrom aún no deja leer el falcon por BAR0. */
    if (*mbox0 && (*mbox0 & 0xffffff00u) == 0xbadf4100u) {
        return 0;
    }
    if (*mbox0) {
        uint32_t mbox1 = gsp_mmio_rd32(NV_PFALCON_MAILBOX1);
        uint64_t val = ((uint64_t)mbox1 << 32) | *mbox0;
        /* Lo único válido ahí es la dirección de los boot params; otra cosa
         * es un código de error del FMC. */
        if (val != args_addr) {
            return 1;
        }
    }
    hwcfg2 = gsp_mmio_rd32(NV_PFALCON_HWCFG2);
    return (hwcfg2 & HWCFG2_RISCV_BR_PRIV_LOCKDOWN) ? 0 : 1;
}

static void log_gsp_state(const char *what)
{
    uint32_t mbox0 = gsp_mmio_rd32(NV_PFALCON_MAILBOX0);
    uint32_t mbox1 = gsp_mmio_rd32(NV_PFALCON_MAILBOX1);
    uint32_t hwcfg2 = gsp_mmio_rd32(NV_PFALCON_HWCFG2);
    uint32_t cpuctl = gsp_mmio_rd32(NV_PRISCV_RISCV_CPUCTL);

    lx_printk("nouveau-lx: GSP %s mbox0=0x%08x mbox1=0x%08x hwcfg2=0x%08x (lockdown=%u) "
              "cpuctl=0x%08x (halted=%u)\n",
              what, mbox0, mbox1, hwcfg2,
              (hwcfg2 & HWCFG2_RISCV_BR_PRIV_LOCKDOWN) ? 1u : 0u,
              cpuctl, (cpuctl & CPUCTL_HALTED) ? 1u : 0u);
}

/* --- Precondiciones --------------------------------------------------------- */

static int fsp_ready_to_send(void)
{
    uint32_t boot = gsp_mmio_rd32(NV_THERM_I2CS_SCRATCH_GB202);
    uint32_t qh = gsp_mmio_rd32(NV_PFSP_QUEUE_HEAD0);
    uint32_t qt = gsp_mmio_rd32(NV_PFSP_QUEUE_TAIL0);
    uint32_t mh = gsp_mmio_rd32(NV_PFSP_MSGQ_HEAD0);
    uint32_t mt = gsp_mmio_rd32(NV_PFSP_MSGQ_TAIL0);

    /* Antes que nada: sin esto, una tarjeta muerta se anunciaba como "FSP sin
     * secure boot (0xffffffff)", y las dos comprobaciones de cola de abajo la
     * daban por libre (all-ones == all-ones). */
    if (!gsp_mmio_alive()) {
        lx_printk("nouveau-lx: la GPU no contesta antes de mandar el COT\n");
        return -1;
    }
    if (boot != FSP_BOOT_COMPLETE_SUCCESS) {
        lx_printk("nouveau-lx: FSP sin secure boot (0x%08x) — no se manda el COT\n", boot);
        return -1;
    }
    if (qh != qt || mh != mt) {
        lx_printk("nouveau-lx: colas del FSP ocupadas (%08x/%08x %08x/%08x)\n",
                  qh, qt, mh, mt);
        return -1;
    }
    return 0;
}

int fsp_lx_boot_gsp_fmc(const struct fmc_staged *fmc, const struct gsp_libos *libos,
                        const struct gsp_wpr *wpr)
{
    struct fsp_cot_msg *msg;
    struct fsp_reply reply;
    uint64_t args_addr;
    unsigned time;
    uint32_t mbox0 = 0;
    int ret;

    if (!fmc || !libos || !wpr || !libos->ready || !wpr->ready || !fmc->img.va) {
        lx_printk("nouveau-lx: COT sin payload completo — no se envía\n");
        return -1;
    }
    if (fmc->hash.size != COT_HASH_SIZE || fmc->pkey.size != COT_PKEY_SIZE ||
        fmc->sig.size != COT_SIG_SIZE) {
        lx_printk("nouveau-lx: cadena de firma %lu/%lu/%lu — gb20x espera 48/97/96\n",
                  fmc->hash.size, fmc->pkey.size, fmc->sig.size);
        return -1;
    }
    if (fsp_ready_to_send() != 0) {
        return -1;
    }

    msg = lx_kzalloc(sizeof(*msg), GFP_KERNEL);
    if (!msg) {
        return -1;
    }
    args_addr = libos->boot_params.phys;

    msg->mctp_header = MCTP_HEADER_SOM | MCTP_HEADER_EOM;   /* SEID y SEQ a 0 */
    msg->nvdm_header = NVDM_HEADER(NVDM_TYPE_COT);

    msg->cot.version = COT_VERSION_GB202;
    msg->cot.size = (uint16_t)sizeof(msg->cot);
    msg->cot.gspFmcSysmemOffset = fmc->img.phys;
    /* Arranque en frío: la FRTS va al final de la VRAM, detrás de la reserva. */
    msg->cot.frtsVidmemOffset = wpr->rsvd_size;
    msg->cot.frtsVidmemSize = COT_FRTS_VIDMEM_SIZE;

    /* Por offset y no por `&msg->cot.hash384`: son campos de una struct packed y
     * tomarles la dirección es justo lo que avisa -Waddress-of-packed-member. */
    {
        unsigned char *cot = (unsigned char *)&msg->cot;
        memcpy(cot + offsetof(struct nvdm_payload_cot, hash384), fmc->hash.va, COT_HASH_SIZE);
        memcpy(cot + offsetof(struct nvdm_payload_cot, publicKey), fmc->pkey.va, COT_PKEY_SIZE);
        memcpy(cot + offsetof(struct nvdm_payload_cot, signature), fmc->sig.va, COT_SIG_SIZE);
    }

    msg->cot.gspBootArgsSysmemOffset = args_addr;

    lx_printk("nouveau-lx: COT v%u %u B → fmc=0x%llx args=0x%llx frts=+0x%x\n",
              msg->cot.version, (unsigned)sizeof(*msg),
              (unsigned long long)fmc->img.phys, (unsigned long long)args_addr,
              wpr->rsvd_size);

    ret = fsp_send(msg, (uint32_t)sizeof(*msg));
    lx_kfree(msg);
    if (ret != 0) {
        return -1;
    }
    if (fsp_wait_reply(1000) != 0) {
        return -1;
    }
    memset(&reply, 0, sizeof(reply));
    if (fsp_recv(&reply) < 0 || fsp_check_reply(&reply) != 0) {
        return -1;
    }
    lx_printk("nouveau-lx: COT aceptado por el FSP\n");
    log_gsp_state("tras el COT");

    /* El FMC arranca y baja el lockdown del bootrom RISC-V del GSP. Upstream
     * espera 4000 vueltas de `usleep_range(1000,2000)` = 4–8 s; aquí 8 s, que
     * este camino es lento y de una sola vez. */
    for (time = 8000; time > 0; time--) {
        /* Si la tarjeta se cae del bus a mitad del arranque del FMC, todo el
         * MMIO pasa a leerse 0xffffffff. Decirlo con estas palabras evita
         * confundirlo con un código de error del FMC (2026-07-25). */
        if (!gsp_mmio_alive()) {
            /* Puede que la GPU siga en el bus y solo haya perdido el decode de
             * memoria (lo que deja un reset de función). El espacio de
             * configuración lo dice; si es eso, se reactiva y se sigue esperando. */
            if (fsp_gone_recovered("mientras arrancaba el FMC")) {
                continue;
            }
            return -1;
        }
        if (lockdown_released(args_addr, &mbox0)) {
            break;
        }
        if ((time % 1000u) == 0u) {
            log_gsp_state("esperando");
        }
        lx_mdelay(1);
    }
    if (time == 0) {
        lx_printk("nouveau-lx: GSP-FMC no arrancó a tiempo\n");
        log_gsp_state("al agotarse la espera");
        return -1;
    }
    if (mbox0) {
        /* Upstream trata cualquier mbox0 distinto de cero como fallo, aunque
         * `lockdown_released` considere válido que lleve la dirección de los boot
         * params. Si sale justo ese caso, conviene saberlo antes de tocar nada. */
        if (mbox0 == (uint32_t)args_addr) {
            lx_printk("nouveau-lx: GSP-FMC deja en mbox0 la dirección de los boot args "
                      "(0x%08x) — caso ambiguo de gh100_gsp_lockdown_released\n", mbox0);
        }
        lx_printk("nouveau-lx: GSP-FMC falló (mbox0=0x%08x)\n", mbox0);
        return -1;
    }
    lx_printk("nouveau-lx: GSP-FMC arrancado, lockdown liberado\n");
    log_gsp_state("arrancado");
    return 0;
}
