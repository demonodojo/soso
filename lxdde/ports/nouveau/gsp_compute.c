/* G4f/G5: objeto compute Blackwell + QMD inline. Ver gsp_compute.h. */
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

static unsigned char *cp_mv(struct gsp_compute *cp, unsigned off)
{
    return (unsigned char *)cp->mv.va + off;
}

/* Que el constant bank quepa donde se le reservó y que los parámetros estén
 * dentro de él. Se comprueba por kernel: los dos comparten la misma región de
 * 1 KiB y basta que uno crezca para que se salga. `param_count` va aquí porque es
 * el único sitio donde se cruzan lo que declara el cubin y lo que el port cree
 * que hay: el setter escribe N parámetros a pelo y el cubin puede decir otra
 * cosa desde que alguien toque el .cu. */
static int kernel_check(const struct gsp_kernel *k, unsigned want_params)
{
    if (k->sass_len < 16u || (k->sass_len % 16u) != 0u) {
        lx_printk("nouveau-lx: compute — SASS de %s son %u B (ni vacío ni "
                  "múltiplo de 16)\n", k->name, k->sass_len);
        return -1;
    }
    if (k->cbank_size > G4F_X_OFF ||
        k->param_base + k->param_size > k->cbank_size) {
        lx_printk("nouveau-lx: compute — cbank de %s: %u B no cabe (params en "
                  "%u+%u)\n", k->name, k->cbank_size, k->param_base,
                  k->param_size);
        return -1;
    }
    if (k->param_count != want_params) {
        lx_printk("nouveau-lx: compute — %s declara %u parámetros y el port "
                  "escribe %u: recompila o arregla el setter\n",
                  k->name, k->param_count, want_params);
        return -1;
    }
    return 0;
}

/* Sonda de variantes del objeto de compute, para gastar UN ciclo de hardware en vez
 * de uno por hipótesis.
 *
 * Si la combinación que dice el catálogo del chip (clase de compute sobre el canal
 * de GR0) es rechazada, aquí se prueban las demás y se imprime el status de cada
 * una. No arregla nada por sí sola: convierte "RM_ALLOC falló" —un bit— en una
 * tabla que dice QUÉ acepta RM, que es lo que hace falta para el siguiente cambio.
 * Es el mismo patrón que se usó con las once variantes del canal (2026-07-28), y
 * por la misma razón: un ciclo de VFIO cuesta un reinicio o cerrar la sesión.
 *
 * Cada intento que sale bien se libera acto seguido: la sonda diagnostica, no deja
 * objetos vivos por ahí. Y usa un handle propio para no pisar el del compute de
 * verdad si alguna variante llega a existir. */
static void compute_probe_variantes(struct gsp_rm *rm, struct gsp_chan *chan)
{
    static const uint32_t clases[] = {
        BLACKWELL_COMPUTE_B, BLACKWELL_COMPUTE_A, HOPPER_COMPUTE_A,
        ADA_COMPUTE_A, AMPERE_COMPUTE_B, AMPERE_COMPUTE_A,
    };
    /* Los tres padres plausibles: el canal (lo que hace nouveau), el device y el
     * subdevice. Si RM acepta la clase sobre el device pero no sobre el canal, el
     * problema es el canal; si no la acepta sobre ninguno, es la clase o el chip. */
    struct { const char *nombre; uint32_t handle; } padres[3];
    unsigned np = 0;
    unsigned i, j;
    const uint32_t sonda = NVKM_RM_COMPUTE0 | 0x0f00u;

    padres[np].nombre = "canal";
    padres[np++].handle = chan->handle;
    padres[np].nombre = "device";
    padres[np++].handle = rm->device;
    padres[np].nombre = "subdevice";
    padres[np++].handle = rm->subdevice;

    lx_printk("nouveau-lx: --- sonda compute: %u clases x %u padres ---\n",
              (unsigned)(sizeof(clases) / sizeof(clases[0])), np);
    for (i = 0; i < sizeof(clases) / sizeof(clases[0]); i++) {
        /* Sólo las que el chip dice que NO existen se saltan: pedir una clase que
         * el catálogo no lista contesta INVALID_CLASS por una razón distinta y
         * ensucia la tabla. `gsp_rm_class_supported` devuelve -1 cuando no hay
         * catálogo (`GET_CLASSLIST_V2` falló), y entonces se prueban todas — que es
         * justo cuando más falta hace la sonda. */
        if (gsp_rm_class_supported(clases[i]) == 0) {
            lx_printk("nouveau-lx: sonda compute: cls=0x%04x no está en el "
                      "catálogo del chip — saltada\n", clases[i]);
            continue;
        }
        for (j = 0; j < np; j++) {
            uint32_t status = 0;
            int rc = gsp_rm_alloc(rm, padres[j].handle, sonda, clases[i],
                                  NULL, 0, &status);

            lx_printk("nouveau-lx: sonda compute: cls=0x%04x sobre %s "
                      "(0x%08x) -> %s (status=0x%x)\n",
                      clases[i], padres[j].nombre, padres[j].handle,
                      rc == 0 ? "ACEPTADO" : "rechazado", status);
            if (rc == 0) {
                gsp_rm_free(rm, sonda);
            }
        }
    }
    lx_printk("nouveau-lx: --- fin de la sonda; el motor del canal era %u "
              "(GR0=%u, COPY0=%u) ---\n",
              chan->engine, NV2080_ENGINE_TYPE_GR0, NV2080_ENGINE_TYPE_COPY0);
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
    cp->data_va = G4F_DATA_VA;
    cp->mv_va = G5_MV_VA;

    cp->saxpy.name = "saxpy";
    cp->saxpy.sass = gsp_saxpy_sass;
    cp->saxpy.sass_len = gsp_saxpy_sass_len;
    cp->saxpy.regcount = gsp_saxpy_regcount;
    cp->saxpy.param_base = gsp_saxpy_param_base;
    cp->saxpy.param_size = gsp_saxpy_param_size;
    cp->saxpy.cbank_size = gsp_saxpy_cbank_size;
    cp->saxpy.param_off = gsp_saxpy_param_off;
    cp->saxpy.param_count = gsp_saxpy_param_count;
    cp->saxpy.sass_va = G4F_SASS_VA;

    cp->matvec.name = "matvec";
    cp->matvec.sass = gsp_matvec_sass;
    cp->matvec.sass_len = gsp_matvec_sass_len;
    cp->matvec.regcount = gsp_matvec_regcount;
    cp->matvec.param_base = gsp_matvec_param_base;
    cp->matvec.param_size = gsp_matvec_param_size;
    cp->matvec.cbank_size = gsp_matvec_cbank_size;
    cp->matvec.param_off = gsp_matvec_param_off;
    cp->matvec.param_count = gsp_matvec_param_count;
    cp->matvec.sass_va = G5_SASS_VA;

    /* El constant bank tiene que caber entero: el kernel lee sus parámetros en
     * `param_base`, que está al final de .nv.constant0. */
    if (kernel_check(&cp->saxpy, 4u) != 0 ||
        kernel_check(&cp->matvec, 5u) != 0) {
        return -1;
    }
    /* Los dos blobs viven en páginas distintas de VRAM y se copian por la página
     * de rebote de 4 KiB: si uno crece por encima de eso, pisaría al otro. */
    if (cp->saxpy.sass_len > 4096u || cp->matvec.sass_len > 4096u ||
        cp->matvec.sass_va - cp->saxpy.sass_va < 4096ull) {
        lx_printk("nouveau-lx: compute — los blobs SASS no caben en su página "
                  "(saxpy %u B, matvec %u B)\n",
                  cp->saxpy.sass_len, cp->matvec.sass_len);
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

    /* Staging de G5. No es fatal quedarse sin él: sin este búfer el matvec se
     * hace en CPU y el resto de G4f sigue en pie, así que se avisa y se sigue. */
    if (gsp_dma_alloc(&cp->mv, G5_MV_SIZE, "compute matvec") != 0) {
        lx_printk("nouveau-lx: compute — sin staging de matvec (G5 irá por CPU)\n");
    } else if (gsp_vmm_map(chan->vmm, cp->mv_va, cp->mv.phys, G5_MV_SIZE,
                           GSP_VMM_SYSMEM) != 0) {
        lx_printk("nouveau-lx: compute — no se pudo mapear el staging de matvec\n");
        gsp_dma_free(&cp->mv);
    } else {
        cp->mv_mapped = 1;
    }

    {
        /* GB20x lleva `BLACKWELL_COMPUTE_B`; la A que había aquí es de GB100.
         * Igual que el canal y el CE: lo dice el catálogo, no un #define. */
        static const uint32_t cand[] = {
            BLACKWELL_COMPUTE_B, BLACKWELL_COMPUTE_A, HOPPER_COMPUTE_A,
            ADA_COMPUTE_A, AMPERE_COMPUTE_B, AMPERE_COMPUTE_A,
        };

        cp->cls = gsp_rm_class_pick("compute", cand,
                                    (unsigned)(sizeof(cand) / sizeof(cand[0])));
    }

    {
        uint32_t status = 0;

        if (gsp_rm_alloc(rm, chan->handle, cp->handle, cp->cls,
                         NULL, 0, &status) != 0) {
            lx_printk("nouveau-lx: RM_ALLOC compute rechazado (cls=0x%04x, "
                      "status=0x%x) — sondeando variantes en ESTE arranque\n",
                      cp->cls, status);
            compute_probe_variantes(rm, chan);
            /* En HW esto devolvió INVALID_CLASS (0x22) con la clase BIEN:
             * `cand[0]` es `BLACKWELL_COMPUTE_B`, que es lo que `rm/gb20x.c` pone
             * para este chip, y el mismo canal aceptó `BLACKWELL_DMA_COPY_B` sin
             * queja (2026-07-28).
             *
             * La hipótesis es que lo que no cuadra es el CANAL, no la clase: aquel
             * colgaba de un canal atado a `NV2080_ENGINE_TYPE_COPY0`, y un objeto
             * de compute necesita un canal del motor de gráficos (GR0 = 1). Por eso
             * el bring-up levanta ahora un segundo canal sobre GR0. Si aun así se
             * llega hasta aquí, la hipótesis era falsa — y la tabla que acaba de
             * imprimir la sonda dice qué acepta RM de verdad. */
            lx_printk("nouveau-lx: RM_ALLOC compute falló (cls=0x%04x) sobre el "
                      "canal 0x%08x del motor %u — si es INVALID_CLASS y el motor "
                      "no es GR0 (%u), el problema es el canal, no la clase\n",
                      cp->cls, chan->handle, chan->engine,
                      NV2080_ENGINE_TYPE_GR0);
            if (cp->mv_mapped) {
                gsp_dma_free(&cp->mv);
                cp->mv_mapped = 0;
            }
            gsp_dma_free(&cp->data);
            cp->mapped = 0;
            return -1;
        }
    }
    cp->ready = 1;
    lx_printk("nouveau-lx: compute listo cls=0x%04x handle=0x%08x sobre canal "
              "0x%08x (motor %u) — saxpy %u B/%u regs, matvec %u B/%u regs, "
              "staging G5 %s\n",
              cp->cls, cp->handle, chan->handle, chan->engine,
              cp->saxpy.sass_len, cp->saxpy.regcount,
              cp->matvec.sass_len, cp->matvec.regcount,
              cp->mv_mapped ? "listo" : "NO");
    return 0;
}

int gsp_compute_stage_sass(struct gsp_compute *cp, struct gsp_ce *ce,
                           const struct gsp_kernel *k,
                           uint64_t scratch_va, void *scratch_cpu)
{
    if (!cp || !cp->ready || !ce || !k || !scratch_cpu || !k->sass_len) {
        return -1;
    }
    if (k->sass_len > 4096) {
        return -1;
    }
    memcpy(scratch_cpu, k->sass, k->sass_len);
    __asm__ __volatile__("mfence" ::: "memory");

    return gsp_ce_copy_sync(ce, k->sass_va, scratch_va, k->sass_len,
                            GSP_CE_WAIT_MS);
}

/* El prólogo del banco 0 (todo lo anterior a `param_base`) lo rellena el driver
 * de CUDA con su propio ABI, que no está documentado. De ahí solo escribimos
 * ntid: el kernel compilado usa blockDim.x, que en Volta y posteriores se lee de
 * c[0x0][0x0]. Es la única suposición que queda en esta ruta; si el readback sale
 * mal, empieza por aquí (y la salida limpia es recompilar el .cu sin blockDim,
 * con el tamaño de bloque constante). */
static unsigned char *cbank_prepare(struct gsp_compute *cp,
                                    const struct gsp_kernel *k)
{
    unsigned char *cb = cp_data(cp, G4F_CBANK_OFF);

    memset(cb, 0, k->cbank_size);
    *(uint32_t *)(cb + 0x0) = G4F_CTA_THREADS;
    *(uint32_t *)(cb + 0x4) = 1u;
    *(uint32_t *)(cb + 0x8) = 1u;
    return cb + k->param_base;
}

void gsp_compute_set_params(struct gsp_compute *cp, float a, uint64_t x_va,
                            uint64_t y_va, unsigned n)
{
    const struct gsp_kernel *k = &cp->saxpy;
    unsigned char *p = cbank_prepare(cp, k);

    *(float *)(p + k->param_off[0]) = a;
    *(uint64_t *)(p + k->param_off[1]) = x_va;
    *(uint64_t *)(p + k->param_off[2]) = y_va;
    *(uint32_t *)(p + k->param_off[3]) = n;

    __asm__ __volatile__("mfence" ::: "memory");
}

void gsp_compute_set_mv_params(struct gsp_compute *cp, uint64_t w_va,
                               uint64_t x_va, uint64_t y_va, unsigned rows,
                               unsigned cols)
{
    const struct gsp_kernel *k = &cp->matvec;
    unsigned char *p = cbank_prepare(cp, k);

    *(uint64_t *)(p + k->param_off[0]) = w_va;
    *(uint64_t *)(p + k->param_off[1]) = x_va;
    *(uint64_t *)(p + k->param_off[2]) = y_va;
    /* `rows` y `cols` son `int` en el .cu: 32 bits, no 64. Escribir 8 B aquí
     * pisaría el siguiente parámetro. */
    *(uint32_t *)(p + k->param_off[3]) = rows;
    *(uint32_t *)(p + k->param_off[4]) = cols;

    __asm__ __volatile__("mfence" ::: "memory");
}

void gsp_compute_fill_qmd(struct gsp_compute *cp, const struct gsp_kernel *k,
                          GspQmdV05 *qmd, unsigned grid_x)
{
    uint64_t prog_shift = k->sass_va >> 4;
    uint64_t cb_va = cp->data_va + G4F_CBANK_OFF;
    uint64_t cb_shift = cb_va >> 6;
    uint64_t sem_va = cp->data_va + G4F_SEM_OFF;
    /* SIZE_SHIFTED4 ⇒ redondeo a múltiplo de 16 B hacia arriba. */
    uint32_t cb_size = (k->cbank_size + 15u) & ~15u;

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
     * declara en EIATTR_REGCOUNT y con 0 el lanzamiento no es válido. Sale del
     * kernel que se va a lanzar, no de saxpy: matvec usa 37 registros y saxpy 10,
     * y lanzar matvec con 10 es corrupción de registros, no un error. */
    qmd_set_bits(qmd->words, QMDV05_REGISTER_COUNT, k->regcount);
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

/* Encola el QMD de `k` y espera su semáforo. El semáforo tiene que estar a cero
 * ANTES de llamar aquí: quien stagea los datos es quien lo pone, porque es la
 * misma escritura que publica lo que el kernel va a leer.
 *
 * El rebobinado del pushbuffer está aquí y no en el bucle de tandas porque la
 * condición es "no cupo", no "voy por la tanda N": un QMD inline son ~900 B y en
 * los 4 KiB del pushbuffer caben cuatro. */
static int launch_wait(struct gsp_compute *cp, const struct gsp_kernel *k,
                       unsigned grid, const char *what)
{
    GspQmdV05 qmd;
    unsigned pb_off = 0, pb_len = 0, waited;
    const volatile uint32_t *sem;

    gsp_compute_fill_qmd(cp, k, &qmd, grid);
    if (gsp_compute_encode_qmd(cp, &qmd, &pb_off, &pb_len) != 0) {
        if (gsp_chan_pb_rewind(cp->chan) != 0 ||
            gsp_compute_encode_qmd(cp, &qmd, &pb_off, &pb_len) != 0) {
            lx_printk("nouveau-lx: %s — pushbuffer lleno y sin rebobinar\n", what);
            return -1;
        }
    }
    if (gsp_chan_submit(cp->chan, pb_off, pb_len) != 0) {
        lx_printk("nouveau-lx: %s — fallo al encolar QMD\n", what);
        return -1;
    }

    sem = (const volatile uint32_t *)cp_data(cp, G4F_SEM_OFF);
    /* Primero sondeo a pelo y sólo después `mdelay`. Un kernel de una tanda tarda
     * microsegundos y el bucle de milisegundos convertiría un matvec de 8192 filas
     * (miles de tandas) en segundos de dormir, no de calcular. */
    for (waited = 0; waited < G4F_SPIN_TRIES; waited++) {
        __asm__ __volatile__("mfence" ::: "memory");
        if (*sem == G4F_SEM_PAYLOAD) {
            return 0;
        }
    }
    for (waited = 0; waited <= G4F_WAIT_MS; waited++) {
        __asm__ __volatile__("mfence" ::: "memory");
        if (*sem == G4F_SEM_PAYLOAD) {
            return 0;
        }
        lx_mdelay(1);
    }
    lx_printk("nouveau-lx: %s — el QMD no señalizó en %u ms (sem=0x%08x)\n",
              what, G4F_WAIT_MS, *sem);
    /* GPGet parte el problema en dos mitades que no se solapan; sin esto, un
     * kernel que no arranca y uno que no señaliza se ven igual desde fuera. */
    gsp_chan_dump(cp->chan, what);
    return -1;
}

int gsp_compute_saxpy(struct gsp_compute *cp, struct gsp_ce *ce,
                      float a, const float *x, float *y, unsigned n,
                      uint64_t scratch_va, void *scratch_cpu)
{
    unsigned grid, i;
    float *gx, *gy;

    if (!cp || !cp->ready || !ce || !x || !y || n == 0 || n > G4F_MAX_N) {
        return -1;
    }

    if (gsp_compute_stage_sass(cp, ce, &cp->saxpy, scratch_va, scratch_cpu) != 0) {
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
    if (launch_wait(cp, &cp->saxpy, grid, "saxpy") != 0) {
        return -1;
    }

    for (i = 0; i < n; i++) {
        y[i] = gy[i];
    }
    lx_printk("nouveau-lx: saxpy en GPU OK — n=%u grid=%u\n", n, grid);
    return 0;
}

/* Última forma de matvec anunciada por el log. Ver el final de
 * `gsp_compute_matvec_f32`. */
static unsigned g_mv_last_rows;
static unsigned g_mv_last_cols;

unsigned gsp_compute_mv_rows_per_tile(unsigned cols)
{
    unsigned rows;

    if (cols == 0u || cols > G5_MAX_COLS) {
        return 0u;
    }
    rows = G5_MV_W_BYTES / (cols * 4u);
    /* La tanda la limitan las DOS regiones: las filas que caben en W y los
     * resultados que caben en Y. Mirar solo W deja `y` corto en cuanto las
     * columnas son pocas (con cols=1 saldrían 59392 filas en 4 KiB de Y). */
    if (rows > G5_MAX_TILE_ROWS) {
        rows = G5_MAX_TILE_ROWS;
    }
    return rows;
}

void gsp_compute_mv_stage(struct gsp_compute *cp, const float *w, const float *x,
                          unsigned rows, unsigned cols, unsigned row0)
{
    unsigned char *gw = cp_mv(cp, G5_MV_W_OFF);
    unsigned char *gx = cp_mv(cp, G5_MV_X_OFF);

    /* `(unsigned long)` en el índice, no `unsigned`: con rows*cols de un modelo
     * de verdad (4096×4096 = 16 M floats) un producto de 32 bits se pasa de vuelta
     * y la tanda copiaría filas de otro sitio de la matriz. */
    memcpy(gw, w + (unsigned long)row0 * (unsigned long)cols,
           (unsigned long)rows * (unsigned long)cols * 4ul);
    memcpy(gx, x, (unsigned long)cols * 4ul);
    memset(cp_mv(cp, G5_MV_Y_OFF), 0, (unsigned long)rows * 4ul);
    /* El semáforo va a cero AQUÍ, en la misma tanda de escrituras que publica los
     * datos: si se pusiera antes del staging, un semáforo del lanzamiento anterior
     * todavía a `PAYLOAD` daría la tanda por buena sin que la GPU hiciera nada. */
    *(uint32_t *)cp_data(cp, G4F_SEM_OFF) = 0;
    __asm__ __volatile__("mfence" ::: "memory");
}

void gsp_compute_mv_read(struct gsp_compute *cp, float *y, unsigned rows,
                         unsigned row0)
{
    __asm__ __volatile__("mfence" ::: "memory");
    memcpy(y + row0, cp_mv(cp, G5_MV_Y_OFF), (unsigned long)rows * 4ul);
}

int gsp_compute_matvec_f32(struct gsp_compute *cp, struct gsp_ce *ce,
                           const float *w, unsigned rows, unsigned cols,
                           const float *x, float *y,
                           uint64_t scratch_va, void *scratch_cpu)
{
    unsigned per_tile, row0, tiles = 0;

    if (!cp || !cp->ready || !ce || !w || !x || !y || rows == 0u || cols == 0u) {
        return -1;
    }
    if (!cp->mv_mapped) {
        return -1;
    }
    per_tile = gsp_compute_mv_rows_per_tile(cols);
    if (per_tile == 0u) {
        lx_printk("nouveau-lx: matvec — %u columnas no caben en el staging "
                  "(tope %u)\n", cols, G5_MAX_COLS);
        return -1;
    }
    if (gsp_compute_stage_sass(cp, ce, &cp->matvec, scratch_va, scratch_cpu) != 0) {
        lx_printk("nouveau-lx: matvec — fallo al copiar SASS a VRAM\n");
        return -1;
    }

    for (row0 = 0u; row0 < rows; row0 += per_tile) {
        unsigned n = (rows - row0) < per_tile ? (rows - row0) : per_tile;
        unsigned grid = (n + G4F_CTA_THREADS - 1u) / G4F_CTA_THREADS;

        gsp_compute_mv_stage(cp, w, x, n, cols, row0);
        gsp_compute_set_mv_params(cp, cp->mv_va + G5_MV_W_OFF,
                                  cp->mv_va + G5_MV_X_OFF,
                                  cp->mv_va + G5_MV_Y_OFF, n, cols);
        /* Una tanda que falla aborta el matvec entero. Devolver 0 con las filas
         * de arriba hechas y las de abajo a medias sería lo peor de los dos
         * mundos: `soso-llm` se creería el vector y el modelo escupiría texto
         * plausible pero mal. Quien llama vuelve a CPU con la matriz intacta. */
        if (launch_wait(cp, &cp->matvec, grid, "matvec") != 0) {
            lx_printk("nouveau-lx: matvec — tanda %u (filas %u..%u de %u) falló\n",
                      tiles, row0, row0 + n, rows);
            return -1;
        }
        gsp_compute_mv_read(cp, y, n, row0);
        tiles++;
    }

    /* Una línea por FORMA, no por llamada. Un matvec por capa y por token son
     * miles de llamadas con las mismas dimensiones, y escribir eso por el puerto
     * serie costaría más que el propio cálculo — pero callar del todo dejaría sin
     * la única prueba de que el camino de GPU se está usando. Las formas se
     * repiten, así que esto se estabiliza en unas pocas líneas por modelo. */
    if (rows != g_mv_last_rows || cols != g_mv_last_cols) {
        g_mv_last_rows = rows;
        g_mv_last_cols = cols;
        lx_printk("nouveau-lx: matvec en GPU OK — %ux%u en %u tanda(s) de %u "
                  "filas\n", rows, cols, tiles, per_tile);
    }
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
    if (cp->mv.va) {
        gsp_dma_free(&cp->mv);
    }
    if (cp->data.va) {
        gsp_dma_free(&cp->data);
    }
    memset(cp, 0, sizeof(*cp));
}
