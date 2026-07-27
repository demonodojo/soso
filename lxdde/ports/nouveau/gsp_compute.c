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

static unsigned char *cp_data(struct gsp_compute *cp, unsigned off)
{
    return (unsigned char *)cp->data.va + off;
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
    cp->data_va = G4F_DATA_VA;

    /* El constant bank tiene que caber entero: el kernel lee sus parámetros en
     * `param_base`, que está al final de .nv.constant0. */
    if (gsp_saxpy_cbank_size > G4F_X_OFF ||
        gsp_saxpy_param_base + gsp_saxpy_param_size > gsp_saxpy_cbank_size) {
        lx_printk("nouveau-lx: compute — cbank de %u B no cabe (params en %u+%u)\n",
                  gsp_saxpy_cbank_size, gsp_saxpy_param_base, gsp_saxpy_param_size);
        return -1;
    }

    if (gsp_dma_alloc(&cp->data, G4F_DATA_SIZE, "compute data") != 0) {
        return -1;
    }
    if (gsp_vmm_map(chan->vmm, cp->data_va, cp->data.phys, G4F_DATA_SIZE,
                    GSP_VMM_SYSMEM) != 0) {
        lx_printk("nouveau-lx: compute — no se pudo mapear la sysmem de datos\n");
        gsp_dma_free(&cp->data);
        return -1;
    }
    cp->mapped = 1;

    if (gsp_rm_alloc(rm, chan->handle, cp->handle, BLACKWELL_COMPUTE_A,
                     NULL, 0, NULL) != 0) {
        lx_printk("nouveau-lx: RM_ALLOC compute falló\n");
        gsp_dma_free(&cp->data);
        cp->mapped = 0;
        return -1;
    }
    cp->ready = 1;
    lx_printk("nouveau-lx: compute listo handle=0x%08x sass=%u B regs=%u "
              "params@cbank0+0x%x\n",
              cp->handle, cp->sass_size, gsp_saxpy_regcount, gsp_saxpy_param_base);
    return 0;
}

int gsp_compute_stage_sass(struct gsp_compute *cp, struct gsp_ce *ce,
                           uint64_t scratch_va, void *scratch_cpu)
{
    if (!cp || !cp->ready || !ce || !scratch_cpu || !cp->sass_size) {
        return -1;
    }
    if (cp->sass_size > 4096) {
        return -1;
    }
    memcpy(scratch_cpu, gsp_saxpy_sass, cp->sass_size);
    __asm__ __volatile__("mfence" ::: "memory");

    return gsp_ce_copy_sync(ce, cp->sass_va, scratch_va, cp->sass_size,
                            GSP_CE_WAIT_MS);
}

void gsp_compute_set_params(struct gsp_compute *cp, float a, uint64_t x_va,
                            uint64_t y_va, unsigned n)
{
    unsigned char *cb = cp_data(cp, G4F_CBANK_OFF);
    unsigned char *p = cb + gsp_saxpy_param_base;

    memset(cb, 0, gsp_saxpy_cbank_size);

    /* El prólogo del banco 0 (todo lo anterior a `param_base`) lo rellena el
     * driver de CUDA con su propio ABI, que no está documentado. De ahí solo
     * escribimos ntid: el kernel compilado usa blockDim.x, que en Volta y
     * posteriores se lee de c[0x0][0x0]. Es la única suposición que queda en
     * esta ruta; si el readback sale mal, empieza por aquí (y la salida limpia
     * es recompilar el .cu sin blockDim, con el tamaño de bloque constante). */
    *(uint32_t *)(cb + 0x0) = G4F_CTA_THREADS;
    *(uint32_t *)(cb + 0x4) = 1u;
    *(uint32_t *)(cb + 0x8) = 1u;

    *(float *)(p + gsp_saxpy_param_off[0]) = a;
    *(uint64_t *)(p + gsp_saxpy_param_off[1]) = x_va;
    *(uint64_t *)(p + gsp_saxpy_param_off[2]) = y_va;
    *(uint32_t *)(p + gsp_saxpy_param_off[3]) = n;

    __asm__ __volatile__("mfence" ::: "memory");
}

void gsp_compute_fill_saxpy_qmd(struct gsp_compute *cp, GspQmdV05 *qmd,
                                uint64_t prog_va, unsigned grid_x)
{
    uint64_t prog_shift = prog_va >> 4;
    uint64_t cb_va = cp->data_va + G4F_CBANK_OFF;
    uint64_t cb_shift = cb_va >> 6;
    uint64_t sem_va = cp->data_va + G4F_SEM_OFF;
    /* SIZE_SHIFTED4 ⇒ redondeo a múltiplo de 16 B hacia arriba. */
    uint32_t cb_size = (gsp_saxpy_cbank_size + 15u) & ~15u;

    memset(qmd, 0, sizeof(*qmd));
    qmd_set_bits(qmd->words, QMDV05_QMD_TYPE, NVCDC0_QMDV05_00_QMD_TYPE_GRID_CTA);
    qmd_set_bits(qmd->words, QMDV05_QMD_MAJOR_VERSION,
                 NVCDC0_QMDV05_00_QMD_MAJOR_VERSION_V05);

    qmd_set_bits(qmd->words, QMDV05_GRID_WIDTH, grid_x ? grid_x : 1u);
    qmd_set_bits(qmd->words, QMDV05_GRID_HEIGHT, 1u);
    qmd_set_bits(qmd->words, QMDV05_GRID_DEPTH, 1u);
    qmd_set_bits(qmd->words, QMDV05_CTA_THREAD_DIMENSION0, G4F_CTA_THREADS);
    qmd_set_bits(qmd->words, QMDV05_CTA_THREAD_DIMENSION1, 1u);
    qmd_set_bits(qmd->words, QMDV05_CTA_THREAD_DIMENSION2, 1u);

    qmd_set_bits(qmd->words, QMDV05_PROGRAM_ADDRESS_LOWER_S4,
                 (uint32_t)(prog_shift & 0xffffffffu));
    qmd_set_bits(qmd->words, QMDV05_PROGRAM_ADDRESS_UPPER_S4,
                 (uint32_t)((prog_shift >> 32) & 0x1ffffu));

    /* Sin esto el SM no sabe cuántos registros reservar por hilo: el cubin lo
     * declara en EIATTR_REGCOUNT y con 0 el lanzamiento no es válido. */
    qmd_set_bits(qmd->words, QMDV05_REGISTER_COUNT, gsp_saxpy_regcount);
    qmd_set_bits(qmd->words, QMDV05_BARRIER_COUNT, 0u);
    qmd_set_bits(qmd->words, QMDV05_SHARED_MEMORY_SIZE_S7, 0u);

    /* Constant bank 0 = donde el kernel lee sus parámetros. */
    qmd_set_bits(qmd->words, QMDV05_CBANK0_ADDR_LOWER_S6,
                 (uint32_t)(cb_shift & 0xffffffffu));
    qmd_set_bits(qmd->words, QMDV05_CBANK0_ADDR_UPPER_S6,
                 (uint32_t)((cb_shift >> 32) & 0x7ffffu));
    qmd_set_bits(qmd->words, QMDV05_CBANK0_SIZE_S4, cb_size >> 4);
    qmd_set_bits(qmd->words, QMDV05_CBANK0_VALID,
                 NVCDC0_QMDV05_00_CONSTANT_BUFFER_VALID_TRUE);
    qmd_set_bits(qmd->words, QMDV05_CBANK0_INVALIDATE,
                 NVCDC0_QMDV05_00_CONSTANT_BUFFER_INVALIDATE_TRUE);

    /* Semáforo de fin: sin él no hay forma de saber que el kernel terminó, y
     * "on_gpu" volvería a ser una afirmación sin prueba. */
    qmd_set_bits(qmd->words, QMDV05_RELEASE_ENABLE0,
                 NVCDC0_QMDV05_00_RELEASE_ENABLE_TRUE);
    qmd_set_bits(qmd->words, QMDV05_RELEASE_STRUCTURE_SIZE0,
                 NVCDC0_QMDV05_00_RELEASE_STRUCTURE_SIZE_SEMAPHORE_ONE_WORD);
    qmd_set_bits(qmd->words, QMDV05_RELEASE_MEMBAR_TYPE0,
                 NVCDC0_QMDV05_00_RELEASE_MEMBAR_TYPE_FE_SYSMEMBAR);
    qmd_set_bits(qmd->words, QMDV05_RELEASE_SEM0_ADDR_LOWER,
                 (uint32_t)(sem_va & 0xffffffffu));
    qmd_set_bits(qmd->words, QMDV05_RELEASE_SEM0_ADDR_UPPER,
                 (uint32_t)((sem_va >> 32) & 0x1ffffffu));
    qmd_set_bits(qmd->words, QMDV05_RELEASE_SEM0_PAYLOAD_LOWER, G4F_SEM_PAYLOAD);
}

int gsp_compute_encode_qmd(struct gsp_compute *cp, const GspQmdV05 *qmd,
                           unsigned *pb_off, unsigned *pb_len)
{
    struct gsp_chan *c;
    unsigned pos;
    unsigned start;
    unsigned i;
    uint64_t qmd_va = G4F_QMD_VA;

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
    unsigned grid, i, waited;
    float *gx, *gy;
    const volatile uint32_t *sem;

    if (!cp || !cp->ready || !ce || !x || !y || n == 0 || n > G4F_MAX_N) {
        return -1;
    }

    if (gsp_compute_stage_sass(cp, ce, scratch_va, scratch_cpu) != 0) {
        lx_printk("nouveau-lx: saxpy — fallo al copiar SASS a VRAM\n");
        return -1;
    }

    gx = (float *)cp_data(cp, G4F_X_OFF);
    gy = (float *)cp_data(cp, G4F_Y_OFF);
    for (i = 0; i < n; i++) {
        gx[i] = x[i];
        gy[i] = y[i];
    }
    *(uint32_t *)cp_data(cp, G4F_SEM_OFF) = 0;

    gsp_compute_set_params(cp, a, cp->data_va + G4F_X_OFF,
                           cp->data_va + G4F_Y_OFF, n);

    grid = (n + G4F_CTA_THREADS - 1u) / G4F_CTA_THREADS;
    gsp_compute_fill_saxpy_qmd(cp, &qmd, cp->sass_va, grid);
    if (gsp_compute_encode_qmd(cp, &qmd, &pb_off, &pb_len) != 0 ||
        gsp_chan_submit(cp->chan, pb_off, pb_len) != 0) {
        lx_printk("nouveau-lx: saxpy — fallo al encolar QMD\n");
        return -1;
    }

    sem = (const volatile uint32_t *)cp_data(cp, G4F_SEM_OFF);
    for (waited = 0; waited <= G4F_WAIT_MS; waited++) {
        __asm__ __volatile__("mfence" ::: "memory");
        if (*sem == G4F_SEM_PAYLOAD) {
            break;
        }
        lx_mdelay(1);
    }
    if (*sem != G4F_SEM_PAYLOAD) {
        lx_printk("nouveau-lx: saxpy — el QMD no señalizó en %u ms (sem=0x%08x)\n",
                  G4F_WAIT_MS, *sem);
        return -1;
    }

    for (i = 0; i < n; i++) {
        y[i] = gy[i];
    }
    lx_printk("nouveau-lx: saxpy en GPU OK — n=%u grid=%u\n", n, grid);
    return 0;
}

void gsp_compute_fini(struct gsp_compute *cp)
{
    if (!cp) {
        return;
    }
    if (cp->ready && cp->rm && cp->rm->ready) {
        gsp_rm_free(cp->rm, cp->handle);
    }
    if (cp->data.va) {
        gsp_dma_free(&cp->data);
    }
    memset(cp, 0, sizeof(*cp));
}
