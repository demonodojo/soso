/* G4f/G5: objeto compute Blackwell + QMD en sysmem + SEND_PCAS. Ver gsp_compute.h. */
#include "gsp_compute.h"
#include "sass_sets.h"
/* Constantes de bloque de los formatos cuantizados, compartidas con los .cu. */
#include "q4k_decode.h"

void *memset(void *dst, int c, unsigned long n);
void *memcpy(void *dst, const void *src, unsigned long n);

/* `NVA06F_SUBCHANNEL_COMPUTE` = 1 (`cla06fsubch.h`). */
#define GSP_COMPUTE_SUBCHANNEL 1u

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

/* IMMD_DATA_METHOD lleva el dato EN LA CABECERA (bits 28:16, máx 13 bits) y no
 * hay dword de datos detrás. La reescritura de G4h puso un `1` literal en ese
 * campo y escribió el dato como dword suelto; el PBDMA leía ese dword como la
 * SIGUIENTE cabecera —tras el WFI quedaba un 0x00000000 crudo, método INC con
 * count=0— y levantaba PBDMA_ERROR (type 32) sin consumir nada (2026-07-29,
 * GPGet=0 con GPPut avanzando). */
static void cp_pb_immd(struct gsp_chan *c, unsigned *pos, unsigned subc,
                       unsigned method, uint32_t data)
{
    uint32_t hdr = (NVC56F_DMA_SEC_OP_IMMD_DATA_METHOD << 29) |
                   (subc << 13) |
                   ((data & 0x1fffu) << 16) |
                   ((method >> 2) & 0xfffu);

    if (data > 0x1fffu) {
        lx_printk("nouveau-lx: compute — dato IMMD 0x%x no cabe en 13 bits "
                  "(método 0x%x); usa cp_pb_method\n", data, method);
    }
    cp_pb_write(c, pos, hdr);
}

/* Paridad nouveau/UVM: `SET_OBJECT` lleva la clase (bits 15:0) por el subcanal
 * de compute (1), no el handle de RM por el subcanal 0 (= GR). */
static void cp_pb_set_object(struct gsp_chan *c, unsigned *pos, unsigned subc,
                             uint32_t method, uint32_t oclass)
{
    cp_pb_method(c, pos, subc, method, 1);
    cp_pb_write(c, pos, oclass);
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

static unsigned char *cp_res(struct gsp_compute *cp, unsigned off)
{
    return (unsigned char *)cp->res.va + off;
}

/* Comparación de cadenas sin libc: el port no enlaza strcmp. */
static int str_igual(const char *a, const char *b)
{
    if (!a || !b) {
        return 0;
    }
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == '\0' && *b == '\0';
}

/* --- R5: capacidades por familia ---------------------------------------
 *
 * Aceptar una clase de compute no demuestra nada sobre el descriptor ni sobre
 * el código máquina. Aquí se junta lo que hace falta para lanzar de verdad:
 *
 *   - la versión de QMD, que es distinta por familia y de la que solo se tiene
 *     transcrito el layout de Blackwell (`QMDV05_*` en nvrm_r570.h, de
 *     `clcec0qmd.h`). El árbol solo trae `cla0c0qmd.h`, que llega a V01_07
 *     (era Pascal): NO es el QMD de Ampere/Ada/Hopper, así que para esas
 *     familias `qmd_version` es 0 = «no sé escribir el descriptor», y el
 *     lanzamiento se rechaza ANTES de enviar nada.
 *   - la arquitectura del SASS. Los blobs no son intercambiables: el mismo
 *     .cu compilado a sm_86 y a sm_120 cambia hasta de sitio los parámetros
 *     (param_base 352 frente a 896), así que lanzar el juego equivocado es
 *     leer basura en el constant bank.
 */
static const struct gsp_family_caps g_family_caps[] = {
    { GSP_FAM_AMPERE,    "Ampere",    GSP_QMD_VERSION_AMPERE, 256u, "sm_86"  },
    { GSP_FAM_ADA,       "Ada",       0u, 0u,   "sm_89"  },
    { GSP_FAM_HOPPER,    "Hopper",    0u, 0u,   "sm_90"  },
    { GSP_FAM_BLACKWELL, "Blackwell", GSP_QMD_VERSION_CURRENT, 384u, "sm_120" },
};

unsigned gsp_family_from_class(uint32_t cls)
{
    switch (cls) {
    case AMPERE_COMPUTE_A:
    case AMPERE_COMPUTE_B:
        return GSP_FAM_AMPERE;
    case ADA_COMPUTE_A:
        return GSP_FAM_ADA;
    case HOPPER_COMPUTE_A:
        return GSP_FAM_HOPPER;
    case BLACKWELL_COMPUTE_A:
    case BLACKWELL_COMPUTE_B:
        return GSP_FAM_BLACKWELL;
    default:
        return GSP_FAM_DESCONOCIDA;
    }
}

unsigned gsp_family_from_nv(int nv_family)
{
    switch (nv_family) {
    case NV_FAM_AMPERE:
        return GSP_FAM_AMPERE;
    case NV_FAM_ADA:
        return GSP_FAM_ADA;
    case NV_FAM_BLACKWELL:
        return GSP_FAM_BLACKWELL;
    default:
        return GSP_FAM_DESCONOCIDA;
    }
}

const struct gsp_family_caps *gsp_family_caps_of(unsigned family)
{
    unsigned i;

    for (i = 0; i < sizeof(g_family_caps) / sizeof(g_family_caps[0]); i++) {
        if (g_family_caps[i].family == family) {
            return &g_family_caps[i];
        }
    }
    return 0;
}

const struct gsp_sass_set *gsp_sass_pick(const struct gsp_sass_set *const *sets,
                                         unsigned n, unsigned family)
{
    unsigned i;

    if (!sets || family == GSP_FAM_DESCONOCIDA) {
        return 0;
    }
    for (i = 0; i < n; i++) {
        if (sets[i] && sets[i]->family == family) {
            return sets[i];
        }
    }
    return 0;
}

const struct gsp_sass_variant *gsp_sass_variant_of(const struct gsp_sass_set *set,
                                                   const char *name)
{
    unsigned i;

    if (!set || !name) {
        return 0;
    }
    for (i = 0; i < set->count; i++) {
        const char *a = set->vars[i].name;
        const char *b = name;

        while (*a && *a == *b) {
            a++;
            b++;
        }
        if (*a == '\0' && *b == '\0') {
            return &set->vars[i];
        }
    }
    return 0;
}

int gsp_compute_launch_ready(const struct gsp_compute *cp,
                             const struct gsp_kernel *k, const char *what)
{
    const char *quien = what ? what : "compute";

    if (!cp || !cp->ready || !k) {
        return -1;
    }
    if (!cp->caps || cp->caps->qmd_version == 0u) {
        lx_printk("nouveau-lx: %s — sin descriptor QMD para la familia %s: no se "
                  "envía nada.\n", quien, cp->caps ? cp->caps->nombre : "desconocida");
        return -1;
    }
    if (cp->caps->qmd_version == GSP_QMD_VERSION_AMPERE) {
        if (cp->caps->qmd_bytes != (unsigned)sizeof(GspQmdV02)) {
            lx_printk("nouveau-lx: %s — Ampere pide QMD v2 de %u B (struct %zu)\n",
                      quien, cp->caps->qmd_bytes, sizeof(GspQmdV02));
            return -1;
        }
    } else if (cp->caps->qmd_version != GSP_QMD_VERSION_CURRENT ||
               cp->caps->qmd_bytes != (unsigned)sizeof(GspQmdV05)) {
        lx_printk("nouveau-lx: %s — la familia %s pide QMD v%u de %u B y el "
                  "port escribe v%u de %u B\n", quien, cp->caps->nombre,
                  cp->caps->qmd_version, cp->caps->qmd_bytes,
                  GSP_QMD_VERSION_CURRENT, (unsigned)sizeof(GspQmdV05));
        return -1;
    }
    if (!k->arch || !cp->caps->sass_arch || !str_igual(k->arch, cp->caps->sass_arch)) {
        lx_printk("nouveau-lx: %s — SASS de %s es %s y la familia %s necesita "
                  "%s: recompila con SOSO_SASS_ARCHS\n", quien, k->name,
                  k->arch ? k->arch : "?", cp->caps->nombre,
                  cp->caps->sass_arch ? cp->caps->sass_arch : "?");
        return -1;
    }
    if (!k->staged) {
        lx_printk("nouveau-lx: %s — el SASS de %s no está en VRAM\n", quien,
                  k->name);
        return -1;
    }
    return 0;
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

    /* R5: los kernels salen del juego de SASS de la familia detectada, no de
     * unos símbolos fijos. La familia se decide con la clase de compute del
     * catálogo del chip; si no hay catálogo, con la familia del chip. */
    {
        static const struct gsp_sass_set *const sets[] = GSP_SASS_SETS;
        static const struct { const char *name; uint64_t va; unsigned params; }
        tabla[] = {
            { "saxpy",          G4F_SASS_VA, 4u },
            { "matvec",         G5_SASS_VA,  5u },
            { "matvec_q4k",     G7_SASS_VA,  5u },
            { "matvec_q80",     G8_SASS_VA,  5u },
            { "matmul",         G9_SASS_VA,  6u },
            { "softmax_rows",   G10_SASS_VA, 3u },
            { "layernorm_rows", G11_SASS_VA, 6u },
        };
        struct gsp_kernel *destinos[7] = {
            &cp->saxpy, &cp->matvec, &cp->matvec_q4k, &cp->matvec_q80,
            &cp->matmul, &cp->softmax_rows, &cp->layernorm_rows,
        };
        unsigned n = (unsigned)(sizeof(tabla) / sizeof(tabla[0]));
        unsigned i;

        {
            static const uint32_t cand[] = {
                BLACKWELL_COMPUTE_B, BLACKWELL_COMPUTE_A, HOPPER_COMPUTE_A,
                ADA_COMPUTE_A, AMPERE_COMPUTE_B, AMPERE_COMPUTE_A,
            };

            cp->cls = gsp_rm_class_pick("compute", cand,
                                        (unsigned)(sizeof(cand) / sizeof(cand[0])));
        }
        cp->family = gsp_family_from_class(cp->cls);
        if (cp->family == GSP_FAM_DESCONOCIDA) {
            cp->family = gsp_family_from_nv((int)gsp_nv_family_current());
        }
        cp->caps = gsp_family_caps_of(cp->family);
        cp->sass = gsp_sass_pick(sets, GSP_SASS_SET_COUNT, cp->family);
        if (!cp->sass) {
            lx_printk("nouveau-lx: compute — no hay SASS para la familia %s "
                      "(cls=0x%04x); compila con SOSO_SASS_ARCHS\n",
                      cp->caps ? cp->caps->nombre : "desconocida", cp->cls);
            return -1;
        }
        lx_printk("nouveau-lx: compute — familia %s, SASS %s, QMD %s\n",
                  cp->caps ? cp->caps->nombre : "desconocida", cp->sass->arch,
                  (cp->caps && cp->caps->qmd_version)
                      ? "escribible"
                      : "SIN layout en el árbol (no se lanzará)");

        for (i = 0; i < n; i++) {
            const struct gsp_sass_variant *v =
                gsp_sass_variant_of(cp->sass, tabla[i].name);
            struct gsp_kernel *k = destinos[i];

            if (!v) {
                lx_printk("nouveau-lx: compute — el juego %s no trae %s\n",
                          cp->sass->arch, tabla[i].name);
                return -1;
            }
            k->name = v->name;
            k->arch = cp->sass->arch;
            k->sass = v->sass;
            k->sass_len = v->sass_len;
            k->regcount = v->regcount;
            k->param_base = v->param_base;
            k->param_size = v->param_size;
            k->cbank_size = v->cbank_size;
            k->param_off = v->param_off;
            k->param_count = v->param_count;
            k->sass_va = tabla[i].va;
            if (kernel_check(k, tabla[i].params) != 0) {
                return -1;
            }
        }
        cp->launch_ok = (cp->caps && cp->caps->qmd_version != 0u &&
                         str_igual(cp->sass->arch, cp->caps->sass_arch)) ? 1 : 0;
    }

    /* Cada blob tiene que caber en el hueco que va hasta la VA del siguiente. */
    {
        const struct gsp_kernel *ks[7] = { &cp->saxpy, &cp->matvec, &cp->matvec_q4k,
                                          &cp->matvec_q80, &cp->matmul,
                                          &cp->softmax_rows, &cp->layernorm_rows };
        unsigned i;

        for (i = 0; i < 7u; i++) {
            uint64_t hueco = i + 1u < 7u ? ks[i + 1]->sass_va - ks[i]->sass_va
                                         : G_SASS_SLOT;

            if ((uint64_t)ks[i]->sass_len > hueco) {
                lx_printk("nouveau-lx: compute — el blob SASS de %s (%u B) no cabe "
                          "en su hueco de %llu B\n",
                          ks[i]->name, ks[i]->sass_len,
                          (unsigned long long)hueco);
                return -1;
            }
        }
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

    /* G6: staging de x/y completos para matvec residente (W en VRAM). */
    cp->res_va = G6_RES_VA;
    if (gsp_dma_alloc(&cp->res, G6_RES_SIZE, "compute matvec resident") != 0) {
        lx_printk("nouveau-lx: compute — sin staging G6 (matvec residente irá "
                  "por tandas)\n");
    } else if (gsp_vmm_map(chan->vmm, cp->res_va, cp->res.phys, G6_RES_SIZE,
                           GSP_VMM_SYSMEM) != 0) {
        lx_printk("nouveau-lx: compute — no se pudo mapear staging G6\n");
        gsp_dma_free(&cp->res);
    } else {
        cp->res_mapped = 1;
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
              "staging G5 %s, G6 %s\n",
              cp->cls, cp->handle, chan->handle, chan->engine,
              cp->saxpy.sass_len, cp->saxpy.regcount,
              cp->matvec.sass_len, cp->matvec.regcount,
              cp->mv_mapped ? "listo" : "NO",
              cp->res_mapped ? "listo" : "NO");
    return 0;
}

static int stage_sass_kernel(struct gsp_ce *ce, struct gsp_kernel *k,
                            uint64_t scratch_va, void *scratch_cpu,
                            unsigned scratch_bytes)
{
    unsigned off;

    if (!ce || !k || !scratch_cpu || !k->sass_len || scratch_bytes == 0u) {
        return -1;
    }
    if (k->staged) {
        return 0;
    }
    for (off = 0; off < k->sass_len; off += scratch_bytes) {
        unsigned c = k->sass_len - off;

        if (c > scratch_bytes) {
            c = scratch_bytes;
        }
        memcpy(scratch_cpu, k->sass + off, c);
        if (c < scratch_bytes) {
            memset((unsigned char *)scratch_cpu + c, 0, scratch_bytes - c);
        }
        __asm__ __volatile__("mfence" ::: "memory");

        {
            unsigned ce_bytes = c;

            if (ce_bytes < GSP_CE_LINE_BYTES &&
                scratch_bytes >= GSP_CE_LINE_BYTES) {
                ce_bytes = GSP_CE_LINE_BYTES;
            }
            if (gsp_ce_copy_sync(ce, k->sass_va + off, scratch_va, ce_bytes,
                                 GSP_CE_WAIT_MS) != 0) {
                return -1;
            }
        }
    }
    k->staged = 1;
    return 0;
}

int gsp_compute_stage_sass(struct gsp_compute *cp, struct gsp_ce *ce,
                           struct gsp_kernel *k,
                           uint64_t scratch_va, void *scratch_cpu,
                           unsigned scratch_bytes)
{
    if (!cp || !cp->ready) {
        return -1;
    }
    return stage_sass_kernel(ce, k, scratch_va, scratch_cpu, scratch_bytes);
}

/* Sube a VRAM los blobs del juego que corresponde a la familia del chip,
 * antes de que exista el objeto de compute (y antes de GR0). Usa la familia
 * del chip porque el catálogo de clases aún no se ha consultado. */
int gsp_compute_stage_sass_bringup(struct gsp_ce *ce, uint64_t scratch_va,
                                   void *scratch_cpu, unsigned scratch_bytes)
{
    static const struct gsp_sass_set *const sets[] = GSP_SASS_SETS;
    static const uint64_t vas[] = {
        G4F_SASS_VA, G5_SASS_VA, G7_SASS_VA, G8_SASS_VA,
        G9_SASS_VA, G10_SASS_VA, G11_SASS_VA,
    };
    static const char *const nombres[] = {
        "saxpy", "matvec", "matvec_q4k", "matvec_q80",
        "matmul", "softmax_rows", "layernorm_rows",
    };
    unsigned familia = gsp_family_from_nv((int)gsp_nv_family_current());
    const struct gsp_sass_set *set = gsp_sass_pick(sets, GSP_SASS_SET_COUNT, familia);
    unsigned n = (unsigned)(sizeof(nombres) / sizeof(nombres[0]));
    unsigned subidos = 0;
    unsigned bytes = 0;
    unsigned i;

    if (!ce || !ce->ready || ce->stuck || !scratch_cpu || scratch_bytes == 0u) {
        return -1;
    }
    if (!set) {
        lx_printk("nouveau-lx: SASS de bring-up — sin juego para la familia %u\n",
                  familia);
        return -1;
    }
    /* Solo los cuatro primeros hacen falta en el bring-up; el resto se suben
     * cuando compute esté listo (misma función idempotente). */
    for (i = 0; i < n && i < 4u; i++) {
        const struct gsp_sass_variant *v = gsp_sass_variant_of(set, nombres[i]);
        struct gsp_kernel k;

        if (!v) {
            lx_printk("nouveau-lx: SASS de bring-up — %s no está en %s\n",
                      nombres[i], set->arch);
            return -1;
        }
        memset(&k, 0, sizeof(k));
        k.name = v->name;
        k.arch = set->arch;
        k.sass = v->sass;
        k.sass_len = v->sass_len;
        k.sass_va = vas[i];
        if (stage_sass_kernel(ce, &k, scratch_va, scratch_cpu, scratch_bytes) != 0) {
            lx_printk("nouveau-lx: SASS de bring-up — %s (%u B) no llegó a "
                      "VRAM\n", v->name, v->sass_len);
            return -1;
        }
        subidos++;
        bytes += v->sass_len;
    }
    lx_printk("nouveau-lx: SASS %s en VRAM — %u kernels, %u B\n", set->arch,
              subidos, bytes);
    return 0;
}

void gsp_compute_mark_sass_staged(struct gsp_compute *cp)
{
    if (!cp) {
        return;
    }
    cp->saxpy.staged = 1;
    cp->matvec.staged = 1;
    cp->matvec_q4k.staged = 1;
    cp->matvec_q80.staged = 1;
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

void gsp_compute_set_mv_params(struct gsp_compute *cp, const struct gsp_kernel *k,
                               uint64_t w_va, uint64_t x_va, uint64_t y_va,
                               unsigned rows, unsigned cols)
{
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

void gsp_compute_set_matmul_params(struct gsp_compute *cp, const struct gsp_kernel *k,
                                   uint64_t w_va, uint64_t x_va, uint64_t y_va,
                                   unsigned rows, unsigned cols, unsigned n)
{
    unsigned char *p = cbank_prepare(cp, k);

    *(uint64_t *)(p + k->param_off[0]) = w_va;
    *(uint64_t *)(p + k->param_off[1]) = x_va;
    *(uint64_t *)(p + k->param_off[2]) = y_va;
    *(uint32_t *)(p + k->param_off[3]) = rows;
    *(uint32_t *)(p + k->param_off[4]) = cols;
    *(uint32_t *)(p + k->param_off[5]) = n;

    __asm__ __volatile__("mfence" ::: "memory");
}

static void gsp_compute_fill_qmd_v02_grid(struct gsp_compute *cp,
                                          const struct gsp_kernel *k,
                                          GspQmdV02 *qmd, unsigned grid_x,
                                          unsigned grid_y, unsigned sem_slot)
{
    /* QMDV01_07 PROGRAM_OFFSET son 32 bits: offset relativo a GSP_VA_BASE (>>4),
     * no la VA absoluta de 40 bits que usa v05. */
    uint64_t prog_shift = (k->sass_va - GSP_VA_BASE) >> 4;
    uint64_t cb_va = cp->data_va + G4F_CBANK_OFF;
    uint64_t cb_shift = (cb_va - GSP_VA_BASE) >> 6;
    uint64_t sem_va = cp->data_va + G4F_SEM_SLOT(sem_slot);
    uint32_t cb_size = (k->cbank_size + 15u) & ~15u;

    memset(qmd, 0, sizeof(*qmd));
    qmd_set_bits(qmd->words, QMDV02_SEMAPHORE_RELEASE_ENABLE0,
                 NVA0C0_QMDV01_07_SEMAPHORE_RELEASE_ENABLE0_TRUE);
    qmd_set_bits(qmd->words, QMDV02_RELEASE_MEMBAR_TYPE,
                 NVA0C0_QMDV01_07_RELEASE_MEMBAR_TYPE_FE_SYSMEMBAR);
    qmd_set_bits(qmd->words, QMDV02_API_VISIBLE_CALL_LIMIT,
                 NVA0C0_QMDV01_07_API_VISIBLE_CALL_LIMIT_NO_CHECK);
    qmd_set_bits(qmd->words, QMDV02_QMD_MAJOR_VERSION,
                 NVA0C0_QMDV01_07_QMD_MAJOR_VERSION_V01);

    qmd_set_bits(qmd->words, QMDV02_CTA_RASTER_WIDTH, grid_x ? grid_x : 1u);
    qmd_set_bits(qmd->words, QMDV02_CTA_RASTER_HEIGHT, grid_y ? grid_y : 1u);
    qmd_set_bits(qmd->words, QMDV02_CTA_RASTER_DEPTH, 1u);
    qmd_set_bits(qmd->words, QMDV02_CTA_THREAD_DIMENSION0, G4F_CTA_THREADS);
    qmd_set_bits(qmd->words, QMDV02_CTA_THREAD_DIMENSION1, 1u);
    qmd_set_bits(qmd->words, QMDV02_CTA_THREAD_DIMENSION2, 1u);

    qmd_set_bits(qmd->words, QMDV02_PROGRAM_OFFSET, (uint32_t)prog_shift);

    qmd_set_bits(qmd->words, QMDV02_REGISTER_COUNT, k->regcount);
    qmd_set_bits(qmd->words, QMDV02_BARRIER_COUNT, 0u);
    qmd_set_bits(qmd->words, QMDV02_SHARED_MEMORY_SIZE, 0u);

    qmd_set_bits(qmd->words, QMDV02_CONSTANT_BUFFER_VALID0,
                 NVA0C0_QMDV01_07_CONSTANT_BUFFER_VALID_TRUE);
    qmd_set_bits(qmd->words, QMDV02_CONSTANT_BUFFER_INVALIDATE0,
                 NVA0C0_QMDV01_07_CONSTANT_BUFFER_INVALIDATE_TRUE);
    qmd_set_bits(qmd->words, QMDV02_CONSTANT_BUFFER_ADDR_LOWER0,
                 (uint32_t)(cb_shift & 0xffffffffu));
    qmd_set_bits(qmd->words, QMDV02_CONSTANT_BUFFER_ADDR_UPPER0,
                 (uint32_t)((cb_shift >> 32) & 0xffu));
    qmd_set_bits(qmd->words, QMDV02_CONSTANT_BUFFER_SIZE0, cb_size);

    qmd_set_bits(qmd->words, QMDV02_RELEASE0_STRUCTURE_SIZE,
                 NVA0C0_QMDV01_07_RELEASE0_STRUCTURE_SIZE_ONE_WORD);
    qmd_set_bits(qmd->words, QMDV02_RELEASE0_ADDRESS_LOWER,
                 (uint32_t)(sem_va & 0xffffffffu));
    qmd_set_bits(qmd->words, QMDV02_RELEASE0_ADDRESS_UPPER,
                 (uint32_t)((sem_va >> 32) & 0xffu));
    qmd_set_bits(qmd->words, QMDV02_RELEASE0_PAYLOAD, G4F_SEM_PAYLOAD);
    (void)cp;
}

void gsp_compute_fill_qmd_grid(struct gsp_compute *cp, const struct gsp_kernel *k,
                               GspQmdV05 *qmd, unsigned grid_x, unsigned grid_y,
                               unsigned sem_slot)
{
    if (cp && cp->caps && cp->caps->qmd_version == GSP_QMD_VERSION_AMPERE) {
        gsp_compute_fill_qmd_v02_grid(cp, k, (GspQmdV02 *)qmd, grid_x, grid_y,
                                      sem_slot);
        return;
    }

    uint64_t prog_shift = k->sass_va >> 4;
    uint64_t cb_va = cp->data_va + G4F_CBANK_OFF;
    uint64_t cb_shift = cb_va >> 6;
    uint64_t sem_va = cp->data_va + G4F_SEM_SLOT(sem_slot);
    /* SIZE_SHIFTED4 ⇒ redondeo a múltiplo de 16 B hacia arriba. */
    uint32_t cb_size = (k->cbank_size + 15u) & ~15u;

    memset(qmd, 0, sizeof(*qmd));
    qmd_set_bits(qmd->words, QMDV05_QMD_TYPE, NVCEC0_QMDV05_00_QMD_TYPE_GRID_CTA);
    qmd_set_bits(qmd->words, QMDV05_QMD_MAJOR_VERSION,
                 NVCEC0_QMDV05_00_QMD_MAJOR_VERSION_V05);
    qmd_set_bits(qmd->words, QMDV05_QMD_GROUP_ID, 0x1fu);
    qmd_set_bits(qmd->words, QMDV05_API_VISIBLE_CALL_LIMIT,
                 NVCEC0_QMDV05_00_API_VISIBLE_CALL_LIMIT_NO_CHECK);

    qmd_set_bits(qmd->words, QMDV05_GRID_WIDTH, grid_x ? grid_x : 1u);
    qmd_set_bits(qmd->words, QMDV05_GRID_HEIGHT, grid_y ? grid_y : 1u);
    qmd_set_bits(qmd->words, QMDV05_GRID_DEPTH, 1u);
    qmd_set_bits(qmd->words, QMDV05_CTA_THREAD_DIMENSION0, G4F_CTA_THREADS);
    qmd_set_bits(qmd->words, QMDV05_CTA_THREAD_DIMENSION1, 1u);
    qmd_set_bits(qmd->words, QMDV05_CTA_THREAD_DIMENSION2, 1u);

    qmd_set_bits(qmd->words, QMDV05_PROGRAM_ADDRESS_LOWER_S4,
                 (uint32_t)(prog_shift & 0xffffffffu));
    qmd_set_bits(qmd->words, QMDV05_PROGRAM_ADDRESS_UPPER_S4,
                 (uint32_t)((prog_shift >> 32) & 0x1ffffu));

    qmd_set_bits(qmd->words, QMDV05_REGISTER_COUNT, k->regcount);
    qmd_set_bits(qmd->words, QMDV05_BARRIER_COUNT, 0u);
    qmd_set_bits(qmd->words, QMDV05_SHARED_MEMORY_SIZE_S7, 0u);

    qmd_set_bits(qmd->words, QMDV05_CBANK0_ADDR_LOWER_S6,
                 (uint32_t)(cb_shift & 0xffffffffu));
    qmd_set_bits(qmd->words, QMDV05_CBANK0_ADDR_UPPER_S6,
                 (uint32_t)((cb_shift >> 32) & 0x7ffffu));
    qmd_set_bits(qmd->words, QMDV05_CBANK0_SIZE_S4, cb_size >> 4);
    qmd_set_bits(qmd->words, QMDV05_CBANK0_VALID,
                 NVCEC0_QMDV05_00_CONSTANT_BUFFER_VALID_TRUE);
    qmd_set_bits(qmd->words, QMDV05_CBANK0_INVALIDATE,
                 NVCEC0_QMDV05_00_CONSTANT_BUFFER_INVALIDATE_TRUE);

    qmd_set_bits(qmd->words, QMDV05_RELEASE_ENABLE0,
                 NVCEC0_QMDV05_00_RELEASE_ENABLE_TRUE);
    qmd_set_bits(qmd->words, QMDV05_RELEASE_STRUCTURE_SIZE0,
                 NVCEC0_QMDV05_00_RELEASE_STRUCTURE_SIZE_SEMAPHORE_ONE_WORD);
    qmd_set_bits(qmd->words, QMDV05_RELEASE_MEMBAR_TYPE0,
                 NVCEC0_QMDV05_00_RELEASE_MEMBAR_TYPE_FE_SYSMEMBAR);
    qmd_set_bits(qmd->words, QMDV05_RELEASE_SEM0_ADDR_LOWER,
                 (uint32_t)(sem_va & 0xffffffffu));
    qmd_set_bits(qmd->words, QMDV05_RELEASE_SEM0_ADDR_UPPER,
                 (uint32_t)((sem_va >> 32) & 0x1ffffffu));
    qmd_set_bits(qmd->words, QMDV05_RELEASE_SEM0_PAYLOAD_LOWER, G4F_SEM_PAYLOAD);
}

void gsp_compute_fill_qmd(struct gsp_compute *cp, const struct gsp_kernel *k,
                          GspQmdV05 *qmd, unsigned grid_x)
{
    gsp_compute_fill_qmd_grid(cp, k, qmd, grid_x, 1u, 0u);
}

int gsp_compute_encode_qmd(struct gsp_compute *cp, const GspQmdV05 *qmd,
                           unsigned *pb_off, unsigned *pb_len)
{
    struct gsp_chan *c;
    unsigned pos;
    unsigned start;
    uint64_t qmd_va;
    unsigned qmd_bytes;
    uint32_t set_object;
    uint32_t send_pcas;
    uint32_t send_sig;
    uint32_t sig_action;

    if (!cp || !cp->ready || !qmd) {
        return -1;
    }
    c = cp->chan;
    start = (unsigned)c->pb_pos;
    if (gsp_chan_pb_reserve(c, 64) < 0) {
        return -1;
    }
    pos = start;

    qmd_va = cp->data_va + G4F_QMD_OFF;
    if (cp->caps && cp->caps->qmd_version == GSP_QMD_VERSION_AMPERE) {
        qmd_bytes = (unsigned)sizeof(GspQmdV02);
        memcpy(cp_data(cp, G4F_QMD_OFF), qmd->words, qmd_bytes);
        set_object = NVC7C0_SET_OBJECT;
        send_pcas = NVC7C0_SEND_PCAS_A;
        send_sig = NVC7C0_SEND_SIGNALING_PCAS2_B;
        sig_action = NVC7C0_SEND_SIGNALING_PCAS2_B_PCAS_ACTION_INVALIDATE_COPY_SCHEDULE;
    } else {
        qmd_bytes = (unsigned)sizeof(GspQmdV05);
        memcpy(cp_data(cp, G4F_QMD_OFF), qmd->words, qmd_bytes);
        set_object = NVCEC0_SET_OBJECT;
        send_pcas = NVCEC0_SEND_PCAS_A;
        send_sig = NVCEC0_SEND_SIGNALING_PCAS2_B;
        sig_action = NVCEC0_SEND_SIGNALING_PCAS2_B_PCAS_ACTION_INVALIDATE_COPY_SCHEDULE;
    }
    __asm__ __volatile__("mfence" ::: "memory");

    cp_pb_set_object(c, &pos, GSP_COMPUTE_SUBCHANNEL, set_object, cp->cls);
    cp_pb_immd(c, &pos, 0u, NVC86F_WFI, 0u);
    cp_pb_method(c, &pos, GSP_COMPUTE_SUBCHANNEL, send_pcas, 1);
    cp_pb_write(c, &pos, (uint32_t)(qmd_va >> 8));
    cp_pb_immd(c, &pos, GSP_COMPUTE_SUBCHANNEL, send_sig, sig_action);

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
static int launch_wait_sem(struct gsp_compute *cp, const struct gsp_kernel *k,
                           unsigned grid_x, unsigned grid_y, unsigned sem_slot,
                           const char *what)
{
    GspQmdV05 qmd;
    unsigned pb_off = 0, pb_len = 0, waited;
    const volatile uint32_t *sem;
    extern uint64_t lx_ktime_get_ns(void);

    if (sem_slot >= G4F_SEM_COUNT) {
        return -1;
    }
    /* R5: nada de escribir un QMD que no es de esta familia ni de lanzar SASS
     * de otra arquitectura. Se comprueba aquí, antes del submit. */
    if (gsp_compute_launch_ready(cp, k, what) != 0) {
        return -1;
    }
    gsp_compute_fill_qmd_grid(cp, k, &qmd, grid_x, grid_y, sem_slot);
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

    sem = (const volatile uint32_t *)cp_data(cp, G4F_SEM_SLOT(sem_slot));
    {
        uint64_t t_fin = lx_ktime_get_ns() + (uint64_t)G4F_SPIN_US * 1000ull;

        do {
            __asm__ __volatile__("mfence" ::: "memory");
            if (*sem == G4F_SEM_PAYLOAD) {
                gsp_chan_ack_progress(cp->chan);
                return 0;
            }
        } while (lx_ktime_get_ns() < t_fin);
    }
    for (waited = 0; waited <= G4F_WAIT_MS; waited++) {
        __asm__ __volatile__("mfence" ::: "memory");
        if (*sem == G4F_SEM_PAYLOAD) {
            gsp_chan_ack_progress(cp->chan);
            return 0;
        }
        lx_mdelay(1);
    }
    lx_printk("nouveau-lx: %s — el QMD no señalizó en %u ms (sem=0x%08x)\n",
              what, G4F_WAIT_MS, *sem);
    gsp_chan_dump(cp->chan, what);
    if (cp->rm && cp->rm->rpc) {
        gsp_rpc_drain(cp->rm->rpc, GSP_CE_RC_DRAIN_MS);
    }
    return -1;
}

static int launch_wait(struct gsp_compute *cp, const struct gsp_kernel *k,
                       unsigned grid, const char *what)
{
    return launch_wait_sem(cp, k, grid, 1u, 0u, what);
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

    if (gsp_compute_stage_sass(cp, ce, &cp->saxpy, scratch_va, scratch_cpu,
                               G4F_STAGE_CHUNK) != 0) {
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
    if (gsp_compute_stage_sass(cp, ce, &cp->matvec, scratch_va, scratch_cpu,
                               G4F_STAGE_CHUNK) != 0) {
        lx_printk("nouveau-lx: matvec — fallo al copiar SASS a VRAM\n");
        return -1;
    }

    for (row0 = 0u; row0 < rows; row0 += per_tile) {
        unsigned n = (rows - row0) < per_tile ? (rows - row0) : per_tile;
        unsigned grid = (n + G6_ROWS_PER_CTA - 1u) / G6_ROWS_PER_CTA;

        gsp_compute_mv_stage(cp, w, x, n, cols, row0);
        gsp_compute_set_mv_params(cp, &cp->matvec, cp->mv_va + G5_MV_W_OFF,
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

int gsp_compute_matvec_resident(struct gsp_compute *cp, struct gsp_ce *ce,
                                uint64_t w_va, unsigned rows, unsigned cols,
                                const float *x, float *y,
                                uint64_t scratch_va, void *scratch_cpu)
{
    unsigned grid;
    float *gx, *gy;
    uint64_t x_va, y_va;
    extern uint64_t lx_ktime_get_ns(void);
    uint64_t t0, t1;

    if (!cp || !cp->ready || !ce || !x || !y || rows == 0u || cols == 0u ||
        w_va == 0) {
        return -1;
    }
    if (!cp->res_mapped) {
        return -1;
    }
    if (cols > G5_MAX_COLS) {
        lx_printk("nouveau-lx: matvec residente — %u columnas > tope %u\n",
                  cols, G5_MAX_COLS);
        return -1;
    }
    if ((unsigned long)cols * 4ul > G6_RES_X_BYTES) {
        return -1;
    }

    if (gsp_compute_stage_sass(cp, ce, &cp->matvec, scratch_va, scratch_cpu,
                               G4F_STAGE_CHUNK) != 0) {
        return -1;
    }

    gx = (float *)cp_res(cp, G6_RES_X_OFF);
    gy = (float *)cp_res(cp, G6_RES_Y_OFF);
    memcpy(gx, x, (unsigned long)cols * 4ul);
    x_va = cp->res_va + G6_RES_X_OFF;
    y_va = cp->res_va + G6_RES_Y_OFF;
    t0 = lx_ktime_get_ns();

    if (rows <= G6_MAX_ROWS) {
        if ((unsigned long)rows * 4ul > G6_RES_Y_BYTES) {
            return -1;
        }
        memset(gy, 0, (unsigned long)rows * 4ul);
        *(uint32_t *)cp_data(cp, G4F_SEM_OFF) = 0;
        __asm__ __volatile__("mfence" ::: "memory");

        gsp_compute_set_mv_params(cp, &cp->matvec, w_va, x_va, y_va, rows, cols);
        grid = (rows + G6_ROWS_PER_CTA - 1u) / G6_ROWS_PER_CTA;
        if (launch_wait(cp, &cp->matvec, grid, "matvec-res") != 0) {
            return -1;
        }
        __asm__ __volatile__("mfence" ::: "memory");
        memcpy(y, gy, (unsigned long)rows * 4ul);
    } else {
        unsigned row0, qmds = 0;

        for (row0 = 0u; row0 < rows; row0 += G6_MAX_ROWS) {
            unsigned n = (rows - row0) < G6_MAX_ROWS ? (rows - row0) : G6_MAX_ROWS;
            uint64_t w_chunk = w_va + (uint64_t)row0 * (uint64_t)cols * 4ull;

            if ((unsigned long)n * 4ul > G6_RES_Y_BYTES) {
                return -1;
            }
            memset(gy, 0, (unsigned long)n * 4ul);
            *(uint32_t *)cp_data(cp, G4F_SEM_OFF) = 0;
            __asm__ __volatile__("mfence" ::: "memory");

            gsp_compute_set_mv_params(cp, &cp->matvec, w_chunk, x_va, y_va, n,
                                      cols);
            grid = (n + G6_ROWS_PER_CTA - 1u) / G6_ROWS_PER_CTA;
            if (launch_wait(cp, &cp->matvec, grid, "matvec-res") != 0) {
                lx_printk("nouveau-lx: matvec residente — tanda falló "
                          "(filas %u..%u de %u)\n", row0, row0 + n, rows);
                return -1;
            }
            __asm__ __volatile__("mfence" ::: "memory");
            memcpy(y + row0, gy, (unsigned long)n * 4ul);
            qmds++;
        }
        t1 = lx_ktime_get_ns();
        (void)t0;
        (void)t1;
        (void)qmds;
        return 0;
    }
    t1 = lx_ktime_get_ns();
    (void)t0;
    (void)t1;

    return 0;
}

/* Bytes que ocupa una fila de `cols` columnas en `dtype`. Devuelve 0 si el dtype no
 * tiene kernel o `cols` no es número entero de bloques: es la aritmética que decide
 * qué trozo de VRAM lee el warp, y equivocarla no da error — da la fila del tensor
 * de al lado y un vector perfectamente creíble. */
static unsigned long q_row_bytes(unsigned dtype, unsigned cols)
{
    unsigned elems, bytes;

    switch (dtype) {
    case GSP_DTYPE_Q4_K:
        elems = Q4K_BLOCK_ELEMS;
        bytes = Q4K_BLOCK_BYTES;
        break;
    case GSP_DTYPE_Q8_0:
        elems = Q80_BLOCK_ELEMS;
        bytes = Q80_BLOCK_BYTES;
        break;
    default:
        return 0ul;
    }
    if (cols == 0u || (cols % elems) != 0u) {
        return 0ul;
    }
    return ((unsigned long)cols / elems) * bytes;
}

static struct gsp_kernel *q_kernel(struct gsp_compute *cp, unsigned dtype)
{
    switch (dtype) {
    case GSP_DTYPE_Q4_K:
        return &cp->matvec_q4k;
    case GSP_DTYPE_Q8_0:
        return &cp->matvec_q80;
    default:
        return NULL;
    }
}

int gsp_compute_matvec_q_resident(struct gsp_compute *cp, struct gsp_ce *ce,
                                  uint64_t w_va, unsigned dtype, unsigned rows,
                                  unsigned cols, const float *x, float *y,
                                  uint64_t scratch_va, void *scratch_cpu,
                                  unsigned scratch_bytes)
{
    struct gsp_kernel *k;
    unsigned grid;
    float *gx, *gy;
    uint64_t x_va, y_va;

    if (!cp || !cp->ready || !ce || !x || !y || rows == 0u || cols == 0u ||
        w_va == 0) {
        return -1;
    }
    if (!cp->res_mapped) {
        return -1;
    }
    k = q_kernel(cp, dtype);
    if (!k || k->sass_len == 0u || q_row_bytes(dtype, cols) == 0ul) {
        return -1;
    }
    /* Mismos topes que el matvec f32 residente, y por la misma razón: los fija el
     * staging de x e y, que siguen siendo f32. Lo que cambia es la MATRIZ, que no
     * pasa por el staging. */
    if (cols > G5_MAX_COLS || rows > G6_MAX_ROWS) {
        lx_printk("nouveau-lx: matvec-q residente — %ux%u fuera de topes (%u/%u)\n",
                  rows, cols, G6_MAX_ROWS, G5_MAX_COLS);
        return -1;
    }
    if ((unsigned long)cols * 4ul > G6_RES_X_BYTES ||
        (unsigned long)rows * 4ul > G6_RES_Y_BYTES) {
        return -1;
    }

    if (gsp_compute_stage_sass(cp, ce, k, scratch_va, scratch_cpu,
                               scratch_bytes) != 0) {
        return -1;
    }

    gx = (float *)cp_res(cp, G6_RES_X_OFF);
    gy = (float *)cp_res(cp, G6_RES_Y_OFF);
    memcpy(gx, x, (unsigned long)cols * 4ul);
    memset(gy, 0, (unsigned long)rows * 4ul);
    *(uint32_t *)cp_data(cp, G4F_SEM_OFF) = 0;
    __asm__ __volatile__("mfence" ::: "memory");

    x_va = cp->res_va + G6_RES_X_OFF;
    y_va = cp->res_va + G6_RES_Y_OFF;
    gsp_compute_set_mv_params(cp, k, w_va, x_va, y_va, rows, cols);

    grid = (rows + G6_ROWS_PER_CTA - 1u) / G6_ROWS_PER_CTA;
    if (launch_wait(cp, k, grid, k->name) != 0) {
        return -1;
    }

    __asm__ __volatile__("mfence" ::: "memory");
    memcpy(y, gy, (unsigned long)rows * 4ul);

    return 0;
}

int gsp_compute_matmul_resident(struct gsp_compute *cp, struct gsp_ce *ce,
                                uint64_t w_va, unsigned rows, unsigned cols,
                                unsigned n, const float *x, float *y,
                                uint64_t scratch_va, void *scratch_cpu)
{
    unsigned batch0, grid_x, grid_y, max_n;
    float *gx, *gy;
    uint64_t x_va, y_va;

    if (!cp || !cp->ready || !ce || !x || !y || rows == 0u || cols == 0u ||
        n == 0u || w_va == 0) {
        return -1;
    }
    if (!cp->res_mapped) {
        return -1;
    }
    if (cols > G5_MAX_COLS) {
        return -1;
    }
    if (gsp_compute_stage_sass(cp, ce, &cp->matmul, scratch_va, scratch_cpu,
                               G4F_STAGE_CHUNK) != 0) {
        return -1;
    }

    max_n = G6_RES_X_BYTES / ((unsigned long)cols * 4ul);
    if (max_n == 0u) {
        return -1;
    }
    if (max_n > G6_RES_Y_BYTES / ((unsigned long)rows * 4ul)) {
        max_n = G6_RES_Y_BYTES / ((unsigned long)rows * 4ul);
    }
    if (max_n == 0u) {
        return -1;
    }

    gx = (float *)cp_res(cp, G6_RES_X_OFF);
    gy = (float *)cp_res(cp, G6_RES_Y_OFF);
    x_va = cp->res_va + G6_RES_X_OFF;
    y_va = cp->res_va + G6_RES_Y_OFF;

    for (batch0 = 0u; batch0 < n; batch0 += max_n) {
        unsigned bn = (n - batch0) < max_n ? (n - batch0) : max_n;

        memcpy(gx, x + (unsigned long)batch0 * cols,
               (unsigned long)bn * cols * 4ul);
        memset(gy, 0, (unsigned long)bn * rows * 4ul);
        *(uint32_t *)cp_data(cp, G4F_SEM_OFF) = 0;
        __asm__ __volatile__("mfence" ::: "memory");

        gsp_compute_set_matmul_params(cp, &cp->matmul, w_va, x_va, y_va,
                                      rows, cols, bn);
        grid_x = (rows + G6_ROWS_PER_CTA - 1u) / G6_ROWS_PER_CTA;
        grid_y = bn;
        if (launch_wait_sem(cp, &cp->matmul, grid_x, grid_y, 0u,
                            "matmul-res") != 0) {
            return -1;
        }
        __asm__ __volatile__("mfence" ::: "memory");
        memcpy(y + (unsigned long)batch0 * rows, gy,
               (unsigned long)bn * rows * 4ul);
    }
    return 0;
}

int gsp_compute_softmax_rows(struct gsp_compute *cp, struct gsp_ce *ce,
                             float *x, unsigned rows, unsigned cols,
                             uint64_t scratch_va, void *scratch_cpu)
{
    float *gx;
    uint64_t x_va;
    unsigned grid;

    if (!cp || !cp->ready || !ce || !x || rows == 0u || cols == 0u) {
        return -1;
    }
    if (!cp->res_mapped) {
        return -1;
    }
    if ((unsigned long)cols * 4ul > G6_RES_X_BYTES ||
        (unsigned long)rows * cols * 4ul > G6_RES_X_BYTES) {
        return -1;
    }
    if (gsp_compute_stage_sass(cp, ce, &cp->softmax_rows, scratch_va,
                               scratch_cpu, G4F_STAGE_CHUNK) != 0) {
        return -1;
    }

    gx = (float *)cp_res(cp, G6_RES_X_OFF);
    memcpy(gx, x, (unsigned long)rows * cols * 4ul);
    x_va = cp->res_va + G6_RES_X_OFF;
    *(uint32_t *)cp_data(cp, G4F_SEM_OFF) = 0;
    __asm__ __volatile__("mfence" ::: "memory");

    {
        unsigned char *p = cbank_prepare(cp, &cp->softmax_rows);
        *(uint64_t *)(p + cp->softmax_rows.param_off[0]) = x_va;
        *(uint32_t *)(p + cp->softmax_rows.param_off[1]) = rows;
        *(uint32_t *)(p + cp->softmax_rows.param_off[2]) = cols;
        __asm__ __volatile__("mfence" ::: "memory");
    }

    grid = rows;
    if (launch_wait_sem(cp, &cp->softmax_rows, grid, 1u, 0u, "softmax") != 0) {
        return -1;
    }
    __asm__ __volatile__("mfence" ::: "memory");
    memcpy(x, gx, (unsigned long)rows * cols * 4ul);
    return 0;
}

int gsp_compute_layernorm_rows(struct gsp_compute *cp, struct gsp_ce *ce,
                               float *x, const float *weight, const float *bias,
                               unsigned rows, unsigned cols, float eps,
                               uint64_t scratch_va, void *scratch_cpu)
{
    float *gx, *gw, *gb;
    uint64_t x_va, w_va, b_va;
    unsigned grid;

    if (!cp || !cp->ready || !ce || !x || !weight || !bias || rows == 0u ||
        cols == 0u) {
        return -1;
    }
    if (!cp->res_mapped) {
        return -1;
    }
    if ((unsigned long)rows * cols * 4ul + (unsigned long)cols * 8ul >
        G6_RES_X_BYTES) {
        return -1;
    }
    if (gsp_compute_stage_sass(cp, ce, &cp->layernorm_rows, scratch_va,
                               scratch_cpu, G4F_STAGE_CHUNK) != 0) {
        return -1;
    }

    gx = (float *)cp_res(cp, G6_RES_X_OFF);
    gw = gx + rows * cols;
    gb = gw + cols;
    memcpy(gx, x, (unsigned long)rows * cols * 4ul);
    memcpy(gw, weight, (unsigned long)cols * 4ul);
    memcpy(gb, bias, (unsigned long)cols * 4ul);
    x_va = cp->res_va + G6_RES_X_OFF;
    w_va = cp->res_va + (unsigned long)rows * cols * 4ul;
    b_va = w_va + (unsigned long)cols * 4ul;
    *(uint32_t *)cp_data(cp, G4F_SEM_OFF) = 0;
    __asm__ __volatile__("mfence" ::: "memory");

    {
        unsigned char *p = cbank_prepare(cp, &cp->layernorm_rows);
        *(uint64_t *)(p + cp->layernorm_rows.param_off[0]) = x_va;
        *(uint64_t *)(p + cp->layernorm_rows.param_off[1]) = w_va;
        *(uint64_t *)(p + cp->layernorm_rows.param_off[2]) = b_va;
        *(uint32_t *)(p + cp->layernorm_rows.param_off[3]) = rows;
        *(uint32_t *)(p + cp->layernorm_rows.param_off[4]) = cols;
        *(float *)(p + cp->layernorm_rows.param_off[5]) = eps;
        __asm__ __volatile__("mfence" ::: "memory");
    }

    grid = rows;
    if (launch_wait_sem(cp, &cp->layernorm_rows, grid, 1u, 0u,
                        "layernorm") != 0) {
        return -1;
    }
    __asm__ __volatile__("mfence" ::: "memory");
    memcpy(x, gx, (unsigned long)rows * cols * 4ul);
    return 0;
}

int gsp_compute_launch_enqueue(struct gsp_compute *cp, const struct gsp_kernel *k,
                               unsigned grid_x, unsigned grid_y,
                               unsigned sem_slot, const char *what)
{
    GspQmdV05 qmd;
    unsigned pb_off = 0, pb_len = 0;

    if (!cp || !k || sem_slot >= G4F_SEM_COUNT) {
        return -1;
    }
    if (gsp_compute_launch_ready(cp, k, what) != 0) {
        return -1;
    }
    *(uint32_t *)cp_data(cp, G4F_SEM_SLOT(sem_slot)) = 0;
    __asm__ __volatile__("mfence" ::: "memory");
    gsp_compute_fill_qmd_grid(cp, k, &qmd, grid_x, grid_y, sem_slot);
    if (gsp_compute_encode_qmd(cp, &qmd, &pb_off, &pb_len) != 0) {
        if (gsp_chan_pb_rewind(cp->chan) != 0 ||
            gsp_compute_encode_qmd(cp, &qmd, &pb_off, &pb_len) != 0) {
            lx_printk("nouveau-lx: %s — pushbuffer lleno\n", what);
            return -1;
        }
    }
    if (gsp_chan_submit(cp->chan, pb_off, pb_len) != 0) {
        return -1;
    }
    return (int)sem_slot;
}

int gsp_compute_wait_fence(struct gsp_compute *cp, unsigned sem_slot)
{
    const volatile uint32_t *sem;
    unsigned waited;
    extern uint64_t lx_ktime_get_ns(void);

    if (!cp || sem_slot >= G4F_SEM_COUNT) {
        return -1;
    }
    sem = (const volatile uint32_t *)cp_data(cp, G4F_SEM_SLOT(sem_slot));
    {
        uint64_t t_fin = lx_ktime_get_ns() + (uint64_t)G4F_SPIN_US * 1000ull;
        do {
            __asm__ __volatile__("mfence" ::: "memory");
            if (*sem == G4F_SEM_PAYLOAD) {
                gsp_chan_ack_progress(cp->chan);
                return 0;
            }
        } while (lx_ktime_get_ns() < t_fin);
    }
    for (waited = 0; waited <= G4F_WAIT_MS; waited++) {
        __asm__ __volatile__("mfence" ::: "memory");
        if (*sem == G4F_SEM_PAYLOAD) {
            gsp_chan_ack_progress(cp->chan);
            return 0;
        }
        lx_mdelay(1);
    }
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
    if (cp->mv.va) {
        gsp_dma_free(&cp->mv);
    }
    if (cp->res.va) {
        gsp_dma_free(&cp->res);
    }
    if (cp->data.va) {
        gsp_dma_free(&cp->data);
    }
    memset(cp, 0, sizeof(*cp));
}
