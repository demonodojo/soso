/* H1: motor 3D en subcanal 0 — CLEAR_SURFACE en VRAM. */
#include "gsp_3d.h"
#include "nvrm_r570.h"

void *memset(void *dst, int c, unsigned long n);

/* Paridad con gsp_ce.c / gsp_compute.c. */
static void td_pb_write(struct gsp_chan *c, unsigned *pos, uint32_t v)
{
    uint32_t *p = (uint32_t *)((unsigned char *)c->pushbuf.va + *pos);

    *p = v;
    *pos += 4;
}

static void td_pb_method(struct gsp_chan *c, unsigned *pos, unsigned subc,
                      unsigned method, unsigned count)
{
    uint32_t hdr = (NVC56F_DMA_INCR_OPCODE_VALUE << 29) |
                   (subc << 13) |
                   (count << 16) |
                   ((method >> 2) & 0xfffu);

    td_pb_write(c, pos, hdr);
}

static void td_pb_immd(struct gsp_chan *c, unsigned *pos, unsigned subc,
                    unsigned method, uint32_t data)
{
    uint32_t hdr = (NVC56F_DMA_SEC_OP_IMMD_DATA_METHOD << 29) |
                   (subc << 13) |
                   ((data & 0x1fffu) << 16) |
                   ((method >> 2) & 0xfffu);

    td_pb_write(c, pos, hdr);
}

static void td_pb_set_object(struct gsp_chan *c, unsigned *pos, unsigned subc,
                          uint32_t oclass)
{
    td_pb_method(c, pos, subc, GSP_3D_SET_OBJECT, 1);
    td_pb_write(c, pos, oclass & 0xffffu);
}

/* Comprueba offsets frente a clc097.h en tiempo de compilación. */
typedef char gsp_3d_assert_clear_surface
    [(GSP_3D_CLEAR_SURFACE == 0x19d0u) ? 1 : -1];
typedef char gsp_3d_assert_set_ct[(GSP_3D_SET_CT_SELECT == 0x121cu) ? 1 : -1];

static int encode_clear_pb(struct gsp_chan *c, uint32_t threed_cls,
                           uint64_t rt_va, uint32_t w, uint32_t h,
                           uint32_t pitch, uint32_t rgba, uint64_t sem_va,
                           uint32_t sem_payload, unsigned *pb_off,
                           unsigned *pb_len)
{
    unsigned pos;
    unsigned start;
    unsigned j = 0;
    uint32_t mem;

    if (!c || !c->ready || !w || !h || !pitch) {
        return -1;
    }
    if (gsp_chan_pb_reserve(c, 512) < 0) {
        if (gsp_chan_pb_rewind(c) != 0 ||
            gsp_chan_pb_reserve(c, 512) < 0) {
            return -1;
        }
    }
    start = (unsigned)c->pb_pos;
    pos = start;

    td_pb_set_object(c, &pos, GSP_3D_SUBCHANNEL, threed_cls);
    td_pb_immd(c, &pos, 0u, NVC86F_WFI, 0u);

    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_COLOR_TARGET_A(j), 1);
    td_pb_write(c, &pos, (uint32_t)(rt_va >> 32) & 0xffu);
    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_COLOR_TARGET_B(j), 1);
    td_pb_write(c, &pos, (uint32_t)rt_va);
    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_COLOR_TARGET_WIDTH(j), 1);
    td_pb_write(c, &pos, w);
    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_COLOR_TARGET_HEIGHT(j), 1);
    td_pb_write(c, &pos, h);
    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_COLOR_TARGET_FORMAT(j), 1);
    td_pb_write(c, &pos, GSP_3D_FMT_A8B8G8R8);

    mem = GSP_3D_CT_MEM_LAYOUT_PITCH;
    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_COLOR_TARGET_MEMORY(j), 1);
    td_pb_write(c, &pos, mem);

    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_COLOR_TARGET_THIRD_DIM(j), 1);
    td_pb_write(c, &pos, 1u);
    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_COLOR_TARGET_ARRAY_PITCH(j), 1);
    td_pb_write(c, &pos, pitch);

    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_SURFACE_CLIP_HORIZONTAL, 1);
    td_pb_write(c, &pos, w << 16);
    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_SURFACE_CLIP_VERTICAL, 1);
    td_pb_write(c, &pos, h << 16);

    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_CT_SELECT, 1);
    td_pb_write(c, &pos, GSP_3D_CT_SELECT_ONE);

    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_COLOR_CLEAR_VALUE(0), 4);
    td_pb_write(c, &pos, rgba);
    td_pb_write(c, &pos, rgba);
    td_pb_write(c, &pos, rgba);
    td_pb_write(c, &pos, rgba);

    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_REPORT_SEMAPHORE_A, 1);
    td_pb_write(c, &pos, (uint32_t)(sem_va >> 32));
    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_REPORT_SEMAPHORE_B, 1);
    td_pb_write(c, &pos, (uint32_t)sem_va);
    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_REPORT_SEMAPHORE_C, 1);
    td_pb_write(c, &pos, sem_payload);
    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_SET_REPORT_SEMAPHORE_D, 1);
    td_pb_write(c, &pos, GSP_3D_REPORT_RELEASE_WRITES);

    td_pb_method(c, &pos, GSP_3D_SUBCHANNEL, GSP_3D_CLEAR_SURFACE, 1);
    td_pb_write(c, &pos, GSP_3D_CLEAR_RGBA);

    if (pb_off) {
        *pb_off = start;
    }
    if (pb_len) {
        *pb_len = pos - start;
    }
    return 0;
}

int gsp_3d_selftest_encode(struct gsp_chan *c, uint32_t threed_cls,
                           uint64_t rt_va, uint32_t w, uint32_t h,
                           uint32_t pitch, uint32_t rgba, uint64_t sem_va,
                           uint32_t sem_payload)
{
    unsigned pb_off = 0, pb_len = 0;
    const uint32_t *words;
    uint32_t hdr;

    if (encode_clear_pb(c, threed_cls, rt_va, w, h, pitch, rgba, sem_va,
                        sem_payload, &pb_off, &pb_len) != 0) {
        return -1;
    }
    if (pb_len < 8) {
        return -1;
    }
    words = (const uint32_t *)((const unsigned char *)c->pushbuf.va + pb_off);
    hdr = words[0];
    if (((hdr >> 29) & 7u) != NVC56F_DMA_INCR_OPCODE_VALUE) {
        return -1;
    }
    if (((hdr >> 13) & 7u) != GSP_3D_SUBCHANNEL) {
        return -1;
    }
    if ((words[1] & 0xffffu) != (threed_cls & 0xffffu)) {
        return -1;
    }
    return 0;
}

static int gsp_3d_wait(struct gsp_3d *td, unsigned ms)
{
    const volatile uint32_t *sem;
    unsigned waited;

    if (!td || !td->ready || !td->chan || !td->chan->notifier.va) {
        return -1;
    }
    sem = (const volatile uint32_t *)td->chan->notifier.va;
    for (waited = 0; waited <= ms; waited++) {
        __asm__ __volatile__("mfence" ::: "memory");
        if ((int32_t)(*sem - td->seq) >= 0) {
            gsp_chan_ack_progress(td->chan);
            return 0;
        }
        lx_mdelay(1);
    }
    lx_printk("nouveau-lx: 3D — semáforo no llegó a %u en %u ms (vale %u)\n",
              td->seq, ms, *sem);
    gsp_chan_dump(td->chan, "3D sin señalizar");
    return -1;
}

int gsp_3d_init(struct gsp_rm *rm, struct gsp_chan *chan, struct gsp_3d *td)
{
    uint32_t rm_status = 0;
    static const uint32_t cand[] = {
        BLACKWELL_3D_B, BLACKWELL_3D_A, HOPPER_3D_A, ADA_3D_A,
        AMPERE_B, AMPERE_A,
    };

    if (!rm || !chan || !td || !rm->ready || !chan->ready) {
        return -1;
    }
    memset(td, 0, sizeof(*td));
    td->rm = rm;
    td->chan = chan;
    td->handle = NVKM_RM_THREED0;
    td->cls = gsp_rm_class_pick("3D", cand,
                                (unsigned)(sizeof(cand) / sizeof(cand[0])));
    if (gsp_rm_alloc(rm, chan->handle, td->handle, td->cls, NULL, 0,
                     &rm_status) != 0) {
        lx_printk("nouveau-lx: RM_ALLOC 3D cls=0x%04x rechazado (0x%x)\n",
                  td->cls, rm_status);
        return -1;
    }
    td->ready = 1;
    lx_printk("nouveau-lx: 3D listo cls=0x%04x handle=0x%08x\n", td->cls,
              td->handle);
    return 0;
}

void gsp_3d_fini(struct gsp_3d *td)
{
    if (!td || !td->ready || !td->rm) {
        return;
    }
    gsp_rm_free(td->rm, td->handle);
    td->ready = 0;
}

int gsp_3d_clear(struct gsp_3d *td, uint64_t rt_va, uint32_t width,
                 uint32_t height, uint32_t pitch, uint32_t rgba)
{
    unsigned pb_off = 0, pb_len = 0;
    uint64_t sem_va;

    if (!td || !td->ready || !td->chan) {
        return -1;
    }
    sem_va = td->chan->notifier_va;
    td->seq++;
    if (encode_clear_pb(td->chan, td->cls, rt_va, width, height, pitch, rgba,
                        sem_va, td->seq, &pb_off, &pb_len) != 0) {
        return -1;
    }
    if (gsp_chan_submit(td->chan, pb_off, pb_len) != 0) {
        return -1;
    }
    return gsp_3d_wait(td, GSP_3D_WAIT_MS);
}
