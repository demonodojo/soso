/* G4e (1/2): canal GPFIFO. Ver gsp_chan.h. */
#include "gsp_chan.h"

void *memset(void *dst, int c, unsigned long n);
void *memcpy(void *dst, const void *src, unsigned long n);

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
    p->engineType = NV2080_ENGINE_TYPE_COPY0;
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

/* ANDAMIO DE BRING-UP (2026-07-28) — quitar en cuanto el canal arranque.
 *
 * RM contesta 0x3b (INVALID_PARAMETER) sin decir QUÉ parámetro le disgusta, y
 * cada hipótesis probada a razón de una por ciclo de hardware son cinco minutos
 * y un riesgo de cuelgue. Un RM_ALLOC, en cambio, son milisegundos: se prueban
 * todas las variantes dentro del MISMO arranque y el log dice cuál pasa.
 *
 * Las variantes no son adivinanzas al azar: cada una es una decisión concreta
 * donde upstream y nosotros divergimos, o donde upstream tiene dos ramas y hubo
 * que elegir una. Se paran en la primera que RM acepta.
 *
 * Si ninguna pasa, el dato TAMBIÉN sirve: significa que hay dos cosas mal a la
 * vez, y por eso las dos últimas son combinaciones. */
struct chan_variant {
    const char *name;
    void (*apply)(struct gsp_chan *c, NV_CHANNEL_ALLOC_PARAMS *p);
};

/* Handle del USERD en VRAM para la variante 3; 0 si no se pudo reservar. */
static uint64_t chan_probe_userd_vram;

static void var_base(struct gsp_chan *c, NV_CHANNEL_ALLOC_PARAMS *p)
{
    (void)c; (void)p;
}

/* Upstream elige entre priv y no-priv; nosotros pusimos priv (somos el kernel),
 * pero pedir ADMIN puede ser justo lo que RM no le concede a este cliente. */
static void var_unpriv(struct gsp_chan *c, NV_CHANNEL_ALLOC_PARAMS *p)
{
    (void)c;
    p->flags &= ~NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE;
    p->internalFlags &= ~0x3u;   /* PRIVILEGE_USER = 0 */
}

/* La desviación consciente: upstream pone el USERD en VRAM (apertura 2). Aquí
 * sólo se comprueba si RM lo EXIGE; si es que sí, la CPU se queda sin poder
 * escribir GPPut y habrá que ir al doorbell de usermode. */
static void var_userd_vram(struct gsp_chan *c, NV_CHANNEL_ALLOC_PARAMS *p)
{
    (void)c;
    if (!chan_probe_userd_vram) {
        return;
    }
    p->userdMem.base = chan_probe_userd_vram;
    p->userdMem.addressSpace = NV_ADDRESS_SPACE_FBMEM;
    p->userdMem.cacheAttrib = NV_CACHE_ATTR_CACHED;
}

/* ¿Y si el ring lo quiere por dirección física, y la incoherencia con las GP
 * entries es cosa nuestra y no de RM? */
static void var_gpfifo_phys(struct gsp_chan *c, NV_CHANNEL_ALLOC_PARAMS *p)
{
    p->gpFifoOffset = c->gpfifo.phys;
}

/* Entradas que caben en la página entera (4096/8), por si RM comprueba que el
 * número declarado cuadre con la región mapeada. */
static void var_entries_full(struct gsp_chan *c, NV_CHANNEL_ALLOC_PARAMS *p)
{
    (void)c;
    p->gpFifoEntries = GSP_CHAN_GPFIFO_SIZE / NVC56F_GP_ENTRY__SIZE;
}

/* Sin method buffer, por si para un canal de CE sobra y molesta. */
static void var_no_mthdbuf(struct gsp_chan *c, NV_CHANNEL_ALLOC_PARAMS *p)
{
    (void)c;
    memset(&p->mthdbufMem, 0, sizeof(p->mthdbufMem));
}

/* Control: el 6 de antes. Si ESTA pasara y las demás no, el 9 estaría mal. */
static void var_engine6(struct gsp_chan *c, NV_CHANNEL_ALLOC_PARAMS *p)
{
    (void)c;
    p->engineType = 6u;
}

static void var_unpriv_userd_vram(struct gsp_chan *c, NV_CHANNEL_ALLOC_PARAMS *p)
{
    var_unpriv(c, p);
    var_userd_vram(c, p);
}

static void var_unpriv_gpfifo_phys(struct gsp_chan *c, NV_CHANNEL_ALLOC_PARAMS *p)
{
    var_unpriv(c, p);
    var_gpfifo_phys(c, p);
}

static const struct chan_variant chan_variants[] = {
    { "base (priv, USERD sysmem, GPFIFO por VA, engine 9)", var_base },
    { "sin privilegio (flags bit5=0, internalFlags USER)",  var_unpriv },
    { "USERD en VRAM (apertura 2)",                          var_userd_vram },
    { "GPFIFO por dirección física",                         var_gpfifo_phys },
    { "gpFifoEntries = 512 (la página entera)",              var_entries_full },
    { "sin method buffer",                                   var_no_mthdbuf },
    { "engineType = 6 (el viejo, de control)",               var_engine6 },
    { "sin privilegio + USERD en VRAM",                      var_unpriv_userd_vram },
    { "sin privilegio + GPFIFO físico",                      var_unpriv_gpfifo_phys },
};

static int chan_alloc_probing(struct gsp_chan *c, struct gsp_vram *vram,
                              const NV_CHANNEL_ALLOC_PARAMS *base)
{
    unsigned i;

    chan_probe_userd_vram = gsp_vram_alloc(vram, GSP_CHAN_INST_SIZE,
                                           GSP_CHAN_INST_SIZE);
    if (!chan_probe_userd_vram) {
        lx_printk("nouveau-lx: sonda — sin VRAM para el USERD de prueba\n");
    }

    for (i = 0; i < sizeof(chan_variants) / sizeof(chan_variants[0]); i++) {
        NV_CHANNEL_ALLOC_PARAMS params = *base;
        uint32_t status = 0;

        chan_variants[i].apply(c, &params);
        lx_printk("nouveau-lx: sonda %u/%u — %s\n", i + 1,
                  (unsigned)(sizeof(chan_variants) / sizeof(chan_variants[0])),
                  chan_variants[i].name);
        if (gsp_rm_alloc(c->rm, c->rm->device, c->handle, AMPERE_CHANNEL_GPFIFO_A,
                         &params, (uint32_t)sizeof(params), &status) == 0) {
            lx_printk("nouveau-lx: sonda %u ACEPTADA — es la buena\n", i + 1);
            return 0;
        }
    }
    lx_printk("nouveau-lx: sonda — las %u variantes rechazadas; hay más de una "
              "cosa mal a la vez\n",
              (unsigned)(sizeof(chan_variants) / sizeof(chan_variants[0])));
    return -1;
}

int gsp_chan_init(struct gsp_rm *rm, struct gsp_vmm *vmm, struct gsp_vram *vram,
                  struct gsp_chan *c, uint32_t vaspace)
{
    NV_CHANNEL_ALLOC_PARAMS params;

    if (!rm || !vmm || !vram || !c || !rm->ready || !vmm->ready || !vram->ready) {
        return -1;
    }
    memset(c, 0, sizeof(*c));
    c->rm = rm;
    c->vmm = vmm;
    c->handle = NVKM_RM_CHAN(0);

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

    c->gpfifo_va = GSP_CHAN_VA_BASE;
    c->userd_va = GSP_CHAN_VA_BASE + 4096ull;
    c->pushbuf_va = GSP_CHAN_VA_BASE + 8192ull;
    c->notifier_va = GSP_CHAN_VA_BASE + 12288ull;

    if (chan_map_buf(c, &c->gpfifo, c->gpfifo_va) != 0 ||
        chan_map_buf(c, &c->userd, c->userd_va) != 0 ||
        chan_map_buf(c, &c->pushbuf, c->pushbuf_va) != 0 ||
        chan_map_buf(c, &c->notifier, c->notifier_va) != 0) {
        gsp_chan_fini(c);
        return -1;
    }

    c->userd_ctl = (Nvc56fControl *)c->userd.va;
    chan_fill_alloc(c, &params, vaspace);

    if (chan_alloc_probing(c, vram, &params) != 0) {
        lx_printk("nouveau-lx: RM_ALLOC canal GPFIFO falló\n");
        gsp_chan_fini(c);
        return -1;
    }

    c->ready = 1;
    lx_printk("nouveau-lx: canal GPFIFO listo handle=0x%08x gpfifo=0x%llx (VA) "
              "userd=0x%llx+0x%x inst=0x%llx (VRAM) mthdbuf=%u B\n",
              c->handle, (unsigned long long)c->gpfifo_va,
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
    idx = c->gpput % GSP_CHAN_GPFIFO_ENTRIES;
    ring = (unsigned char *)c->gpfifo.va + idx * NVC56F_GP_ENTRY__SIZE;

    e0 = NVC56F_GP_ENTRY0_FETCH_UNCONDITIONAL |
         (uint32_t)((gp_get >> 2) & 0x3fffffffu);
    e1 = (uint32_t)((gp_get >> 32) & 0xffu) |
         (NVC56F_GP_ENTRY1_LEVEL_MAIN << 9) |
         (((pb_len + 3u) / 4u) << 10) |
         (NVC56F_GP_ENTRY1_SYNC_PROCEED << 31);

    memcpy(ring, &e0, 4);
    memcpy(ring + 4, &e1, 4);

    c->gpput++;
    c->userd_ctl->GPPut = c->gpput;
    c->userd_ctl->Put = pb_off + pb_len;
    return 0;
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
