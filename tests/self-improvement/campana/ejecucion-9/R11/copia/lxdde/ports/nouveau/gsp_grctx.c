/* G4f: contexto de GR. Ver gsp_grctx.h. */
#include "gsp_grctx.h"

void *memset(void *dst, int c, unsigned long n);

/* El mapa de `r535_gr_get_ctxbuf_info`, copiado entrada por entrada y en su orden.
 * `global` = upstream lo comparte entre canales (lo reserva una vez en el contexto
 * dorado); `init` = RM lo inicializa, y entonces la entrada tiene que llevar la
 * dirección física; `ro` = se mapea de sólo lectura. */
static const struct {
    uint32_t prop;
    uint32_t buffer_id;
    uint8_t global;
    uint8_t init;
    uint8_t ro;
} grctx_map[] = {
    { NV0080_CTX_PROP_GRAPHICS,                 NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_MAIN,             0, 1, 0 },
    { NV0080_CTX_PROP_GRAPHICS_PATCH,           NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PATCH,            0, 1, 0 },
    { NV0080_CTX_PROP_GRAPHICS_BUNDLE_CB,       NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_BUFFER_BUNDLE_CB, 1, 0, 0 },
    { NV0080_CTX_PROP_GRAPHICS_PAGEPOOL,        NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PAGEPOOL,         1, 0, 0 },
    { NV0080_CTX_PROP_GRAPHICS_ATTRIBUTE_CB,    NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_ATTRIBUTE_CB,     1, 0, 0 },
    { NV0080_CTX_PROP_GRAPHICS_RTV_CB_GLOBAL,   NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_RTV_CB_GLOBAL,    1, 0, 0 },
    { NV0080_CTX_PROP_GRAPHICS_FECS_EVENT,      NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_FECS_EVENT,       1, 1, 0 },
    { NV0080_CTX_PROP_GRAPHICS_PRIV_ACCESS_MAP, NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PRIV_ACCESS_MAP,  1, 1, 1 },
};

#define GRCTX_MAP_NR ((unsigned)(sizeof(grctx_map) / sizeof(grctx_map[0])))
/* FECS construye el golden context la primera vez; en silicio tardó >2 s (2026-07-29). */
#define GRCTX_PROMOTE_TIMEOUT_MS 15000u

static struct gsp_grctx g_golden_grctx;
static int g_golden_grctx_ready;

void gsp_grctx_golden_publish(const struct gsp_grctx *ctx)
{
    if (!ctx || !ctx->promoted) {
        return;
    }
    g_golden_grctx = *ctx;
    g_golden_grctx_ready = 1;
}

static const struct gsp_grctx_buf *grctx_golden_lookup(uint32_t buffer_id)
{
    unsigned i;

    if (!g_golden_grctx_ready) {
        return NULL;
    }
    for (i = 0; i < g_golden_grctx.nr; i++) {
        if (g_golden_grctx.buf[i].buffer_id == buffer_id) {
            return &g_golden_grctx.buf[i];
        }
    }
    return NULL;
}

/* Tamaño mapeado en el VMM: alineado a 2^page_shift como `nvkm_memory_size`. */
static uint64_t grctx_map_bytes(const struct gsp_grctx_buf *b)
{
    uint64_t gran = 1ull << b->page_shift;

    return (b->size + gran - 1ull) & ~(gran - 1ull);
}

static const char *grctx_buf_name(uint32_t buffer_id)
{
    switch (buffer_id) {
    case NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_MAIN:             return "MAIN";
    case NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PATCH:            return "PATCH";
    case NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_BUFFER_BUNDLE_CB: return "BUNDLE_CB";
    case NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PAGEPOOL:         return "PAGEPOOL";
    case NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_ATTRIBUTE_CB:     return "ATTRIBUTE_CB";
    case NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_RTV_CB_GLOBAL:    return "RTV_CB_GLOBAL";
    case NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_FECS_EVENT:       return "FECS_EVENT";
    case NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PRIV_ACCESS_MAP:  return "PRIV_ACCESS_MAP";
    case NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_UNRESTRICTED_PRIV_ACCESS_MAP:
        return "UNRESTRICTED_PRIV_ACCESS_MAP";
    default: return "?";
    }
}

/* `order_base_2(size)`: el exponente de la potencia de dos que **cubre** size (para
 * un size que ya es potencia de dos, su propio exponente). Upstream lo usa sólo
 * para el attribute CB, y no es lo mismo que el desplazamiento de página: en un
 * búfer de 24 MiB da 25 (32 MiB), no 21. Confundirlos alinea el búfer a 2 MiB y RM
 * no se queja hasta que el gráfico lee fuera. */
static unsigned order_base_2_u64(uint64_t v)
{
    unsigned n = 0;

    if (v == 0u) {
        return 0;
    }
    v--;
    while (v) {
        v >>= 1;
        n++;
    }
    return n;
}

int gsp_grctx_plan(const NV2080_CTRL_INTERNAL_STATIC_GR_GET_CONTEXT_BUFFERS_INFO_PARAMS *info,
                   unsigned engine_idx, struct gsp_grctx *ctx)
{
    unsigned i;

    if (!info || !ctx || engine_idx >= NV2080_CTRL_INTERNAL_GR_MAX_ENGINES) {
        return -1;
    }
    memset(ctx, 0, sizeof(*ctx));

    for (i = 0; i < GRCTX_MAP_NR; i++) {
        const NV2080_CTRL_INTERNAL_ENGINE_CONTEXT_BUFFER_INFO *bi =
            &info->engineContextBuffersInfo[engine_idx].engine[grctx_map[i].prop];
        struct gsp_grctx_buf *b;
        uint64_t size = bi->size;

        /* Un tamaño a cero significa que este chip no usa ese búfer: se salta, no
         * se reserva una página "por si acaso" que RM luego no espera. */
        if (size == 0u) {
            continue;
        }
        if (ctx->nr >= GSP_GRCTX_MAX) {
            lx_printk("nouveau-lx: grctx: más búferes que sitio (%u)\n", GSP_GRCTX_MAX);
            return -1;
        }
        b = &ctx->buf[ctx->nr];

        /* El principal se lleva 64 páginas de más sobre su tamaño redondeado. Es de
         * upstream tal cual (`ALIGN(size, 0x1000) + 64 * 0x1000`) y no está
         * explicado allí; lo que sí se sabe es que sin ese margen RM escribe fuera. */
        if (grctx_map[i].buffer_id == NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_MAIN) {
            size = align_up_u64(size, 0x1000ull) + 64ull * 0x1000ull;
        }

        if (size >= (1ull << 21)) {
            b->page_shift = 21;
        } else if (size >= (1ull << 16)) {
            b->page_shift = 16;
        } else {
            b->page_shift = 12;
        }
        if (grctx_map[i].buffer_id == NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_ATTRIBUTE_CB) {
            b->align = 1ull << order_base_2_u64(size);
        } else {
            b->align = 1ull << b->page_shift;
        }

        b->buffer_id = grctx_map[i].buffer_id;
        b->prop_id = grctx_map[i].prop;
        b->size = size;
        b->global = grctx_map[i].global;
        b->init = grctx_map[i].init;
        b->ro = grctx_map[i].ro;
        ctx->nr++;

        /* El mapa de acceso privilegiado se promociona DOS veces: una con su
         * bufferId y otra como "sin restricciones", con el mismo tamaño. Upstream
         * duplica la entrada aquí mismo, y en la promoción la segunda sólo viaja
         * cuando se reserva memoria nueva. */
        if (grctx_map[i].buffer_id == NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PRIV_ACCESS_MAP) {
            if (ctx->nr >= GSP_GRCTX_MAX) {
                lx_printk("nouveau-lx: grctx: sin sitio para el duplicado del "
                          "PRIV_ACCESS_MAP\n");
                return -1;
            }
            ctx->buf[ctx->nr] = *b;
            ctx->buf[ctx->nr].buffer_id =
                NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_UNRESTRICTED_PRIV_ACCESS_MAP;
            ctx->nr++;
        }
    }

    if (ctx->nr == 0u) {
        lx_printk("nouveau-lx: grctx: RM no da tamaño para ningún búfer de "
                  "contexto — la respuesta no es la que creemos\n");
        return -1;
    }
    ctx->planned = 1;
    for (i = 0; i < ctx->nr; i++) {
        const struct gsp_grctx_buf *b = &ctx->buf[i];

        lx_printk("nouveau-lx: grctx %2u: %s (id %u, prop 0x%02x) %llu KiB "
                  "página 2^%u alineación 0x%llx%s%s%s\n", i,
                  grctx_buf_name(b->buffer_id), b->buffer_id, b->prop_id,
                  (unsigned long long)(b->size / 1024ull), b->page_shift,
                  (unsigned long long)b->align, b->global ? " global" : "",
                  b->init ? " init" : "", b->ro ? " ro" : "");
    }
    return (int)ctx->nr;
}

int gsp_grctx_query(struct gsp_rm *rm, unsigned engine_idx, struct gsp_grctx *ctx)
{
    NV2080_CTRL_INTERNAL_STATIC_GR_GET_CONTEXT_BUFFERS_INFO_PARAMS *info;
    uint32_t status = 0;
    int ret;

    if (!rm || !rm->ready || !ctx) {
        return -1;
    }
    /* 1664 B no van en la pila del bring-up. */
    info = lx_kzalloc(sizeof(*info), GFP_KERNEL);
    if (!info) {
        lx_printk("nouveau-lx: grctx: sin memoria para la consulta\n");
        return -1;
    }
    /* Como todas las RPC de tipo `_rd`: se manda el struct entero aunque vaya
     * vacío, porque RM lo usa de hueco donde escribir la respuesta (la regla que
     * salió del 0xff100002 de G4d). */
    if (gsp_rm_control(rm, rm->subdevice,
                       NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO,
                       info, (uint32_t)sizeof(*info), &status) != 0) {
        lx_printk("nouveau-lx: GET_CONTEXT_BUFFERS_INFO rechazado (%s, 0x%x)\n",
                  nv_status_name(status), status);
        lx_kfree(info);
        return -1;
    }
    ret = gsp_grctx_plan(info, engine_idx, ctx);
    lx_kfree(info);
    return ret;
}

int gsp_grctx_promote(struct gsp_rm *rm, struct gsp_vmm *vmm, struct gsp_vram *vram,
                      struct gsp_chan *chan, struct gsp_grctx *ctx, int golden)
{
    NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS *p;
    uint32_t status = 0;
    unsigned i;
    int ret = -1;

    if (!rm || !vmm || !vram || !chan || !ctx || !ctx->planned || !chan->ready) {
        return -1;
    }
    p = lx_kzalloc(sizeof(*p), GFP_KERNEL);
    if (!p) {
        lx_printk("nouveau-lx: grctx: sin memoria para la promoción\n");
        return -1;
    }

    /* `engineType` es el 1 literal de la promoción —el GR— y no el engineType del
     * canal: upstream escribe `ctrl->engineType = 1` tal cual. `hClient`, `ChID`,
     * `hVirtMemory`, `virtAddress` y `size` se quedan a cero, también como upstream:
     * el contexto va descrito entrada por entrada, no por ese bloque. */
    p->engineType = 1u;
    p->hChanClient = rm->client;
    p->hObject = chan->handle;

    if (!golden && !g_golden_grctx_ready) {
        lx_printk("nouveau-lx: grctx: promote sin golden y sin contexto dorado "
                  "publicado\n");
        goto out;
    }

    ctx->va_next = GSP_GRCTX_VA_BASE;
    for (i = 0; i < ctx->nr; i++) {
        struct gsp_grctx_buf *b = &ctx->buf[i];
        NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY *e = &p->promoteEntry[p->entryCount];
        const struct gsp_grctx_buf *gb;
        int alloc;
        uint64_t va;

        if (p->entryCount >= NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES) {
            lx_printk("nouveau-lx: grctx: más entradas que las %u del control\n",
                      NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES);
            goto out;
        }

        /* `r535_gr_promote_ctx`: el segundo canal hereda globales y omite el
         * UNRESTRICTED_PRIV_ACCESS_MAP. */
        if (!golden &&
            b->buffer_id ==
                NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_UNRESTRICTED_PRIV_ACCESS_MAP) {
            continue;
        }

        alloc = golden || !b->global;
        if (alloc) {
            b->phys = gsp_vram_alloc(vram, b->size, b->align);
            if (b->phys == 0u) {
                lx_printk("nouveau-lx: grctx: sin VRAM para %s (%llu KiB, "
                          "alineación 0x%llx)\n", grctx_buf_name(b->buffer_id),
                          (unsigned long long)(b->size / 1024ull),
                          (unsigned long long)b->align);
                goto out;
            }
            ctx->vram_bytes += b->size;
        } else {
            gb = grctx_golden_lookup(b->buffer_id);
            if (!gb || gb->phys == 0u) {
                lx_printk("nouveau-lx: grctx: global %s sin golden publicado\n",
                          grctx_buf_name(b->buffer_id));
                goto out;
            }
            b->phys = gb->phys;
            /* Linux r535_gr_promote_ctx: bNonmapped=1 sólo en alloc (golden).
             * El canal de usuario reutiliza la física global pero mapea en su
             * VMM — heredar nonmapped del golden dejaba PRIV_ACCESS_MAP sin VA
             * y RM devolvía INVALID_ARGUMENT (0x1f). */
            b->nonmapped = 0;
        }

        /* El propio PRIV_ACCESS_MAP se promociona sin mapear sólo en golden
         * (upstream pone `bNonmapped` cuando lo reserva él). */
        if (alloc) {
            b->nonmapped =
                b->buffer_id ==
                NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PRIV_ACCESS_MAP;
        }

        e->bufferId = (uint16_t)b->buffer_id;
        e->bInitialize = (b->init && alloc) ? 1u : 0u;
        e->bNonmapped = b->nonmapped;

        if (!b->nonmapped) {
            uint64_t map_bytes = grctx_map_bytes(b);

            va = align_up_u64(ctx->va_next, b->align);
            if (va + map_bytes > GSP_GRCTX_VA_BASE + GSP_GRCTX_VA_SIZE) {
                lx_printk("nouveau-lx: grctx: %s no cabe en la ventana de VAs "
                          "(0x%llx + %llu KiB)\n", grctx_buf_name(b->buffer_id),
                          (unsigned long long)va,
                          (unsigned long long)(map_bytes / 1024ull));
                goto out;
            }
            /* `ro` como upstream (`.ro = ctxbuf[i].ro` en sus args de mapeo): el PCF
             * del PTE pasa a `REGULAR_RO_*`. Queda una desviación, dicha en
             * `pte_encode`: upstream mapea además con `priv = 1` y aquí el PCF es
             * REGULAR, que es más permisivo y por tanto no rompe. */
            if (gsp_vmm_map_flags(vmm, va, b->phys, map_bytes, GSP_VMM_VRAM,
                                  b->ro ? GSP_VMM_RO : 0u) != 0) {
                lx_printk("nouveau-lx: grctx: no se pudo mapear %s en 0x%llx\n",
                          grctx_buf_name(b->buffer_id), (unsigned long long)va);
                goto out;
            }
            b->va = va;
            ctx->va_next = va + map_bytes;
            e->gpuVirtAddr = va;
        }

        /* La física sólo viaja cuando RM va a inicializar el búfer, igual que
         * upstream: en los demás el campo se queda a cero y RM usa la VA. */
        if (e->bInitialize) {
            e->gpuPhysAddr = b->phys;
            e->size = b->size;
            e->physAttr = NV2080_CTRL_GPU_PROMOTE_CTX_PHYS_ATTR_DEFAULT;
        }

        /* Se imprime la física **reservada**, no la del mensaje: en las entradas que
         * RM no inicializa ese campo va a cero a propósito, y un log que enseñara el
         * campo diría "pa=0x0" de un búfer que sí tiene VRAM detrás. */
        lx_printk("nouveau-lx: grctx promote %2u: %s vram=0x%llx%s va=0x%llx "
                  "%llu KiB init=%u nm=%u\n", (unsigned)p->entryCount,
                  grctx_buf_name(b->buffer_id), (unsigned long long)b->phys,
                  e->bInitialize ? " (va en el mensaje)" : " (no viaja)",
                  (unsigned long long)e->gpuVirtAddr,
                  (unsigned long long)(b->size / 1024ull), e->bInitialize,
                  e->bNonmapped);
        p->entryCount++;
    }

    if (gsp_rm_control_timeout(rm, rm->subdevice, NV2080_CTRL_CMD_GPU_PROMOTE_CTX,
                               p, (uint32_t)sizeof(*p), &status,
                               GRCTX_PROMOTE_TIMEOUT_MS) != 0) {
        lx_printk("nouveau-lx: PROMOTE_CTX rechazado (%s, 0x%x) — el canal de GR "
                  "se queda sin contexto\n", nv_status_name(status), status);
        goto out;
    }
    ctx->promoted = 1;
    lx_printk("nouveau-lx: contexto de GR promocionado: %u entradas, %llu KiB de "
              "VRAM (canal 0x%08x)\n", (unsigned)p->entryCount,
              (unsigned long long)(ctx->vram_bytes / 1024ull), chan->handle);
    ret = 0;
out:
    lx_kfree(p);
    return ret;
}
