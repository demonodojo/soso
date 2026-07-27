/* G4e (1/2): canal GPFIFO. Ver gsp_chan.h. */
#include "gsp_chan.h"

void *memset(void *dst, int c, unsigned long n);
void *memcpy(void *dst, const void *src, unsigned long n);

static int chan_map_buf(struct gsp_chan *c, struct gsp_dma_buf *b, uint64_t va)
{
    return gsp_vmm_map(c->vmm, va, b->phys, b->size, GSP_VMM_SYSMEM);
}

static int chan_fill_alloc(struct gsp_chan *c, NV_CHANNEL_ALLOC_PARAMS *p,
                           uint32_t vaspace)
{
    unsigned long gpfifo_bytes = GSP_CHAN_GPFIFO_ENTRIES * NVC56F_GP_ENTRY__SIZE;

    memset(p, 0, sizeof(*p));
    p->gpFifoOffset = c->gpfifo.phys;
    p->gpFifoEntries = GSP_CHAN_GPFIFO_ENTRIES;
    /* Upstream pone CLIENT_MAP_FIFO explícitamente a FALSE: es para que RM mapee
     * el FIFO en espacio de usuario, y aquí no hay usuario que valga. */
    p->flags = NVOS04_FLAGS_CHANNEL_TYPE_PHYSICAL;
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
    p->userdMem.size = GSP_CHAN_USERD_SIZE;
    p->userdMem.addressSpace = NV_ADDRESS_SPACE_SYSMEM;
    p->userdMem.cacheAttrib = NV_CACHE_ATTR_DEFAULT;
    p->mthdbufMem.base = c->pushbuf.phys;
    p->mthdbufMem.size = GSP_CHAN_PB_SIZE;
    p->mthdbufMem.addressSpace = NV_ADDRESS_SPACE_SYSMEM;
    p->mthdbufMem.cacheAttrib = NV_CACHE_ATTR_DEFAULT;

    /* `errorNotifierMem` NO se rellena: upstream no lo toca y en su lugar dice
     * en `internalFlags` que no hay notificador. Rellenarlo mientras el flag
     * decía UNKNOWN era pedirle a RM dos cosas incompatibles. El búfer del
     * notificador sigue reservado y mapeado; lo usa el CE por su cuenta. */
    p->internalFlags = NV_KERNELCHANNEL_ALLOC_INTERNALFLAGS_PRIVILEGE_ADMIN |
                       NV_KERNELCHANNEL_ALLOC_INTERNALFLAGS_ERROR_NOTIFIER_TYPE_NONE |
                       NV_KERNELCHANNEL_ALLOC_INTERNALFLAGS_ECC_ERROR_NOTIFIER_TYPE_NONE;
    (void)gpfifo_bytes;
    return 0;
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

    if (gsp_dma_alloc(&c->gpfifo, GSP_CHAN_GPFIFO_SIZE, "GPFIFO") != 0 ||
        gsp_dma_alloc(&c->userd, GSP_CHAN_USERD_SIZE, "USERD") != 0 ||
        gsp_dma_alloc(&c->pushbuf, GSP_CHAN_PB_SIZE, "pushbuffer") != 0 ||
        gsp_dma_alloc(&c->notifier, GSP_CHAN_NOTIFIER_SIZE, "notifier CE") != 0) {
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

    if (gsp_rm_alloc(rm, rm->device, c->handle, AMPERE_CHANNEL_GPFIFO_A,
                     &params, (uint32_t)sizeof(params), NULL) != 0) {
        lx_printk("nouveau-lx: RM_ALLOC canal GPFIFO falló\n");
        gsp_chan_fini(c);
        return -1;
    }

    c->ready = 1;
    lx_printk("nouveau-lx: canal GPFIFO listo handle=0x%08x gpfifo=0x%llx userd=0x%llx "
              "inst=0x%llx (VRAM)\n",
              c->handle, (unsigned long long)c->gpfifo.phys,
              (unsigned long long)c->userd.phys, (unsigned long long)c->inst_addr);
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
    gsp_dma_free(&c->notifier);
    gsp_dma_free(&c->pushbuf);
    gsp_dma_free(&c->userd);
    gsp_dma_free(&c->gpfifo);
    memset(c, 0, sizeof(*c));
}
