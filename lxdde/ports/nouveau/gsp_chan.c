/* G4e (1/2): canal GPFIFO. Ver gsp_chan.h. */
#include "gsp_chan.h"
#include "gsp_mmio.h"

void *memset(void *dst, int c, unsigned long n);
void *memcpy(void *dst, const void *src, unsigned long n);

/* Mismo patrón que `gsp_rpc_barrier`: lo que se publica en sysmem tiene que
 * haber salido del core antes de tocar el registro que se lo dice a la GPU. */
static void gsp_chan_barrier(void)
{
    __asm__ __volatile__("mfence" ::: "memory");
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
     * hay que poner son los dos subcampos que upstream no deja a cero: canal
     * privilegiado (emparejado con el ADMIN de `internalFlags`, ver abajo) y
     * USERD_INDEX_PAGE_FIXED. Con chid=0 los índices valen 0, pero el bit de
     * "fijo" va igual: es lo que le dice a RM que no elija él. */
    p->flags = NVOS04_FLAGS_CHANNEL_TYPE_PHYSICAL |
               NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE |
               NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE(0) |
               NVOS04_FLAGS_CHANNEL_USERD_INDEX_PAGE_VALUE(0) |
               NVOS04_FLAGS_CHANNEL_USERD_INDEX_PAGE_FIXED_TRUE;
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
    p->instanceMem.cacheAttrib = NV_CACHE_ATTR_CACHED;
    p->ramfcMem.base = c->inst_addr;
    p->ramfcMem.size = GSP_CHAN_RAMFC_SIZE;
    p->ramfcMem.addressSpace = NV_ADDRESS_SPACE_FBMEM;
    p->ramfcMem.cacheAttrib = NV_CACHE_ATTR_CACHED;

    /* Desviación deliberada de upstream: allí el USERD vive en VRAM
     * (addressSpace=2), pero sin BAR1 la CPU no tiene ventana a la VRAM y
     * `userd_ctl` se lee y escribe desde aquí en cada submit. Se queda en
     * sysmem, que RM admite, pero con la apertura BIEN puesta. */
    p->userdMem.base = c->userd.phys;
    /* El tamaño es el del USERD del chip (0x200), no el de la página que lo
     * aloja: la reserva sigue siendo de 4 KiB por granularidad de DMA. */
    p->userdMem.size = GSP_CHAN_USERD_HW_SIZE;
    p->userdMem.addressSpace = NV_ADDRESS_SPACE_SYSMEM;
    p->userdMem.cacheAttrib = NV_CACHE_ATTR_DEFAULT;
    /* Búfer propio y del tamaño que dijo RM. Antes iba aquí el pushbuffer con
     * 4096 a ojo: dos errores a la vez, el búfer y el número. */
    p->mthdbufMem.base = c->mthdbuf.phys;
    p->mthdbufMem.size = c->mthdbuf_size;
    p->mthdbufMem.addressSpace = NV_ADDRESS_SPACE_SYSMEM;
    p->mthdbufMem.cacheAttrib = NV_CACHE_ATTR_DEFAULT;

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
    c->doorbell_ok = 1;
    /* El token de upstream es `(runl->doorbell << 16) | chid`. El chid lo asigna
     * RM, no nosotros: con un solo canal salía 0, y con el de GR0 detrás ya no
     * tiene por qué. Se imprime troceado para poder ver DOS tokens distintos —dos
     * canales que reciben el mismo token es un bug que de otro modo se manifiesta
     * como "el otro canal no arranca". */
    lx_printk("nouveau-lx: doorbell token=0x%08x (runlist=%u chid=%u) motor=%u\n",
              c->doorbell_token, c->doorbell_token >> 16,
              c->doorbell_token & 0xffffu, c->engine);
    return 0;
}

static int chan_start(struct gsp_chan *c)
{
    return (chan_bind_engine(c) != 0 || chan_schedule(c) != 0 ||
            chan_get_doorbell_token(c) != 0) ? -1 : 0;
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
        gsp_dma_alloc(&c->userd, GSP_CHAN_USERD_SIZE, "USERD") != 0 ||
        gsp_dma_alloc(&c->pushbuf, GSP_CHAN_PB_SIZE, "pushbuffer") != 0 ||
        gsp_dma_alloc(&c->notifier, GSP_CHAN_NOTIFIER_SIZE, "notifier CE") != 0 ||
        gsp_dma_alloc(&c->mthdbuf, c->mthdbuf_size, "method buffer CE") != 0) {
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
    c->gpfifo_va = va_base;
    c->userd_va = va_base + 4096ull;
    c->pushbuf_va = va_base + 8192ull;
    c->notifier_va = va_base + 12288ull;

    if (chan_map_buf(c, &c->gpfifo, c->gpfifo_va) != 0 ||
        chan_map_buf(c, &c->userd, c->userd_va) != 0 ||
        chan_map_buf(c, &c->pushbuf, c->pushbuf_va) != 0 ||
        chan_map_buf(c, &c->notifier, c->notifier_va) != 0) {
        gsp_chan_fini(c);
        return -1;
    }

    c->userd_ctl = (Nvc56fControl *)c->userd.va;
    chan_fill_alloc(c, &params, vaspace);

    if (gsp_rm_alloc(c->rm, c->rm->device, c->handle, c->cls,
                     &params, (uint32_t)sizeof(params), 0) != 0) {
        lx_printk("nouveau-lx: RM_ALLOC canal GPFIFO falló\n");
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

/* G5 lanza un QMD por tanda de filas y cada QMD inline ocupa ~900 B: en un
 * pushbuffer de 4 KiB caben cuatro y a la quinta `pb_reserve` falla. Rebobinar es
 * lo que permite que el bucle de tandas sea tan largo como haga falta, pero solo
 * cuando el host ya ha consumido lo anterior: `GPGet` es lo que dice eso, y sin
 * mirarlo se estarían pisando métodos que la GPU aún no ha leído. */
int gsp_chan_pb_rewind(struct gsp_chan *c)
{
    if (!c || !c->ready || !c->userd_ctl) {
        return -1;
    }
    if (c->pb_pos == 0) {
        return 0;
    }
    /* Lectura `volatile`, como en `gsp_chan_dump`: GPGet lo escribe el host por
     * DMA y quien lo lea con un acceso normal se puede quedar con el valor que el
     * compilador tenga a mano — y entonces esto rebobinaría sobre trabajo vivo,
     * que es justo lo que viene a impedir. */
    gsp_chan_barrier();
    if (((const volatile Nvc56fControl *)c->userd_ctl)->GPGet != c->gpput) {
        lx_printk("nouveau-lx: pushbuffer sin rebobinar: GPGet=%u GPPut=%u "
                  "(el host aún no lo consumió)\n",
                  ((const volatile Nvc56fControl *)c->userd_ctl)->GPGet,
                  c->gpput);
        return -1;
    }
    c->pb_pos = 0;
    return 0;
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
    c->userd_ctl->GPPut = c->gpput;
    c->userd_ctl->Put = pb_off + pb_len;

    /* Y el doorbell, que es lo que faltaba: en Volta+ escribir GPPut en el USERD
     * no patea nada por sí solo —el host no sondea el USERD—, hay que avisarle
     * por el registro de usermode. Sin esto el canal se queda con el trabajo
     * encolado y el semáforo a 0 para siempre, que es exactamente lo que hacía
     * (2026-07-28).
     *
     * El USERD está en sysmem no cacheada y el ring también, así que el orden lo
     * garantiza el mapeo; lo que hay que asegurar es que las escrituras de arriba
     * hayan SALIDO del core antes del kick, porque si la GPU lee el ring antes de
     * ver el GPPut se encuentra una entrada a medio poner. */
    gsp_chan_barrier();
    if (c->doorbell_ok) {
        gsp_mmio_wr32(NV_VFN_DOORBELL, c->doorbell_token);
    }
    return 0;
}

void gsp_chan_dump(struct gsp_chan *c, const char *why)
{
    const volatile Nvc56fControl *u;
    const uint32_t *ring;
    unsigned idx;

    if (!c || !c->ready || !c->userd_ctl || !c->gpfifo.va) {
        lx_printk("nouveau-lx: canal sin volcar (%s): no está listo\n",
                  why ? why : "?");
        return;
    }
    u = (const volatile Nvc56fControl *)c->userd_ctl;
    idx = (c->gpput ? c->gpput - 1u : 0u) % GSP_CHAN_GPFIFO_ENTRIES;
    ring = (const uint32_t *)((unsigned char *)c->gpfifo.va +
                              idx * NVC56F_GP_ENTRY__SIZE);

    /* **GPGet es la pregunta.** Lo escribe el host cuando consume una entrada del
     * GPFIFO, así que parte el problema en dos mitades que no se solapan:
     *
     *   GPGet == GPPut  → el host SÍ recogió el trabajo y ejecutó el pushbuffer.
     *                     Entonces lo que falla es de dentro: los métodos del CE,
     *                     la VA del semáforo o su apertura.
     *   GPGet == 0      → el host no llegó a mirar. El problema está antes: el
     *                     doorbell no surte efecto, el canal no corre de verdad,
     *                     o el USERD que RM programó no es este.
     *
     * Sin este número las dos mitades se ven igual desde fuera —semáforo a 0— y
     * cualquier arreglo es a ciegas. */
    lx_printk("nouveau-lx: canal (%s): GPGet=%u GPPut=%u (nuestro gpput=%u) "
              "Get=%u Put=%u\n", why ? why : "?", u->GPGet, u->GPPut, c->gpput,
              u->Get, u->Put);
    lx_printk("nouveau-lx: canal (%s): entrada %u = %08x %08x → pushbuffer "
              "0x%llx, %u dwords\n", why ? why : "?", idx, ring[0], ring[1],
              (unsigned long long)(((uint64_t)(ring[1] & 0xffu) << 32) |
                                   (uint64_t)(ring[0] & 0xfffffffcu)),
              (ring[1] >> 10) & 0x1fffffu);
    lx_printk("nouveau-lx: canal (%s): doorbell token=0x%08x ok=%d, "
              "pushbuf_va=0x%llx notifier_va=0x%llx\n", why ? why : "?",
              c->doorbell_token, c->doorbell_ok,
              (unsigned long long)c->pushbuf_va,
              (unsigned long long)c->notifier_va);
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
    gsp_dma_free(&c->userd);
    gsp_dma_free(&c->gpfifo);
    memset(c, 0, sizeof(*c));
}
