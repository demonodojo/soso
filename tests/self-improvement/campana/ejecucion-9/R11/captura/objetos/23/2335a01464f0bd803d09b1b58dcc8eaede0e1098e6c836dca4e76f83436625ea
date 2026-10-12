/* Reparto de VRAM sobre las regiones utilizables. Ver gsp_vram.h. */
#include "gsp_vram.h"

void *memset(void *dst, int c, unsigned long n);

#define VRAM_PAGE 4096ull
#define VRAM_FREE_MAX 512u

struct vram_free_blk {
    uint64_t phys;
    uint64_t size;
};

static struct vram_free_blk g_free[VRAM_FREE_MAX];
static unsigned g_free_nr;

static uint64_t vram_align_up(uint64_t v, uint64_t a)
{
    return (v + a - 1ull) & ~(a - 1ull);
}

static void free_insert(uint64_t phys, uint64_t size)
{
    unsigned i, j;

    if (phys == 0 || size == 0) {
        return;
    }
    size = vram_align_up(size, VRAM_PAGE);

    for (i = 0; i < g_free_nr; i++) {
        if (g_free[i].phys + g_free[i].size == phys) {
            g_free[i].size += size;
            goto coalesce;
        }
        if (phys + size == g_free[i].phys) {
            g_free[i].phys = phys;
            g_free[i].size += size;
            goto coalesce;
        }
    }
    if (g_free_nr >= VRAM_FREE_MAX) {
        lx_printk("nouveau-lx: VRAM free-list llena (%u); bloque %llu KiB perdido\n",
                  VRAM_FREE_MAX, (unsigned long long)(size >> 10));
        return;
    }
    g_free[g_free_nr].phys = phys;
    g_free[g_free_nr].size = size;
    g_free_nr++;
coalesce:
    for (;;) {
        int merged = 0;
        for (i = 0; i < g_free_nr; i++) {
            for (j = i + 1; j < g_free_nr; j++) {
                if (g_free[i].phys + g_free[i].size == g_free[j].phys) {
                    g_free[i].size += g_free[j].size;
                    g_free[j] = g_free[g_free_nr - 1];
                    g_free_nr--;
                    merged = 1;
                    break;
                }
                if (g_free[j].phys + g_free[j].size == g_free[i].phys) {
                    g_free[i].phys = g_free[j].phys;
                    g_free[i].size += g_free[j].size;
                    g_free[j] = g_free[g_free_nr - 1];
                    g_free_nr--;
                    merged = 1;
                    break;
                }
            }
            if (merged) {
                break;
            }
        }
        if (!merged) {
            break;
        }
    }
}

static uint64_t free_take(uint64_t size, uint64_t align)
{
    unsigned i, best = VRAM_FREE_MAX;
    uint64_t best_waste = ~0ull;

    if (align < VRAM_PAGE) {
        align = VRAM_PAGE;
    }
    size = vram_align_up(size, VRAM_PAGE);

    for (i = 0; i < g_free_nr; i++) {
        uint64_t at = vram_align_up(g_free[i].phys, align);
        uint64_t waste;

        if (at >= g_free[i].phys + g_free[i].size || size > g_free[i].phys + g_free[i].size - at) {
            continue;
        }
        waste = (at - g_free[i].phys) + (g_free[i].size - size - (at - g_free[i].phys));
        if (waste < best_waste) {
            best_waste = waste;
            best = i;
        }
    }
    if (best >= VRAM_FREE_MAX) {
        return 0;
    }

    {
        struct vram_free_blk *b = &g_free[best];
        uint64_t at = vram_align_up(b->phys, align);
        uint64_t tail_off = at + size - b->phys;
        uint64_t tail_sz = b->size - tail_off;

        if (at > b->phys) {
            free_insert(b->phys, at - b->phys);
        }
        if (tail_sz > 0) {
            free_insert(at + size, tail_sz);
        }
        *b = g_free[--g_free_nr];
        return at;
    }
}

int gsp_vram_init(struct gsp_vram *v, const struct gsp_static_info *info)
{
    unsigned i;

    if (!v || !info) {
        return -1;
    }
    memset(v, 0, sizeof(*v));
    g_free_nr = 0;

    if (!info->ready) {
        lx_printk("nouveau-lx: sin static info no hay mapa de VRAM\n");
        return -1;
    }

    for (i = 0; i < info->region_nr && i < GSP_FB_REGION_MAX; i++) {
        uint64_t base = (info->region[i].base + VRAM_PAGE - 1) & ~(VRAM_PAGE - 1);
        uint64_t end = (info->region[i].base + info->region[i].size) & ~(VRAM_PAGE - 1);

        /* El offset 0 es una dirección de VRAM perfectamente válida: los
         * offsets van desde el inicio del framebuffer, así que la primera
         * región utilizable empieza ahí. Lo que no puede devolverse es un 0,
         * porque `gsp_vram_alloc` lo usa como "no hay sitio". Se reserva la
         * primera página y en paz — descartar la región entera dejaba la
         * tarjeta sin un solo byte repartible (GB205, 2026-07-27: 11902 MiB
         * utilizables en una única región basada en 0, y G4d se quedó sin
         * VRAM con el GSP ya arrancado). */
        if (base == 0) {
            base = VRAM_PAGE;
        }
        if (end <= base) {
            lx_printk("nouveau-lx: región de VRAM %u inservible (0x%llx+0x%llx)\n",
                      i, (unsigned long long)info->region[i].base,
                      (unsigned long long)info->region[i].size);
            continue;
        }
        v->region[v->region_nr].base = base;
        v->region[v->region_nr].end = end;
        v->region[v->region_nr].next = base;
        v->total += end - base;
        v->region_nr++;
    }

    if (v->region_nr == 0) {
        lx_printk("nouveau-lx: ninguna región de VRAM utilizable\n");
        return -1;
    }

    v->ready = 1;
    lx_printk("nouveau-lx: VRAM repartible: %llu MiB en %u región(es)\n",
              (unsigned long long)(v->total >> 20), v->region_nr);
    return 0;
}

uint64_t gsp_vram_alloc(struct gsp_vram *v, uint64_t size, uint64_t align)
{
    unsigned i;
    uint64_t at;

    if (!v || !v->ready || size == 0) {
        return 0;
    }
    if (align < VRAM_PAGE) {
        align = VRAM_PAGE;
    }
    size = vram_align_up(size, VRAM_PAGE);

    at = free_take(size, align);
    if (at) {
        v->used += size;
        return at;
    }

    for (i = 0; i < v->region_nr; i++) {
        struct gsp_vram_region *r = &v->region[i];
        at = vram_align_up(r->next, align);

        /* `at + size` no puede desbordar: `end` viene de la static info y ya se
         * contrastó contra la VRAM leída por registro. */
        if (at >= r->end || size > r->end - at) {
            continue;
        }
        r->next = at + size;
        v->used += size;
        return at;
    }

    lx_printk("nouveau-lx: sin VRAM para %llu KiB (usados %llu de %llu MiB)\n",
              (unsigned long long)(size >> 10), (unsigned long long)(v->used >> 20),
              (unsigned long long)(v->total >> 20));
    return 0;
}

void gsp_vram_return(struct gsp_vram *v, uint64_t phys, uint64_t size)
{
    if (!v || !v->ready || phys == 0 || size == 0) {
        return;
    }
    size = vram_align_up(size, VRAM_PAGE);
    v->used = v->used > size ? v->used - size : 0;
    free_insert(phys, size);
}
