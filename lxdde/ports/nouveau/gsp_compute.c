/* G4f: objeto compute Blackwell + QMD inline. Ver gsp_compute.h. */
#include "gsp_compute.h"

void *memset(void *dst, int c, unsigned long n);
void *memcpy(void *dst, const void *src, unsigned long n);

static void cp_pb_write(struct gsp_chan *c, unsigned *pos, uint32_t v)
{
    uint32_t *p = (uint32_t *)((unsigned char *)c->pushbuf.va + *pos);

    *p = v;
    *pos += 4;
}

static void cp_pb_method(struct gsp_chan *c, unsigned *pos, unsigned subc,
                      unsigned method, unsigned count)
{
    uint32_t hdr = (NVC56F_DMA_INCR_OPCODE_VALUE << 29) |
                   (subc << 13) |
                   (count << 16) |
                   ((method >> 2) & 0xfffu);

    cp_pb_write(c, pos, hdr);
}

static void cp_pb_immed(struct gsp_chan *c, unsigned *pos, unsigned subc,
                     unsigned method, uint32_t data)
{
    uint32_t hdr = (NVC56F_DMA_SEC_OP_IMMD_DATA_METHOD << 29) |
                   (subc << 13) |
                   ((data & 0x1fffu) << 16) |
                   ((method >> 2) & 0xfffu);

    cp_pb_write(c, pos, hdr);
}

static void qmd_set_bits(uint32_t *qmd, unsigned lo, unsigned hi, uint32_t val)
{
    unsigned w0 = lo / 32u;
    unsigned b0 = lo % 32u;
    unsigned w1 = hi / 32u;
    unsigned i;

    if (w0 == w1) {
        unsigned width = hi - lo + 1u;
        uint32_t mask = width >= 32 ? 0xffffffffu : ((1u << width) - 1u);

        qmd[w0] = (qmd[w0] & ~(mask << b0)) | ((val & mask) << b0);
        return;
    }
    for (i = lo; i <= hi; i++) {
        unsigned bit = i - lo;
        uint32_t bitval = (val >> bit) & 1u;
        unsigned w = i / 32u;
        unsigned b = i % 32u;

        qmd[w] = (qmd[w] & ~(1u << b)) | (bitval << b);
    }
}

int gsp_compute_init(struct gsp_rm *rm, struct gsp_chan *chan,
                     struct gsp_compute *cp)
{
    if (!rm || !chan || !cp || !rm->ready || !chan->ready) {
        return -1;
    }
    memset(cp, 0, sizeof(*cp));
    cp->rm = rm;
    cp->chan = chan;
    cp->handle = NVKM_RM_COMPUTE0;
    cp->sass_va = G4F_SASS_VA;
    cp->sass_size = gsp_saxpy_sass_len;

    if (gsp_rm_alloc(rm, chan->handle, cp->handle, BLACKWELL_COMPUTE_A,
                     NULL, 0, NULL) != 0) {
        lx_printk("nouveau-lx: RM_ALLOC compute falló\n");
        return -1;
    }
    cp->ready = 1;
    lx_printk("nouveau-lx: compute listo handle=0x%08x sass=%u B\n",
              cp->handle, cp->sass_size);
    return 0;
}

int gsp_compute_stage_sass(struct gsp_compute *cp, struct gsp_ce *ce,
                           uint64_t scratch_va, void *scratch_cpu)
{
    unsigned pb_off = 0, pb_len = 0;

    if (!cp || !cp->ready || !ce || !scratch_cpu || !cp->sass_size) {
        return -1;
    }
    if (cp->sass_size > 4096) {
        return -1;
    }
    memcpy(scratch_cpu, gsp_saxpy_sass, cp->sass_size);
    if (gsp_ce_encode_copy(ce, cp->sass_va, scratch_va, cp->sass_size,
                           &pb_off, &pb_len) != 0 ||
        gsp_chan_submit(cp->chan, pb_off, pb_len) != 0) {
        return -1;
    }
    return 0;
}

void gsp_compute_fill_saxpy_qmd(GspQmdV05 *qmd, uint64_t prog_va, unsigned grid_x)
{
    uint64_t prog_shift = prog_va >> 4;

    memset(qmd, 0, sizeof(*qmd));
    qmd_set_bits(qmd->words, 151, 153, NVCDC0_QMDV05_00_QMD_TYPE_GRID_CTA);
    qmd_set_bits(qmd->words, 1248, 1279, grid_x ? grid_x : 1u);
    qmd_set_bits(qmd->words, 1280, 1295, 1u);
    qmd_set_bits(qmd->words, 1312, 1327, 1u);
    qmd_set_bits(qmd->words, 1088, 1103, 256u);
    qmd_set_bits(qmd->words, 1104, 1119, 1u);
    qmd_set_bits(qmd->words, 1120, 1127, 1u);
    qmd_set_bits(qmd->words, 1024, 1055, (uint32_t)(prog_shift & 0xffffffffu));
    qmd_set_bits(qmd->words, 1056, 1076, (uint32_t)((prog_shift >> 32) & 0x1ffffu));
}

int gsp_compute_encode_qmd(struct gsp_compute *cp, const GspQmdV05 *qmd,
                           unsigned *pb_off, unsigned *pb_len)
{
    struct gsp_chan *c;
    unsigned pos;
    unsigned start;
    unsigned i;
    uint64_t qmd_va = G4F_PARAM_VA;

    if (!cp || !cp->ready || !qmd) {
        return -1;
    }
    c = cp->chan;
    start = (unsigned)c->pb_pos;
    if (gsp_chan_pb_reserve(c, 512 + GSP_QMD_INLINE_WORDS * 4) < 0) {
        return -1;
    }
    pos = start;

    cp_pb_immed(c, &pos, 0, NVCDC0_SET_OBJECT, cp->handle);
    cp_pb_method(c, &pos, 0, NVCDC0_SET_QMD_VERSION, 1);
    cp_pb_write(c, &pos, (GSP_QMD_VERSION_CURRENT) |
                      (GSP_QMD_VERSION_CURRENT << 16));
    cp_pb_method(c, &pos, 0, NVCDC0_SET_INLINE_QMD_ADDRESS_A, 1);
    cp_pb_write(c, &pos, NVCDC0_SET_INLINE_QMD_ADDRESS_A_INLINE_SIZE_INLINE_384 |
                      (uint32_t)((qmd_va >> 32) & 0x1ffu));
    cp_pb_method(c, &pos, 0, NVCDC0_SET_INLINE_QMD_ADDRESS_B, 1);
    cp_pb_write(c, &pos, (uint32_t)(qmd_va >> 8));

    for (i = 0; i < GSP_QMD_INLINE_WORDS; i++) {
        cp_pb_method(c, &pos, 0, NVCDC0_LOAD_INLINE_QMD_DATA(i), 1);
        cp_pb_write(c, &pos, qmd->words[i]);
    }

    if (pb_off) {
        *pb_off = start;
    }
    if (pb_len) {
        *pb_len = pos - start;
    }
    return 0;
}

int gsp_compute_saxpy(struct gsp_compute *cp, struct gsp_ce *ce,
                      float a, const float *x, float *y, unsigned n,
                      uint64_t scratch_va, void *scratch_cpu)
{
    GspQmdV05 qmd;
    unsigned pb_off = 0, pb_len = 0;
    unsigned grid;

    (void)a;
    (void)x;
    if (!cp || !cp->ready || !ce || !y || n == 0) {
        return -1;
    }

    if (gsp_compute_stage_sass(cp, ce, scratch_va, scratch_cpu) != 0) {
        lx_printk("nouveau-lx: saxpy — fallo al copiar SASS a VRAM\n");
        return -1;
    }

    grid = (n + 255u) / 256u;
    gsp_compute_fill_saxpy_qmd(&qmd, cp->sass_va, grid);
    if (gsp_compute_encode_qmd(cp, &qmd, &pb_off, &pb_len) != 0 ||
        gsp_chan_submit(cp->chan, pb_off, pb_len) != 0) {
        lx_printk("nouveau-lx: saxpy — fallo al encolar QMD\n");
        return -1;
    }

    lx_printk("nouveau-lx: saxpy encolado (readback HW pendiente)\n");
    return -1;
}

void gsp_compute_fini(struct gsp_compute *cp)
{
    if (!cp) {
        return;
    }
    if (cp->ready && cp->rm && cp->rm->ready) {
        gsp_rm_free(cp->rm, cp->handle);
    }
    memset(cp, 0, sizeof(*cp));
}
