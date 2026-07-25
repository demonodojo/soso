/* Reparto de VRAM sobre las regiones utilizables. Ver gsp_vram.h. */
#include "gsp_vram.h"

void *memset(void *dst, int c, unsigned long n);

#define VRAM_PAGE 4096ull

int gsp_vram_init(struct gsp_vram *v, const struct gsp_static_info *info)
{
    unsigned i;

    if (!v || !info) {
        return -1;
    }
    memset(v, 0, sizeof(*v));

    if (!info->ready) {
        lx_printk("nouveau-lx: sin static info no hay mapa de VRAM\n");
        return -1;
    }

    for (i = 0; i < info->region_nr && i < GSP_FB_REGION_MAX; i++) {
        uint64_t base = (info->region[i].base + VRAM_PAGE - 1) & ~(VRAM_PAGE - 1);
        uint64_t end = (info->region[i].base + info->region[i].size) & ~(VRAM_PAGE - 1);

        /* Una región que empieza en 0 haría indistinguible "la primera página"
         * de "no hay sitio", que es el valor de fallo de gsp_vram_alloc. En esta
         * tarjeta no pasa, pero más vale enterarse aquí que en un PTE. */
        if (base == 0 || end <= base) {
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

    if (!v || !v->ready || size == 0) {
        return 0;
    }
    if (align < VRAM_PAGE) {
        align = VRAM_PAGE;
    }
    size = (size + VRAM_PAGE - 1) & ~(VRAM_PAGE - 1);

    for (i = 0; i < v->region_nr; i++) {
        struct gsp_vram_region *r = &v->region[i];
        uint64_t at = (r->next + align - 1) & ~(align - 1);

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
