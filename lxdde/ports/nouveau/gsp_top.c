/* PTOP: topología de motores leída del chip. Ver gsp_top.h. */
#include "gsp_top.h"
#include "gsp_mmio.h"
#include "lx_emul.h"
#include "nvrm_r570.h"

void *memset(void *dst, int c, unsigned long n);

#define NV_PTOP_SCAL_NUM       0x0224fcu   /* tamaño de la tabla en 31:20 */
#define NV_PTOP_DEVICE_INFO(i) (0x022800u + (unsigned)(i) * 4u)
/* El `size` de upstream es un contador de PALABRAS de la tabla, y con tres por
 * motor un chip grande no pasa de unas decenas. 256 es tope de cordura: si el
 * registro devuelve algo mayor, lo que hemos leído no es el tamaño. */
#define GSP_TOP_WORDS_MAX      256u

static struct gsp_top_engine g_top[GSP_TOP_MAX];
static unsigned g_top_cnt;
static int g_top_ready;

static const char *top_type_name(uint8_t type)
{
    switch (type) {
    case GSP_TOP_TYPE_GR:     return "GR";
    case GSP_TOP_TYPE_SEC2:   return "SEC2";
    case GSP_TOP_TYPE_NVENC:  return "NVENC";
    case GSP_TOP_TYPE_NVDEC:  return "NVDEC";
    case GSP_TOP_TYPE_IOCTRL: return "IOCTRL";
    case GSP_TOP_TYPE_CE:     return "CE";
    case GSP_TOP_TYPE_GSP:    return "GSP";
    case GSP_TOP_TYPE_NVJPG:  return "NVJPG";
    case GSP_TOP_TYPE_OFA:    return "OFA";
    default:                  return "?";
    }
}

/* Entrada "vacía": todo a cero menos el tipo, que usa 0xff porque el 0 **es** un
 * tipo válido (GR). Estaba escrito campo a campo en dos sitios, y añadir un campo
 * al struct habría dejado uno de los dos sin actualizar. */
static void top_entry_reset(struct gsp_top_engine *e)
{
    memset(e, 0, sizeof(*e));
    e->type = 0xffu;
}

int gsp_top_probe(void)
{
    uint32_t scal;
    unsigned words;
    unsigned i;
    unsigned n = 0;
    struct gsp_top_engine cur;

    g_top_cnt = 0;
    g_top_ready = 0;

    scal = gsp_mmio_rd32(NV_PTOP_SCAL_NUM);
    if (scal == 0xffffffffu || gsp_mmio_pri_error(scal)) {
        lx_printk("nouveau-lx: PTOP no contesta (0x%06x=0x%08x)\n",
                  NV_PTOP_SCAL_NUM, scal);
        return -1;
    }
    words = scal >> 20;
    if (words == 0u || words > GSP_TOP_WORDS_MAX) {
        lx_printk("nouveau-lx: PTOP dice %u palabras (0x%06x=0x%08x) — eso no es "
                  "un tamaño\n", words, NV_PTOP_SCAL_NUM, scal);
        return -1;
    }

    /* Igual que `ga100_top_parse`: cada motor son hasta tres palabras encadenadas
     * por el bit 31, y una palabra a cero antes de empezar una entrada es un hueco
     * de la tabla, no el final. */
    top_entry_reset(&cur);
    for (i = 0; i < words; i++) {
        uint32_t data = gsp_mmio_rd32(NV_PTOP_DEVICE_INFO(i));

        if (data == 0xffffffffu || gsp_mmio_pri_error(data)) {
            lx_printk("nouveau-lx: PTOP palabra %u ilegible (0x%08x) — tabla a "
                      "medias (%u motores)\n", i, data, g_top_cnt);
            break;
        }
        if (!data && n == 0u) {
            continue;
        }
        switch (n++) {
        case 0:
            cur.type = (uint8_t)((data & 0x3f000000u) >> 24);
            cur.inst = (uint8_t)((data & 0x000f0000u) >> 16);
            cur.fault = (uint8_t)(data & 0x0000007fu);
            break;
        case 1:
            cur.addr = data & 0x00fff000u;
            cur.reset = (uint8_t)(data & 0x0000001fu);
            break;
        case 2:
            cur.runlist = data & 0x00fffc00u;
            cur.engine = (uint8_t)(data & 0x00000003u);
            break;
        default:
            break;
        }
        if (data & 0x80000000u) {
            continue;   /* siguen palabras de este mismo motor */
        }
        n = 0;
        if (g_top_cnt < GSP_TOP_MAX) {
            g_top[g_top_cnt++] = cur;
            lx_printk("nouveau-lx: PTOP %2u: %s%u (tipo 0x%02x) addr=0x%06x "
                      "runlist=0x%06x engine=%u reset=%u fault=%u\n",
                      g_top_cnt - 1u, top_type_name(cur.type), cur.inst, cur.type,
                      cur.addr, cur.runlist, cur.engine, cur.reset, cur.fault);
        }
        top_entry_reset(&cur);
    }

    if (g_top_cnt == 0u) {
        lx_printk("nouveau-lx: PTOP legible pero sin motores (%u palabras) — o el "
                  "formato no es el de ga100 en este chip\n", words);
        return -1;
    }
    g_top_ready = 1;
    lx_printk("nouveau-lx: PTOP: %u motores en %u palabras\n", g_top_cnt, words);
    return (int)g_top_cnt;
}

int gsp_top_runlist_of(uint8_t type, uint8_t inst, uint32_t *runlist,
                       uint32_t *addr)
{
    unsigned i;

    if (!g_top_ready) {
        return -1;
    }
    for (i = 0; i < g_top_cnt; i++) {
        if (g_top[i].type != type || g_top[i].inst != inst) {
            continue;
        }
        if (runlist) {
            *runlist = g_top[i].runlist;
        }
        if (addr) {
            *addr = g_top[i].addr;
        }
        return 0;
    }
    return -1;
}

unsigned gsp_top_falcon_base(uint8_t type, uint8_t inst, unsigned fallback)
{
    uint32_t addr = 0;

    if (gsp_top_runlist_of(type, inst, NULL, &addr) == 0 && addr) {
        return addr;
    }
    return fallback;
}

uint32_t gsp_top_pick_ce_engine(void)
{
    uint32_t gr_runl = 0;
    unsigned inst;

    if (gsp_top_runlist_of(GSP_TOP_TYPE_GR, 0, &gr_runl, NULL) != 0) {
        return NV2080_ENGINE_TYPE_COPY2;
    }
    for (inst = 0; inst < 16u; inst++) {
        uint32_t ce_runl = 0;

        if (gsp_top_runlist_of(GSP_TOP_TYPE_CE, (uint8_t)inst, &ce_runl, NULL) != 0) {
            continue;
        }
        if (ce_runl != 0 && ce_runl != gr_runl) {
            return NV2080_ENGINE_TYPE_COPY0 + inst;
        }
    }
    return NV2080_ENGINE_TYPE_COPY2;
}

int gsp_top_type_of_engine(uint32_t engine, uint8_t *type, uint8_t *inst)
{
    /* Sólo los dos que usa el port. Un `default` que devolviera 0 sería peor que
     * fallar: el 0 es GR, así que un motor desconocido acabaría mirando los
     * registros del gráfico. */
    if (engine == NV2080_ENGINE_TYPE_GR0) {
        if (type) {
            *type = GSP_TOP_TYPE_GR;
        }
        if (inst) {
            *inst = 0;
        }
        return 0;
    }
    if (engine >= NV2080_ENGINE_TYPE_COPY0 &&
        engine < NV2080_ENGINE_TYPE_COPY0 + 16u) {
        if (type) {
            *type = GSP_TOP_TYPE_CE;
        }
        if (inst) {
            *inst = (uint8_t)(engine - NV2080_ENGINE_TYPE_COPY0);
        }
        return 0;
    }
    return -1;
}

int gsp_top_pmc_enable_mask(uint8_t type, uint8_t inst, uint32_t *mask_out)
{
    unsigned i;

    if (!g_top_ready || !mask_out) {
        return -1;
    }
    for (i = 0; i < g_top_cnt; i++) {
        if (g_top[i].type != type || g_top[i].inst != inst) {
            continue;
        }
        *mask_out = 1u << g_top[i].reset;
        return 0;
    }
    return -1;
}

#define NV_PMC_BOOT_0   0x000200u
#define NV_PMC_ENABLE   0x000600u

void gsp_mc_device_enable(uint32_t mask)
{
    uint32_t cur;

    if (!mask) {
        return;
    }
    cur = gsp_mmio_rd32(NV_PMC_ENABLE);
    gsp_mmio_wr32(NV_PMC_ENABLE, cur | mask);
    (void)gsp_mmio_rd32(NV_PMC_ENABLE);
    (void)gsp_mmio_rd32(NV_PMC_ENABLE);
}

void gsp_mc_device_disable(uint32_t mask)
{
    uint32_t cur;

    if (!mask) {
        return;
    }
    cur = gsp_mmio_rd32(NV_PMC_ENABLE);
    gsp_mmio_wr32(NV_PMC_ENABLE, cur & ~mask);
    (void)gsp_mmio_rd32(NV_PMC_ENABLE);
    (void)gsp_mmio_rd32(NV_PMC_ENABLE);
}

void gsp_mc_init_ampere(void)
{
    static int done;

    if (done) {
        return;
    }
    gsp_mmio_wr32(NV_PMC_BOOT_0, 0xffffffffu);
    gsp_mmio_wr32(NV_PMC_ENABLE, 0xffffffffu);
    (void)gsp_mmio_rd32(NV_PMC_ENABLE);
    done = 1;
}

void gsp_mc_engine_reset(uint8_t type, uint8_t inst)
{
    uint32_t mask = 0;
    uint32_t pmc;

    if (gsp_top_pmc_enable_mask(type, inst, &mask) != 0 || !mask) {
        return;
    }
    pmc = gsp_mmio_rd32(NV_PMC_ENABLE);
    lx_printk("nouveau-lx: PMC reset type=%u inst=%u enable=0x%08x mask=0x%x\n",
              type, inst, pmc, mask);
    gsp_mc_device_disable(mask);
    lx_mdelay(1);
    gsp_mc_device_enable(mask);
    pmc = gsp_mmio_rd32(NV_PMC_ENABLE);
    lx_printk("nouveau-lx: PMC tras reset enable=0x%08x\n", pmc);
}
