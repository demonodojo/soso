/* G4e (2/2): motor de copia CE. Ver gsp_ce.h. */
#include "gsp_ce.h"

void *memset(void *dst, int c, unsigned long n);

/* Mapa de subcanales de NVIDIA (`cla06fsubch.h` / `push906f.h` de nouveau):
 * 3D=0, COMPUTE=1, I2M=2, 2D=3, COPY_ENGINE=4. */
#define GSP_CE_SUBCHANNEL 4u

static void pb_write(struct gsp_chan *c, unsigned *pos, uint32_t v)
{
    uint32_t *p = (uint32_t *)((unsigned char *)c->pushbuf.va + *pos);

    *p = v;
    *pos += 4;
}

static void pb_method(struct gsp_chan *c, unsigned *pos, unsigned subc,
                      unsigned method, unsigned count)
{
    uint32_t hdr = (NVC56F_DMA_INCR_OPCODE_VALUE << 29) |
                   (subc << 13) |
                   (count << 16) |
                   ((method >> 2) & 0xfffu);

    pb_write(c, pos, hdr);
}

/* `SET_OBJECT` en Volta+ lleva la **clase** del motor (bits 15:0), no el handle de
 * RM. nouveau: `PUSH_NVSQ(push, NVA0B5, 0x0000, handle & 0x0000ffff)` con handle
 * = `oclass | engine<<16` → el dato es 0xcab5, no 0xc6b50000. UVM hace lo mismo
 * en el canal CE. Además los métodos del copy engine van por el subcanal 4
 * (`PUSH906F_SUBC_NVA0B5`), no por el 0 (= 3D/GR). Mandar el handle entero por
 * subcanal 0 dejó clase 0x0000 atada a GR → PBDMA_HANG_DURING_HTE (2026-07-29). */
static void pb_set_object(struct gsp_chan *c, unsigned *pos, unsigned subc,
                          uint32_t oclass)
{
    pb_method(c, pos, subc, NVC56F_SET_OBJECT, 1);
    pb_write(c, pos, oclass);
}

int gsp_ce_init(struct gsp_rm *rm, struct gsp_chan *chan, struct gsp_ce *ce)
{
    if (!rm || !chan || !ce || !rm->ready || !chan->ready) {
        return -1;
    }
    memset(ce, 0, sizeof(*ce));
    ce->rm = rm;
    ce->chan = chan;
    ce->handle = NVKM_RM_CE0;
    {
        /* Misma regla que el canal: la clase la dice el catálogo del chip. Para
         * GB20x es `BLACKWELL_DMA_COPY_B` (`rm/gb20x.c`), no la de Ampere que
         * pedíamos aquí. Los métodos `NVC6B5_*` del pushbuffer no cambian entre
         * estas clases —el encoding de la copia lineal es el mismo—, así que
         * sólo cambia el número que va en el ALLOC. */
        static const uint32_t cand[] = {
            BLACKWELL_DMA_COPY_B, BLACKWELL_DMA_COPY_A, HOPPER_DMA_COPY_A,
            AMPERE_DMA_COPY_B, AMPERE_DMA_COPY_A,
        };

        ce->cls = gsp_rm_class_pick("CE (DMA copy)", cand,
                                    (unsigned)(sizeof(cand) / sizeof(cand[0])));
    }

    if (gsp_rm_alloc(rm, chan->handle, ce->handle, ce->cls,
                     NULL, 0, NULL) != 0) {
        lx_printk("nouveau-lx: RM_ALLOC CE falló (cls=0x%04x)\n", ce->cls);
        return -1;
    }

    ce->ready = 1;
    lx_printk("nouveau-lx: CE listo cls=0x%04x handle=0x%08x\n",
              ce->cls, ce->handle);
    return 0;
}

int gsp_ce_encode_copy(struct gsp_ce *ce, uint64_t dst_va, uint64_t src_va,
                       uint32_t size, unsigned *pb_off, unsigned *pb_len)
{
    struct gsp_chan *c;
    unsigned pos;
    unsigned start;
    unsigned end;
    uint32_t launch;
    uint64_t sem_va;
    int off;

    if (!ce || !ce->ready || !ce->chan || !size) {
        return -1;
    }
    c = ce->chan;

    off = gsp_chan_pb_reserve(c, 160);
    if (off < 0) {
        /* ~25 reservas de 160 B llenan el PB; tras el wait CE ya hubo ack del
         * progreso SW y rebobinar es seguro (Blackwell no escribe USERD GPGet). */
        if (gsp_chan_pb_rewind(c) != 0 ||
            (off = gsp_chan_pb_reserve(c, 160)) < 0) {
            lx_printk("nouveau-lx: CE — pushbuffer lleno y sin rebobinar\n");
            return -1;
        }
    }
    start = (unsigned)off;
    pos = start;

    pb_set_object(c, &pos, GSP_CE_SUBCHANNEL, ce->cls);

    pb_method(c, &pos, GSP_CE_SUBCHANNEL, NVC6B5_OFFSET_IN_UPPER, 1);
    pb_write(c, &pos, (uint32_t)(src_va >> 32));
    pb_method(c, &pos, GSP_CE_SUBCHANNEL, NVC6B5_OFFSET_IN_LOWER, 1);
    pb_write(c, &pos, (uint32_t)src_va);
    pb_method(c, &pos, GSP_CE_SUBCHANNEL, NVC6B5_PITCH_IN, 1);
    pb_write(c, &pos, size);

    pb_method(c, &pos, GSP_CE_SUBCHANNEL, NVC6B5_OFFSET_OUT_UPPER, 1);
    pb_write(c, &pos, (uint32_t)(dst_va >> 32));
    pb_method(c, &pos, GSP_CE_SUBCHANNEL, NVC6B5_OFFSET_OUT_LOWER, 1);
    pb_write(c, &pos, (uint32_t)dst_va);
    pb_method(c, &pos, GSP_CE_SUBCHANNEL, NVC6B5_PITCH_OUT, 1);
    pb_write(c, &pos, size);

    pb_method(c, &pos, GSP_CE_SUBCHANNEL, NVC6B5_LINE_LENGTH_IN, 1);
    pb_write(c, &pos, size);
    pb_method(c, &pos, GSP_CE_SUBCHANNEL, NVC6B5_LINE_COUNT, 1);
    pb_write(c, &pos, 1);

    sem_va = c->notifier_va;
    ce->pending = ++ce->seq;
    pb_method(c, &pos, GSP_CE_SUBCHANNEL, NVC6B5_SET_SEMAPHORE_A, 1);
    pb_write(c, &pos, (uint32_t)(sem_va >> 32));
    pb_method(c, &pos, GSP_CE_SUBCHANNEL, NVC6B5_SET_SEMAPHORE_B, 1);
    pb_write(c, &pos, (uint32_t)sem_va);
    pb_method(c, &pos, GSP_CE_SUBCHANNEL, NVC6B5_SET_SEMAPHORE_PAYLOAD, 1);
    pb_write(c, &pos, ce->pending);

    launch = NVC6B5_LAUNCH_DMA_DATA_TRANSFER_TYPE_NON_PIPELINED |
             NVC6B5_LAUNCH_DMA_FLUSH_ENABLE_TRUE |
             NVC6B5_LAUNCH_DMA_SRC_TYPE_VIRTUAL |
             NVC6B5_LAUNCH_DMA_DST_TYPE_VIRTUAL |
             NVC6B5_LAUNCH_DMA_SRC_MEMORY_LAYOUT_PITCH |
             NVC6B5_LAUNCH_DMA_DST_MEMORY_LAYOUT_PITCH |
             NVC6B5_LAUNCH_DMA_SEMAPHORE_TYPE_RELEASE_ONE_WORD;

    pb_method(c, &pos, GSP_CE_SUBCHANNEL, NVC6B5_LAUNCH_DMA, 1);
    pb_write(c, &pos, launch);

    end = pos;
    if (pb_off) {
        *pb_off = start;
    }
    if (pb_len) {
        *pb_len = end - start;
    }
    return 0;
}

int gsp_ce_wait(struct gsp_ce *ce, unsigned ms)
{
    const volatile uint32_t *sem;
    unsigned waited;

    if (!ce || !ce->ready || !ce->chan || !ce->chan->notifier.va) {
        return -1;
    }
    sem = (const volatile uint32_t *)ce->chan->notifier.va;

    for (waited = 0; waited <= ms; waited++) {
        __asm__ __volatile__("mfence" ::: "memory");
        /* Comparación con resta: el payload es monótono y así un envoltorio del
         * contador de 32 bits no deja la espera colgada para siempre. */
        if ((int32_t)(*sem - ce->pending) >= 0) {
            gsp_chan_ack_progress(ce->chan);
            return 0;
        }
        lx_mdelay(1);
    }
    lx_printk("nouveau-lx: CE — semáforo no llegó a %u en %u ms (vale %u)\n",
              ce->pending, ms, *sem);
    gsp_chan_dump(ce->chan, "CE sin señalizar");
    return -1;
}

int gsp_ce_copy_sync(struct gsp_ce *ce, uint64_t dst_va, uint64_t src_va,
                     uint32_t size, unsigned ms)
{
    unsigned pb_off = 0, pb_len = 0;

    if (gsp_ce_encode_copy(ce, dst_va, src_va, size, &pb_off, &pb_len) != 0 ||
        gsp_chan_submit(ce->chan, pb_off, pb_len) != 0) {
        return -1;
    }
    return gsp_ce_wait(ce, ms);
}

int gsp_ce_selftest(struct gsp_ce *ce, uint64_t scratch_va, uint64_t vram_va,
                    void *scratch_cpu, uint32_t size)
{
    unsigned i;
    uint32_t *pat;

    if (!ce || !ce->ready || !scratch_cpu || size < 64 || size > 4096) {
        return -1;
    }

    pat = (uint32_t *)scratch_cpu;
    for (i = 0; i < size / 4; i++) {
        pat[i] = 0xa5c3e1b4u ^ (uint32_t)i;
    }
    memset(ce->chan->notifier.va, 0, ce->chan->notifier.size);
    ce->seq = 0;

    if (gsp_ce_copy_sync(ce, vram_va, scratch_va, size, GSP_CE_WAIT_MS) != 0) {
        lx_printk("nouveau-lx: CE selftest — la copia sysmem → VRAM no señalizó\n");
        return -1;
    }

    /* Borrar el origen antes de la vuelta: si no, un readback que no hiciera
     * nada dejaría el patrón intacto y la comparación pasaría igual. Este
     * memset es lo que convierte la prueba en falsificable. */
    memset(scratch_cpu, 0, size);
    __asm__ __volatile__("mfence" ::: "memory");

    if (gsp_ce_copy_sync(ce, scratch_va, vram_va, size, GSP_CE_WAIT_MS) != 0) {
        lx_printk("nouveau-lx: CE selftest — la copia VRAM → sysmem no señalizó\n");
        return -1;
    }

    for (i = 0; i < size / 4; i++) {
        if (pat[i] != (0xa5c3e1b4u ^ (uint32_t)i)) {
            lx_printk("nouveau-lx: CE selftest — dword %u vale 0x%08x, esperaba 0x%08x\n",
                      i, pat[i], 0xa5c3e1b4u ^ (uint32_t)i);
            return -1;
        }
    }

    lx_printk("nouveau-lx: CE selftest OK — %u B ida y vuelta por VRAM\n", size);
    return 0;
}

void gsp_ce_fini(struct gsp_ce *ce)
{
    if (!ce) {
        return;
    }
    if (ce->ready && ce->rm && ce->rm->ready) {
        gsp_rm_free(ce->rm, ce->handle);
    }
    memset(ce, 0, sizeof(*ce));
}
