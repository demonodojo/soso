/* G4e (1/2): canal GPFIFO. Ver gsp_chan.h. */
#include "gsp_chan.h"
#include "gsp_chip.h"
#include "gsp_mmio.h"
#include "gsp_pramin.h"
#include "gsp_top.h"

void *memset(void *dst, int c, unsigned long n);
void *memcpy(void *dst, const void *src, unsigned long n);

/* Mismo patrón que `gsp_rpc_barrier`: lo que se publica en sysmem tiene que
 * haber salido del core antes de tocar el registro que se lo dice a la GPU. */
static void gsp_chan_barrier(void)
{
    __asm__ __volatile__("mfence" ::: "memory");
}

/* Offsets de `Nvc56fControl` (clc56f.h / nvrm_r570.h). */
#define USERD_OFF_PUT    0x40u
#define USERD_OFF_GET    0x44u
#define USERD_OFF_GPGET  0x88u
#define USERD_OFF_GPPUT  0x8cu

static uint32_t chan_userd_rd32(struct gsp_chan *c, unsigned off)
{
    if (c->userd_vram) {
        return gsp_pramin_rd32(c->userd.phys + off);
    }
    return *(const volatile uint32_t *)((const char *)c->userd.va + off);
}

static void chan_userd_wr32(struct gsp_chan *c, unsigned off, uint32_t val)
{
    if (c->userd_vram) {
        gsp_pramin_wr32(c->userd.phys + off, val);
        return;
    }
    *(volatile uint32_t *)((char *)c->userd.va + off) = val;
}

static void chan_userd_clear(struct gsp_chan *c)
{
    unsigned off;

    for (off = 0; off < GSP_CHAN_USERD_HW_SIZE; off += 4u) {
        chan_userd_wr32(c, off, 0);
    }
}

static int chan_map_buf(struct gsp_chan *c, struct gsp_dma_buf *b, uint64_t va)
{
    return gsp_vmm_map(c->vmm, va, b->phys, b->size, GSP_VMM_SYSMEM);
}

/* Tamaño del method buffer, preguntado a RM sobre el subdevice igual que hace
 * `r535_fifo_ctor`. No hay valor por defecto razonable: si RM no contesta, no
 * sabemos qué poner en el descriptor y un número inventado es justo el error que
 * este cambio viene a quitar. Así que se falla y se dice por qué. */
static int chan_query_mthdbuf_size(struct gsp_chan *c, uint32_t *size)
{
    NV2080_CTRL_CE_GET_FAULT_METHOD_BUFFER_SIZE_PARAMS ctrl;
    uint32_t status = 0;

    memset(&ctrl, 0, sizeof(ctrl));
    lx_printk("nouveau-lx: preguntando el tamaño del method buffer "
              "(sub=0x%08x, %u B de params)\n",
              c->rm->subdevice, (unsigned)sizeof(ctrl));
    if (gsp_rm_control(c->rm, c->rm->subdevice,
                       NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE,
                       &ctrl, (uint32_t)sizeof(ctrl), &status) != 0) {
        lx_printk("nouveau-lx: CE_GET_FAULT_METHOD_BUFFER_SIZE rechazado "
                  "(status=0x%x)\n", status);
        return -1;
    }
    if (ctrl.size == 0 || ctrl.size > GSP_CHAN_MTHDBUF_MAX) {
        lx_printk("nouveau-lx: RM pide un method buffer de %u B (fuera de rango)\n",
                  ctrl.size);
        return -1;
    }
    *size = ctrl.size;
    return 0;
}

static int chan_fill_alloc(struct gsp_chan *c, NV_CHANNEL_ALLOC_PARAMS *p,
                           uint32_t vaspace)
{
    unsigned long gpfifo_bytes = GSP_CHAN_GPFIFO_ENTRIES * NVC56F_GP_ENTRY__SIZE;

    memset(p, 0, sizeof(*p));
    /* La VA del ring en el vaspace del canal, no su dirección física: RM
     * programa el PBDMA para buscar las GP entries a través del vaspace. Lo
     * delataba nuestro propio `gsp_chan_submit`, que mete VAs DENTRO de las
     * entradas — si el contenido es virtual, la base también. */
    p->gpFifoOffset = c->gpfifo_va;
    p->gpFifoEntries = GSP_CHAN_GPFIFO_ENTRIES;
    /* Upstream pone CLIENT_MAP_FIFO explícitamente a FALSE: es para que RM mapee
     * el FIFO en espacio de usuario, y aquí no hay usuario que valga. Lo que sí
     * hay que poner son los subcampos que upstream no deja a cero: canal
     * privilegiado (emparejado con el ADMIN de `internalFlags`, ver abajo) y el
     * slot de USERD, que es **por dónde se pide el chid**.
     *
     * Los índices salen del chid y no son constantes: ver el bloque de
     * NVOS04_FLAGS en nvrm_r570.h. Estaban clavados a 0/0 y por eso el segundo
     * canal —el de GR0— pedía el mismo slot que el primero teniéndolo declarado
     * fijo, y RM contestaba NO_MEMORY (2026-07-28). */
    /* Verbatim de `r535_chan_alloc` (runq=0 en GA107 mobile). */
    p->flags = NVOS04_FLAGS_CHANNEL_TYPE_PHYSICAL |
               NVOS04_FLAGS_VPR_FALSE |
               NVOS04_FLAGS_CHANNEL_SKIP_MAP_REFCOUNTING_FALSE |
               NVOS04_FLAGS_GROUP_CHANNEL_RUNQUEUE_VALUE(0u) |
               NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE |
               NVOS04_FLAGS_DELAY_CHANNEL_SCHEDULING_FALSE |
               NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE_FALSE |
               NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE(c->chid % NV_CHID_PER_USERD) |
               NVOS04_FLAGS_CHANNEL_USERD_INDEX_FIXED_FALSE |
               NVOS04_FLAGS_CHANNEL_USERD_INDEX_PAGE_VALUE(c->chid / NV_CHID_PER_USERD) |
               NVOS04_FLAGS_CHANNEL_USERD_INDEX_PAGE_FIXED_TRUE |
               NVOS04_FLAGS_CHANNEL_DENY_AUTH_LEVEL_PRIV_FALSE |
               NVOS04_FLAGS_CHANNEL_SKIP_SCRUBBER_FALSE |
               NVOS04_FLAGS_CHANNEL_CLIENT_MAP_FIFO_FALSE |
               NVOS04_FLAGS_SET_EVICT_LAST_CE_PREFETCH_CHANNEL_FALSE |
               NVOS04_FLAGS_CHANNEL_VGPU_PLUGIN_CONTEXT_FALSE |
               NVOS04_FLAGS_CHANNEL_PBDMA_ACQUIRE_TIMEOUT_FALSE |
               NVOS04_FLAGS_GROUP_CHANNEL_THREAD_DEFAULT |
               NVOS04_FLAGS_MAP_CHANNEL_FALSE |
               NVOS04_FLAGS_SKIP_CTXBUFFER_ALLOC_FALSE;
    p->hVASpace = vaspace;
    p->engineType = c->engine;
    /* `subDeviceId` se queda a 0 (upstream no lo toca): el 1 que había aquí
     * apuntaba a un subdevice que no es el nuestro. */

    /* Bloque de instancia y RAMFC, los dos en VRAM y apuntando al mismo sitio;
     * el RAMFC son los primeros 0x200 B del bloque. Faltaban por completo —a
     * ceros— y son obligatorios: RM los usa para construir el canal. */
    p->instanceMem.base = c->inst_addr;
    p->instanceMem.size = GSP_CHAN_INST_SIZE;
    p->instanceMem.addressSpace = NV_ADDRESS_SPACE_FBMEM;
    p->instanceMem.cacheAttrib = NV_CACHE_ATTR_UNCACHED;
    p->ramfcMem.base = c->inst_addr;
    p->ramfcMem.size = GSP_CHAN_RAMFC_SIZE;
    p->ramfcMem.addressSpace = NV_ADDRESS_SPACE_FBMEM;
    p->ramfcMem.cacheAttrib = NV_CACHE_ATTR_UNCACHED;

    /* USERD en VRAM como `r535_chan_alloc`: ESCHED sondea GPGet/GPPut en FB.
     * La CPU los toca por PRAMIN (`gsp_pramin_*`), no por sysmem. */
    p->userdMem.base = c->userd.phys;
    p->userdMem.size = GSP_CHAN_USERD_HW_SIZE;
    p->userdMem.addressSpace = NV_ADDRESS_SPACE_FBMEM;
    p->userdMem.cacheAttrib = NV_CACHE_ATTR_UNCACHED;
    /* Búfer propio y del tamaño que dijo RM. Antes iba aquí el pushbuffer con
     * 4096 a ojo: dos errores a la vez, el búfer y el número. */
    p->mthdbufMem.base = c->mthdbuf.phys;
    p->mthdbufMem.size = c->mthdbuf_size;
    p->mthdbufMem.addressSpace = NV_ADDRESS_SPACE_SYSMEM;
    p->mthdbufMem.cacheAttrib = NV_CACHE_ATTR_CACHED;

    /* `errorNotifierMem` NO se rellena: upstream no lo toca y en su lugar dice
     * en `internalFlags` que no hay notificador. Rellenarlo mientras el flag
     * decía UNKNOWN era pedirle a RM dos cosas incompatibles. El búfer del
     * notificador sigue reservado y mapeado; lo usa el CE por su cuenta.
     * El ADMIN va emparejado con el PRIVILEGED_CHANNEL_TRUE de `flags`: upstream
     * mueve los dos a la vez según su `priv`, y aquí somos el kernel. */
    p->internalFlags = NV_KERNELCHANNEL_ALLOC_INTERNALFLAGS_PRIVILEGE_ADMIN |
                       NV_KERNELCHANNEL_ALLOC_INTERNALFLAGS_ERROR_NOTIFIER_TYPE_NONE |
                       NV_KERNELCHANNEL_ALLOC_INTERNALFLAGS_ECC_ERROR_NOTIFIER_TYPE_NONE;
    (void)gpfifo_bytes;
    return 0;
}

/* El andamio de las once variantes vivió aquí un día (2026-07-28) y ya no hace
 * falta: la variante base pasó en cuanto se arregló `NV_MAX_SUBDEVICES`, que era
 * lo único que estaba mal. Que las once fallaran idénticas era la pista —cuando
 * ninguna hipótesis de campo cambia nada, lo que está mal no es un campo—, y
 * queda contada en el comentario del assert de tamaño de nvrm_r570.h.
 *
 * Arrancar el canal, en cambio, son tres pasos que faltaban por completo. Ver el
 * bloque de `NVA06F_CTRL_CMD_BIND` en nvrm_r570.h. */
static int chan_bind_engine(struct gsp_chan *c)
{
    NVA06F_CTRL_BIND_PARAMS bind;
    uint32_t status = 0;

    memset(&bind, 0, sizeof(bind));
    /* El mismo motor que se pidió en el alloc. Que estos dos números pudieran
     * separarse era una trampa esperando: un canal alojado sobre GR0 y atado a
     * COPY0 se reserva sin queja y falla después, al colgarle el objeto. */
    bind.engineType = c->engine;
    if (gsp_rm_control(c->rm, c->handle, NVA06F_CTRL_CMD_BIND,
                       &bind, (uint32_t)sizeof(bind), &status) != 0) {
        lx_printk("nouveau-lx: canal BIND(engine=%u) rechazado (status=0x%x)\n",
                  bind.engineType, status);
        return -1;
    }
    lx_printk("nouveau-lx: canal atado al motor %u\n", bind.engineType);
    return 0;
}

static int chan_schedule(struct gsp_chan *c)
{
    NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS sched;
    uint32_t status = 0;

    memset(&sched, 0, sizeof(sched));
    sched.bEnable = 1;
    if (gsp_rm_control(c->rm, c->handle, NVA06F_CTRL_CMD_GPFIFO_SCHEDULE,
                       &sched, (uint32_t)sizeof(sched), &status) != 0) {
        lx_printk("nouveau-lx: canal GPFIFO_SCHEDULE rechazado (status=0x%x)\n",
                  status);
        return -1;
    }
    lx_printk("nouveau-lx: canal en la runlist (SCHEDULE bEnable=1)\n");
    return 0;
}

/* Sin token no hay forma de patear el canal, así que esto NO es best-effort: si
 * falla, el canal se queda a medias y es mejor decirlo aquí que dejar que el CE
 * se coma un timeout de 2 s sin explicación. */
static int chan_get_doorbell_token(struct gsp_chan *c)
{
    NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS tok;
    uint32_t status = 0;

    memset(&tok, 0, sizeof(tok));
    if (gsp_rm_control(c->rm, c->handle,
                       NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN,
                       &tok, (uint32_t)sizeof(tok), &status) != 0) {
        lx_printk("nouveau-lx: canal GET_WORK_SUBMIT_TOKEN rechazado "
                  "(status=0x%x)\n", status);
        return -1;
    }
    c->doorbell_token = tok.workSubmitToken;
    c->doorbell_kick = gsp_chan_doorbell_kick(gsp_nv_family_current(),
                                              c->doorbell_token);
    c->doorbell_ok = 1;
    /* El RPC devuelve runlist+chid para la vía interna de GSP-RM. El kick en
     * usermode lo construye el driver — en gb20x con bit 30 (`gb202_chan_doorbell_handle`). */
    lx_printk("nouveau-lx: doorbell RPC=0x%08x (runlist=%u chid=%u) kick=0x%08x "
              "bit30=%u motor=%u\n",
              c->doorbell_token, c->doorbell_token >> 16,
              c->doorbell_token & NV_VF_DOORBELL_VECTOR_MASK,
              c->doorbell_kick,
              (unsigned)((c->doorbell_kick & NV_VF_DOORBELL_RUNLIST_DOORBELL_ENABLE) != 0),
              c->engine);
    /* El contraste que prueba la teoría: si el chid que sale del token es el que
     * pedimos por los subcampos de USERD, entonces ésa es la vía —y dos canales con
     * el mismo valor se pisan, que es lo que se cree que dio el NO_MEMORY. Si RM
     * devuelve otro, el chid lo elige él y hay que mirar en otra parte. */
    if ((c->doorbell_token & NV_VF_DOORBELL_VECTOR_MASK) != c->chid) {
        lx_printk("nouveau-lx: pedimos chid=%u por los índices de USERD y RM usa "
                  "el %u — el chid no se pide por ahí\n",
                  c->chid, c->doorbell_token & NV_VF_DOORBELL_VECTOR_MASK);
    }
    return 0;
}

/* ¿Está el aperture de usermode donde creemos? El doorbell es una escritura ciega
 * —no hay nada que leer de vuelta en 0x90— así que un aperture en la dirección
 * equivocada se ve exactamente igual que un canal que no arranca: nada pasa. Lo
 * que sí se puede leer es el reloj: en `clc361.h` el usermode expone TIME_0/TIME_1
 * en +0x80/+0x84 (el doorbell es el +0x90 que escribe `tu102_chan_start`), y el
 * mismo reloj está en el espacio privilegiado en `NV04_PTIMER_TIME_0` = 0x9400.
 * Si el privilegiado avanza y el de usermode no, el aperture no está en 0xbb0000
 * —o su +0x80 no es TIME— y entonces el +0x90 tampoco es el timbre de nadie.
 *
 * Sólo diagnostica: no cambia ninguna decisión, pero el veredicto queda guardado
 * (0 = sin verificar, 1 = el aperture responde, -1 = no responde) para que el
 * volcado del canal pueda repetirlo sin volver a sondear y para que la prueba del
 * host pueda exigir que el camino malo se detecte. */
static int g_usermode_verdict;

static void chan_probe_usermode(void)
{
    uint32_t um0 = gsp_mmio_rd32(NV_VFN_USERMODE_BASE + 0x80u);
    uint32_t pt0 = gsp_mmio_rd32(0x9400u);
    uint32_t um1;
    uint32_t pt1;

    lx_mdelay(1);
    um1 = gsp_mmio_rd32(NV_VFN_USERMODE_BASE + 0x80u);
    pt1 = gsp_mmio_rd32(0x9400u);

    lx_printk("nouveau-lx: usermode 0x%06x: TIME=0x%08x→0x%08x; PTIMER 0x009400: "
              "0x%08x→0x%08x\n", NV_VFN_USERMODE_BASE, um0, um1, pt0, pt1);

    if (gsp_mmio_pri_error(um0) || um0 == 0xffffffffu) {
        g_usermode_verdict = -1;
        lx_printk("nouveau-lx: el aperture de usermode no contesta (0x%08x) — el "
                  "doorbell de 0x%06x se escribe en el vacío\n",
                  um0, NV_VFN_DOORBELL);
        return;
    }
    if (pt0 == pt1) {
        lx_printk("nouveau-lx: el PTIMER privilegiado tampoco avanza (0x%08x) — la "
                  "prueba no concluye nada sobre el usermode\n", pt0);
        return;
    }
    if (um0 == um1) {
        g_usermode_verdict = -1;
        lx_printk("nouveau-lx: el PTIMER avanza y el TIME de usermode no: o el "
                  "aperture no está en 0x%06x o su +0x80 no es TIME. El doorbell "
                  "queda sin verificar\n", NV_VFN_USERMODE_BASE);
        return;
    }
    g_usermode_verdict = 1;
    lx_printk("nouveau-lx: aperture de usermode vivo (su reloj avanza) — el "
              "doorbell 0x%06x es una dirección real\n", NV_VFN_DOORBELL);
}

static int chan_start(struct gsp_chan *c)
{
    if (chan_bind_engine(c) != 0 || chan_schedule(c) != 0 ||
        chan_get_doorbell_token(c) != 0) {
        return -1;
    }
    /* Con el token en la mano y antes del primer submit: así el log dice si el
     * timbre existe **antes** de que el semáforo se coma dos segundos. */
    chan_probe_usermode();
    return 0;
}

int gsp_chan_init(struct gsp_rm *rm, struct gsp_vmm *vmm, struct gsp_vram *vram,
                  struct gsp_chan *c, uint32_t vaspace, unsigned idx,
                  uint32_t engine)
{
    NV_CHANNEL_ALLOC_PARAMS params;
    uint64_t va_base;

    if (!rm || !vmm || !vram || !c || !rm->ready || !vmm->ready || !vram->ready) {
        return -1;
    }
    /* El tope no es decorativo: la ventana del canal `idx` empieza en
     * GSP_CHAN_VA_BASE + idx*stride, y el compute tiene su sysmem en
     * GSP_VA_BASE + 0x30000000. Con idx grande una ventana se comería la otra y el
     * síntoma sería un kernel leyendo el GPFIFO como si fueran sus datos. */
    if (idx >= (0x10000000ull / GSP_CHAN_VA_STRIDE)) {
        lx_printk("nouveau-lx: canal idx=%u fuera de su ventana de VAs\n", idx);
        return -1;
    }
    memset(c, 0, sizeof(*c));
    c->rm = rm;
    c->vmm = vmm;
    c->handle = NVKM_RM_CHAN(idx);
    /* Upstream (`r535_fifo_runl_ctor`) empieza en `rsvd_chids`; en 570.144 vale 1
     * (r570/fifo.c). Pedir chid=0 deja USERD_INDEX=0 fijo y RM contesta NO_MEMORY
     * en GA107 (run12). */
    c->chid = idx + GSP_CHAN_RSVD_CHIDS;
    c->engine = engine;
    /* De más nueva a más vieja. `rm/gb20x.c` de nouveau usa la B de Blackwell
     * para este chip; el catálogo del chip decide y esta lista solo ordena. */
    {
        static const uint32_t cand[] = {
            BLACKWELL_CHANNEL_GPFIFO_B, BLACKWELL_CHANNEL_GPFIFO_A,
            HOPPER_CHANNEL_GPFIFO_A, AMPERE_CHANNEL_GPFIFO_B,
            AMPERE_CHANNEL_GPFIFO_A,
        };

        c->cls = gsp_rm_class_pick("canal GPFIFO", cand,
                                   (unsigned)(sizeof(cand) / sizeof(cand[0])));
    }

    /* Antes de reservar nada: el tamaño del method buffer lo manda RM. */
    if (chan_query_mthdbuf_size(c, &c->mthdbuf_size) != 0) {
        return -1;
    }

    if (gsp_dma_alloc(&c->gpfifo, GSP_CHAN_GPFIFO_SIZE, "GPFIFO") != 0 ||
        gsp_dma_alloc(&c->pushbuf, GSP_CHAN_PB_SIZE, "pushbuffer") != 0 ||
        gsp_dma_alloc(&c->notifier, GSP_CHAN_NOTIFIER_SIZE, "notifier CE") != 0 ||
        gsp_dma_alloc(&c->mthdbuf, c->mthdbuf_size, "method buffer CE") != 0) {
        gsp_chan_fini(c);
        return -1;
    }

    c->userd_vram = 1;
    c->userd.phys = gsp_vram_alloc(vram, GSP_CHAN_USERD_SIZE, GSP_CHAN_USERD_SIZE);
    c->userd.size = GSP_CHAN_USERD_SIZE;
    c->userd.va = NULL;
    if (c->userd.phys == 0) {
        lx_printk("nouveau-lx: sin VRAM para el USERD del canal\n");
        gsp_chan_fini(c);
        return -1;
    }

    /* El bloque de instancia va en VRAM y no se mapea en el vaspace: RM lo
     * direcciona físicamente. gsp_vram_alloc devuelve 0 cuando no cabe. */
    c->inst_addr = gsp_vram_alloc(vram, GSP_CHAN_INST_SIZE, GSP_CHAN_INST_SIZE);
    if (c->inst_addr == 0) {
        lx_printk("nouveau-lx: sin VRAM para el bloque de instancia del canal\n");
        gsp_chan_fini(c);
        return -1;
    }

    va_base = GSP_CHAN_VA_BASE + (uint64_t)idx * GSP_CHAN_VA_STRIDE;
    /* GPFIFO es el grande (32 KiB con 4096 entradas); PB/notifier detrás.
     * Antes estaban a +4/+8/+12 KiB y el GPFIFO de 32 KiB los pisaba. */
    c->gpfifo_va = va_base;
    c->pushbuf_va = va_base + (uint64_t)GSP_CHAN_GPFIFO_SIZE;
    c->notifier_va = c->pushbuf_va + (uint64_t)GSP_CHAN_PB_SIZE;
    c->userd_va = c->notifier_va + (uint64_t)GSP_CHAN_NOTIFIER_SIZE;

    if (chan_map_buf(c, &c->gpfifo, c->gpfifo_va) != 0 ||
        chan_map_buf(c, &c->pushbuf, c->pushbuf_va) != 0 ||
        chan_map_buf(c, &c->notifier, c->notifier_va) != 0) {
        gsp_chan_fini(c);
        return -1;
    }

    chan_userd_clear(c);
    chan_fill_alloc(c, &params, vaspace);

    lx_printk("nouveau-lx: RM_ALLOC canal cls=0x%04x chid=%u inst=0x%llx "
              "userd=0x%llx mthdbuf=0x%llx/%u B flags=0x%08x\n",
              c->cls, c->chid, (unsigned long long)c->inst_addr,
              (unsigned long long)c->userd.phys,
              (unsigned long long)c->mthdbuf.phys, c->mthdbuf_size,
              params.flags);

    if (gsp_rm_alloc(c->rm, c->rm->device, c->handle, c->cls,
                     &params, (uint32_t)sizeof(params), 0) != 0) {
        lx_printk("nouveau-lx: RM_ALLOC canal GPFIFO falló (flags=0x%08x chid=%u)\n",
                  params.flags, c->chid);
        gsp_chan_fini(c);
        return -1;
    }

    /* `ready` ANTES del arranque porque `gsp_chan_fini` lo mira para saber si hay
     * que soltar el objeto en RM: si el BIND o el SCHEDULE fallan, el canal ya
     * existe del lado de RM y dejarlo sin liberar es una fuga que además deja al
     * GSP con un canal en la runlist cuando descarguemos. */
    c->ready = 1;
    if (chan_start(c) != 0) {
        lx_printk("nouveau-lx: canal reservado pero sin arrancar — no se encola "
                  "nada sobre él\n");
        gsp_chan_fini(c);
        return -1;
    }


    lx_printk("nouveau-lx: canal GPFIFO listo cls=0x%04x handle=0x%08x motor=%u "
              "gpfifo=0x%llx (VA) "
              "userd=0x%llx+0x%x inst=0x%llx (VRAM) mthdbuf=%u B\n",
              c->cls, c->handle, c->engine, (unsigned long long)c->gpfifo_va,
              (unsigned long long)c->userd.phys, GSP_CHAN_USERD_HW_SIZE,
              (unsigned long long)c->inst_addr, c->mthdbuf_size);
    return 0;
}

int gsp_chan_pb_reserve(struct gsp_chan *c, unsigned bytes)
{
    unsigned off;

    if (!c || !c->ready) {
        return -1;
    }
    off = (c->pb_pos + 3u) & ~3u;
    if (off + bytes > GSP_CHAN_PB_SIZE) {
        return -1;
    }
    c->pb_pos = off + bytes;
    return (int)off;
}

/* G5 lanza un QMD por tanda de filas: el PB de 4 KiB se agota y hay que
 * rebobinar. En Blackwell el USERD ya no escribe GPGet (ver `gpget` en el
 * header); la señal de "el host lo consumió" es `gsp_chan_ack_progress` tras
 * el wait del semáforo. */
int gsp_chan_pb_rewind(struct gsp_chan *c)
{
    if (!c || !c->ready) {
        return -1;
    }
    if (c->pb_pos == 0) {
        return 0;
    }
    gsp_chan_barrier();
    if (c->gpget != c->gpput) {
        lx_printk("nouveau-lx: pushbuffer sin rebobinar: gpget=%u gpput=%u "
                  "(USERD GPGet=%u, cosmético en Blackwell)\n",
                  c->gpget, c->gpput, chan_userd_rd32(c, USERD_OFF_GPGET));
        return -1;
    }
    c->pb_pos = 0;
    return 0;
}

void gsp_chan_ack_progress(struct gsp_chan *c)
{
    if (!c || !c->ready) {
        return;
    }
    /* Solo progreso SW. Upstream (nouveau Skeggs / nvidia-push) no escribe
     * USERD.GPGet: en Blackwell el writeback no existe y el espacio libre se
     * decide por el semáforo CE/QMD que acaba de señalizar. GPPut en USERD
     * ya va módulo ENTRIES en submit (como nvidia-push), no hace falta
     * resetearlo aquí. */
    c->gpget = c->gpput;
    gsp_chan_barrier();
}

int gsp_chan_submit(struct gsp_chan *c, unsigned pb_off, unsigned pb_len)
{
    uint64_t gp_get;
    uint32_t e0, e1;
    unsigned idx;
    unsigned char *ring;

    if (!c || !c->ready || !pb_len || pb_off + pb_len > GSP_CHAN_PB_SIZE) {
        return -1;
    }
    if (c->gpput - c->gpget >= GSP_CHAN_GPFIFO_ENTRIES) {
        lx_printk("nouveau-lx: GPFIFO lleno (gpget=%u gpput=%u, %u entradas) — "
                  "falta ack tras el wait del semáforo\n",
                  c->gpget, c->gpput, GSP_CHAN_GPFIFO_ENTRIES);
        return -1;
    }

    gp_get = (uint64_t)c->pushbuf_va + pb_off;
    /* `GET_HI` son 8 bits (`clc56f.h`, 7:0), así que una entrada de GPFIFO no
     * alcanza más allá del bit 39. Antes esto se truncaba con un `& 0xff` mudo y
     * con la base del mapa en 1 TiB se perdía justo el bit 40: el host iba a
     * buscar el pushbuffer a otra parte y el semáforo del CE no subía nunca. Un
     * truncado silencioso aquí es indistinguible de un submit correcto. */
    if (gp_get > GSP_GPFIFO_VA_MAX) {
        lx_printk("nouveau-lx: pushbuffer en VA 0x%llx — una entrada de GPFIFO "
                  "sólo llega a 0x%llx (GET 31:2 + GET_HI 7:0)\n",
                  (unsigned long long)gp_get,
                  (unsigned long long)GSP_GPFIFO_VA_MAX);
        return -1;
    }
    idx = c->gpput % GSP_CHAN_GPFIFO_ENTRIES;
    ring = (unsigned char *)c->gpfifo.va + idx * NVC56F_GP_ENTRY__SIZE;

    /* `GP_ENTRY0_GET` es el campo 31:2 y **contiene** los bits 31:2 de la
     * dirección, o sea que la entrada son los 32 bits bajos tal cual — que es lo
     * que escribe `nv50_dma_push` de nouveau (`lower_32_bits(offset)`). Aquí
     * había un `(gp_get >> 2)`, que mete la dirección corrida dos bits y manda al
     * host a un cuarto de donde está el pushbuffer. Los dos bits bajos se
     * enmascaran porque el 0 es FETCH y el 1 no es del campo. */
    e0 = NVC56F_GP_ENTRY0_FETCH_UNCONDITIONAL |
         (uint32_t)(gp_get & 0xfffffffcu);
    /* LENGTH (30:10) va en dwords. El `length << 8` de nouveau es sobre bytes:
     * mismo número, distinta unidad. */
    e1 = (uint32_t)((gp_get >> 32) & 0xffu) |
         (NVC56F_GP_ENTRY1_LEVEL_MAIN << 9) |
         (((pb_len + 3u) / 4u) << 10) |
         (NVC56F_GP_ENTRY1_SYNC_PROCEED << 31);

    memcpy(ring, &e0, 4);
    memcpy(ring + 4, &e1, 4);

    /* La entrada tiene que estar ENTERA en memoria antes de publicar GPPut, o el
     * host puede leer un GPPut nuevo y una entrada a medio escribir. `nv50_dma_push`
     * hace `mb()` y acto seguido una LECTURA del ring por lo mismo: en memoria
     * write-combining la barrera ordena pero no vacía, y la lectura sí. */
    gsp_chan_barrier();
    (void)*(const volatile uint32_t *)ring;

    c->gpput++;
    /* GPPut en USERD es índice 0..ENTRIES-1 (nvidia-push enmascara igual).
     * Escribir el contador libre (4096, 4097…) cuelga el PBDMA en Blackwell. */
    chan_userd_wr32(c, USERD_OFF_GPPUT, c->gpput % GSP_CHAN_GPFIFO_ENTRIES);
    chan_userd_wr32(c, USERD_OFF_PUT, pb_off + pb_len);

    /* Y el doorbell, que es lo que faltaba: en Volta+ escribir GPPut en el USERD
     * no patea nada por sí solo —el host no sondea el USERD—, hay que avisarle
     * por el registro de usermode. Sin esto el canal se queda con el trabajo
     * encolado y el semáforo a 0 para siempre, que es exactamente lo que hacía
     * (2026-07-28).
     *
     * El USERD vive en VRAM (GPGet/GPPut por PRAMIN); el ring GPFIFO en sysmem
     * mapeada. El orden lo garantiza mfence antes del doorbell. */
    gsp_chan_barrier();
    if (c->doorbell_ok) {
        gsp_mmio_wr32(NV_VFN_DOORBELL, c->doorbell_kick);
    }
    return 0;
}

/* Registros del FIFO que se leen para el volcado. Todos salen de nouveau por el
 * camino ga100/tu102 —el que hereda Blackwell— y todos son de LECTURA: en el
 * camino GSP el FIFO lo programa RM y aquí sólo se mira.
 *
 *   runlist+0x004  chcfg   `ga100_runl_new`: chnum = 1 << (chcfg & 0xf) y
 *                          channel RAM = chcfg & 0xfffffff0
 *   runlist+0x008  dbcfg   doorbell = dbcfg >> 16; el token de RM es
 *                          (doorbell << 16) | chid (`ga100_chan_doorbell_handle`)
 *   runlist+0x010/14 pbcfg bit 31 = ese PBDMA existe;
 *                          id = ((pbcfg & 0x03fffc00) - 0x040000) / 0x800
 *   runlist+0x100  INTR_0  (`ga100_runl_init`)
 *   chram + chid*4         estado del canal: `ga100_chan_start` escribe 2,
 *                          `_stop` 3 y `_unbind` 0xffffffff
 *   PBDMA base 0x040000 + id*0x2000 (`gf100_runq_intr`): +0x0c0 método que se
 *                          estaba ejecutando (subc 18:16, mthd 13:2), +0x0c4 su
 *                          dato, +0x108 INTR_0, +0x10c su máscara, +0x120 chid,
 *                          +0x148 INTR_1.
 *
 * Un 0 en los INTR no prueba que no pasara nada: RM los limpia al atenderlos, y
 * en este camino los atiende él. Lo que sí es concluyente es un bit puesto. */
#define RUNL_CHCFG   0x004u
#define RUNL_DBCFG   0x008u
#define RUNL_PBCFG0  0x010u
#define RUNL_INTR0   0x100u
#define PBDMA_BASE   0x040000u
#define PBDMA_STRIDE 0x2000u
#define PBDMA_MTHD   0x0c0u
#define PBDMA_DATA   0x0c4u
#define PBDMA_INTR0  0x108u
#define PBDMA_INTR0_EN 0x10cu
#define PBDMA_CHID   0x120u
#define PBDMA_INTR1  0x148u
#define PBDMA_INTR0_ILLEGAL_MTHD 0x00200000u
#define PBDMA_INTR0_EMPTY_SUBC   0x00800000u
/* Los 16 MiB de BAR0 que mapea `gsp_mmio_set_bar`. */
#define BAR0_LIMIT   0x1000000u

static void chan_dump_pbdma(const char *why, unsigned id)
{
    uint32_t base = PBDMA_BASE + id * PBDMA_STRIDE;
    uint32_t intr0;
    uint32_t addr;

    if (base + PBDMA_INTR1 >= BAR0_LIMIT) {
        lx_printk("nouveau-lx: canal (%s): PBDMA%u cae fuera de BAR0 (0x%06x)\n",
                  why, id, base);
        return;
    }
    intr0 = gsp_mmio_rd32(base + PBDMA_INTR0);
    addr = gsp_mmio_rd32(base + PBDMA_MTHD);
    lx_printk("nouveau-lx: canal (%s): PBDMA%u INTR_0=%08x (en=%08x) INTR_1=%08x "
              "chid=%08x mthd=%04x subc=%u data=%08x\n", why, id, intr0,
              gsp_mmio_rd32(base + PBDMA_INTR0_EN),
              gsp_mmio_rd32(base + PBDMA_INTR1),
              gsp_mmio_rd32(base + PBDMA_CHID),
              (unsigned)(addr & 0x3ffcu), (unsigned)((addr >> 16) & 7u),
              gsp_mmio_rd32(base + PBDMA_DATA));
    if (intr0 & PBDMA_INTR0_EMPTY_SUBC) {
        lx_printk("nouveau-lx: canal (%s): PBDMA%u EMPTY_SUBC — métodos a una "
                  "subchannel sin objeto atado (mira el SET_OBJECT)\n", why, id);
    }
    if (intr0 & PBDMA_INTR0_ILLEGAL_MTHD) {
        lx_printk("nouveau-lx: canal (%s): PBDMA%u ILLEGAL_MTHD — el método de "
                  "arriba no es de la clase atada a esa subchannel\n", why, id);
    }
}

/* La otra mitad del volcado: el USERD dice qué creemos nosotros, esto dice qué
 * ve el silicio. Con `GPGet=0` las dos explicaciones (el host no se enteró / el
 * host se enteró y falló) se ven igual desde sysmem; aquí se separan, porque el
 * estado del canal y los INTR del PBDMA son de la GPU y no los escribimos. */
/* Dos fuentes para la misma dirección: PTOP —que la publica el chip— y la tabla
 * del FIFO de RM leída por un índice que inferimos nosotros. La primera manda; la
 * segunda se compara para saber de una vez si `data[11]` significa lo que creíamos.
 * Devuelve 0 y la base a usar, o -1 si ninguna de las dos la da. */
static int chan_runlist_base(struct gsp_chan *c, const char *why, uint32_t *out,
                             uint32_t *chram_tab)
{
    uint32_t from_rm = 0;
    uint32_t from_top = 0;
    uint8_t type = 0, inst = 0;
    int have_rm = gsp_rm_engine_fifo_regs(c->engine, &from_rm, chram_tab) == 0;
    int have_top = gsp_top_type_of_engine(c->engine, &type, &inst) == 0 &&
                   gsp_top_runlist_of(type, inst, &from_top, NULL) == 0;

    if (have_top && have_rm) {
        lx_printk("nouveau-lx: canal (%s): runlist del motor %u — PTOP dice "
                  "0x%06x, la tabla del FIFO 0x%06x%s\n", why, c->engine,
                  from_top, from_rm,
                  from_top == from_rm ? " (coinciden)" : " (NO coinciden)");
    }
    if (have_top && from_top != 0u && from_top < BAR0_LIMIT) {
        *out = from_top;
        return 0;
    }
    if (have_rm) {
        lx_printk("nouveau-lx: canal (%s): sin runlist por PTOP, se usa la de la "
                  "tabla del FIFO (índice inferido)\n", why);
        *out = from_rm;
        return 0;
    }
    lx_printk("nouveau-lx: canal (%s): nadie sabe dónde está la runlist del motor "
              "%u — sin registros\n", why, c->engine);
    return -1;
}

static void chan_dump_ramfc(struct gsp_chan *c, const char *why)
{
    uint32_t w;
    uint64_t userd_inst, gpfifo_inst;
    unsigned i;
    int userd_ok, gpfifo_ok;

    if (!c->inst_addr) {
        return;
    }
    if (!gsp_pramin_alive()) {
        lx_printk("nouveau-lx: canal (%s): RAMFC — ventana PRAMIN muerta, no se "
                  "puede leer el inst block en 0x%llx\n", why,
                  (unsigned long long)c->inst_addr);
        return;
    }

    lx_printk("nouveau-lx: canal (%s): RAMFC inst block 0x%llx (PRAMIN):\n", why,
              (unsigned long long)c->inst_addr);
    for (i = 0; i < 0x200u; i += 16u) {
        uint32_t a, b, d, e;

        a = gsp_pramin_rd32(c->inst_addr + i);
        b = gsp_pramin_rd32(c->inst_addr + i + 4u);
        d = gsp_pramin_rd32(c->inst_addr + i + 8u);
        e = gsp_pramin_rd32(c->inst_addr + i + 12u);
        if (gsp_mmio_pri_error(a) && gsp_mmio_pri_error(b)) {
            lx_printk("nouveau-lx: canal (%s): RAMFC +0x%03x = badf — inst block "
                      "ilegible\n", why, i);
            return;
        }
        lx_printk("nouveau-lx: canal (%s): RAMFC +0x%03x: %08x %08x %08x %08x\n",
                  why, i, a, b, d, e);
    }

    w = gsp_pramin_rd32(c->inst_addr + 0x020u);
    userd_inst = (uint64_t)w;
    w = gsp_pramin_rd32(c->inst_addr + 0x090u);
    gpfifo_inst = (uint64_t)w;
    w = gsp_pramin_rd32(c->inst_addr + 0x094u);
    gpfifo_inst |= (uint64_t)w << 32;

    userd_ok = userd_inst == (c->userd.phys >> 12);
    gpfifo_ok = gpfifo_inst == c->gpfifo_va;
    lx_printk("nouveau-lx: canal (%s): RAMFC USERD pág inst=0x%llx enviado=0x%llx "
              "(phys 0x%llx) %s\n", why, (unsigned long long)userd_inst,
              (unsigned long long)(c->userd.phys >> 12),
              (unsigned long long)c->userd.phys, userd_ok ? "OK" : "NO_COINCIDE");
    lx_printk("nouveau-lx: canal (%s): RAMFC GPFIFO inst=0x%llx enviado=0x%llx "
              "entries=%u %s\n", why, (unsigned long long)gpfifo_inst,
              (unsigned long long)c->gpfifo_va, GSP_CHAN_GPFIFO_ENTRIES,
              gpfifo_ok ? "OK" : "NO_COINCIDE");
}

static void chan_dump_hw(struct gsp_chan *c, const char *why)
{
    uint32_t runl = 0, chram_tab = 0;
    uint32_t chcfg, dbcfg, chram, chnum, chid;
    unsigned i;

    chan_dump_ramfc(c, why);

    if (!gsp_mmio_alive()) {
        lx_printk("nouveau-lx: canal (%s): sin registros — la GPU no contesta "
                  "en el bus\n", why);
        return;
    }
    if (chan_runlist_base(c, why, &runl, &chram_tab) != 0) {
        return;
    }
    chcfg = gsp_mmio_rd32(runl + RUNL_CHCFG);
    dbcfg = gsp_mmio_rd32(runl + RUNL_DBCFG);
    if (chcfg == 0xffffffffu || dbcfg == 0xffffffffu) {
        lx_printk("nouveau-lx: canal (%s): runlist 0x%06x se lee a unos — o no es "
                  "esa dirección o la GPU está fuera del bus\n", why, runl);
        return;
    }
    chram = chcfg & 0xfffffff0u;
    chnum = 1u << (chcfg & 0xfu);
    chid = c->doorbell_token & 0xffffu;

    {
        uint32_t intr0 = gsp_mmio_rd32(runl + RUNL_INTR0);

        lx_printk("nouveau-lx: canal (%s): runlist 0x%06x chcfg=%08x (chram=0x%06x, "
                  "%u canales) dbcfg=%08x → doorbell=%u, INTR_0=%08x%s\n", why,
                  runl, chcfg, chram, chnum, dbcfg, dbcfg >> 16, intr0,
                  gsp_mmio_pri_error(intr0) ? " (¡error de PRI!)" : "");
        if (gsp_mmio_pri_error(chcfg) || gsp_mmio_pri_error(dbcfg) ||
            gsp_mmio_pri_error(intr0)) {
            lx_printk("nouveau-lx: canal (%s): ese bloque no contesta al anillo PRI "
                      "— o 0x%06x no es la runlist en este chip, o el bloque no "
                      "está inicializado\n", why, runl);
            return;
        }
    }

    /* Aquí se cae el índice de `engineData` si lo hemos elegido mal: `chcfg` lo
     * escribe el chip y la tabla la escribe RM. Que coincidan es la prueba; que
     * no, la señal de que lo que sigue no vale nada.
     *
     * En GB205 (2026-07-28) NO coinciden: `data[11]`=0xd00000 da chcfg=0x00000002
     * —chram 0 y "4 canales", que no es un chcfg de nada— y su +0x100 devuelve
     * 0xbadf5040. Así que en este chip esta ruta no vale, y por eso el volcado se
     * para aquí en vez de inventarse un estado de canal. */
    if (chram_tab != 0u && chram != chram_tab) {
        lx_printk("nouveau-lx: canal (%s): la channel RAM del registro (0x%06x) no "
                  "es la de la tabla (0x%06x) — el índice de engineData que usamos "
                  "no significa lo que creemos\n", why, chram, chram_tab);
        return;
    }
    /* El token lo da RM y su mitad alta tiene que ser este mismo dbcfg. Si no,
     * estamos escribiendo el doorbell de otra runlist. */
    if (c->doorbell_ok && (dbcfg >> 16) != (c->doorbell_token >> 16)) {
        lx_printk("nouveau-lx: canal (%s): el token de RM dice doorbell=%u y el "
                  "registro dice %u\n", why, c->doorbell_token >> 16, dbcfg >> 16);
    }
    if (chid < chnum && chram + chid * 4u < BAR0_LIMIT) {
        lx_printk("nouveau-lx: canal (%s): chid=%u → chram[0x%06x]=%08x "
                  "(2=corriendo, 3=parado, ffffffff=sin atar)\n", why, chid,
                  chram + chid * 4u, gsp_mmio_rd32(chram + chid * 4u));
    } else {
        lx_printk("nouveau-lx: canal (%s): chid=%u fuera de los %u canales de la "
                  "runlist\n", why, chid, chnum);
    }

    for (i = 0; i < 2u; i++) {
        uint32_t pbcfg = gsp_mmio_rd32(runl + RUNL_PBCFG0 + i * 4u);
        uint32_t field = pbcfg & 0x03fffc00u;

        if (!(pbcfg & 0x80000000u)) {
            continue;
        }
        if (field < PBDMA_BASE) {
            lx_printk("nouveau-lx: canal (%s): pbcfg%u=%08x no da un PBDMA "
                      "plausible\n", why, i, pbcfg);
            continue;
        }
        chan_dump_pbdma(why, (field - PBDMA_BASE) / 0x800u);
    }
}

void gsp_chan_dump(struct gsp_chan *c, const char *why)
{
    const uint32_t *ring;
    unsigned idx;
    uint32_t gpget, gpput, get, put;

    if (!c || !c->ready || !c->userd.phys || !c->gpfifo.va) {
        lx_printk("nouveau-lx: canal sin volcar (%s): no está listo\n",
                  why ? why : "?");
        return;
    }
    idx = (c->gpput ? c->gpput - 1u : 0u) % GSP_CHAN_GPFIFO_ENTRIES;
    ring = (const uint32_t *)((unsigned char *)c->gpfifo.va +
                              idx * NVC56F_GP_ENTRY__SIZE);
    gpget = chan_userd_rd32(c, USERD_OFF_GPGET);
    gpput = chan_userd_rd32(c, USERD_OFF_GPPUT);
    get = chan_userd_rd32(c, USERD_OFF_GET);
    put = chan_userd_rd32(c, USERD_OFF_PUT);

    /* En pre-Blackwell, USERD GPGet partía el problema en dos. En Blackwell el
     * writeback no existe: GPGet del USERD se queda a 0 siempre. Lo que cuenta
     * es gpget SW (ack tras semáforo) frente a gpput. */
    lx_printk("nouveau-lx: canal (%s): gpget=%u gpput=%u | USERD GPGet=%u "
              "GPPut=%u Get=%u Put=%u\n", why ? why : "?", c->gpget, c->gpput,
              gpget, gpput, get, put);
    lx_printk("nouveau-lx: canal (%s): entrada %u = %08x %08x → pushbuffer "
              "0x%llx, %u dwords\n", why ? why : "?", idx, ring[0], ring[1],
              (unsigned long long)(((uint64_t)(ring[1] & 0xffu) << 32) |
                                   (uint64_t)(ring[0] & 0xfffffffcu)),
              (ring[1] >> 10) & 0x1fffffu);
    lx_printk("nouveau-lx: canal (%s): doorbell RPC=0x%08x kick=0x%08x ok=%d, "
              "pushbuf_va=0x%llx notifier_va=0x%llx\n", why ? why : "?",
              c->doorbell_token, c->doorbell_kick, c->doorbell_ok,
              (unsigned long long)c->pushbuf_va,
              (unsigned long long)c->notifier_va);
    chan_dump_hw(c, why ? why : "?");
}

void gsp_chan_fini(struct gsp_chan *c)
{
    if (!c) {
        return;
    }
    if (c->ready && c->rm && c->rm->ready) {
        gsp_rm_free(c->rm, c->handle);
    }
    gsp_dma_free(&c->mthdbuf);
    gsp_dma_free(&c->notifier);
    gsp_dma_free(&c->pushbuf);
    if (!c->userd_vram) {
        gsp_dma_free(&c->userd);
    }
    gsp_dma_free(&c->gpfifo);
    memset(c, 0, sizeof(*c));
}
