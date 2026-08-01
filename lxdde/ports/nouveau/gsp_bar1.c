/* Recorrido de las tablas de BAR1 que construyó RM. Ver gsp_bar1.h. */
#include "gsp_bar1.h"
#include "gsp_pramin.h"

/* Geometría de los cinco niveles que puede tener un vaspace de BAR, del más
 * profundo al más alto. Es el mismo `gh100_vmm_desc_12[]` que usa `gsp_vmm.c`,
 * recortado por arriba: la raíz de BAR es PD3. */
static const struct {
    uint8_t shift;
    uint8_t bits;
    uint8_t esize;
    const char *name;
} g_lvl[GSP_BAR1_LEVELS] = {
    { 12, 9,  8, "SPT" },
    { 21, 8, 16, "PD0" },
    { 29, 9,  8, "PD1" },
    { 38, 9,  8, "PD2" },
    { 47, 9,  8, "PD3" },
};

#define BAR1_ADDR_MASK  0x000ffffffffff000ull   /* ADDRESS 51:12 */

/* 0 = INVALID, 1 = VRAM, 2 = SYSMEM_COH, 3 = SYSMEM_NONCOH (PDE). */
static const char *aperture_name(unsigned ap)
{
    switch (ap) {
    case 0u: return "INVALID";
    case 1u: return "VRAM";
    case 2u: return "SYS_COH";
    default: return "SYS_NONCOH";
    }
}

static uint64_t vram_rd64(uint64_t addr)
{
    uint64_t lo = gsp_pramin_rd32(addr);
    uint64_t hi = gsp_pramin_rd32(addr + 4u);

    return lo | (hi << 32);
}

int gsp_bar1_init(struct gsp_bar1 *b, uint64_t aperture_phys,
                  uint64_t aperture_size, uint64_t pd3_vram)
{
    if (!b) {
        return -1;
    }
    b->aperture_phys = aperture_phys;
    b->aperture_size = aperture_size;
    b->pd3 = pd3_vram;
    b->ready = 0;

    /* Una raíz a cero no es "todavía no": es que RM no la ha dado o que el
     * struct de la config estática está desplazado, y en los dos casos lo que
     * salga del recorrido sería inventado. */
    if (!pd3_vram || (pd3_vram & 0xfffull)) {
        lx_printk("nouveau-lx: BAR1 — bar1PdeBase=0x%llx no es una tabla "
                  "alineada a página\n", (unsigned long long)pd3_vram);
        return -1;
    }
    if (!aperture_phys) {
        lx_printk("nouveau-lx: BAR1 — sin apertura en el espacio de "
                  "configuración\n");
        return -1;
    }
    b->ready = 1;
    return 0;
}

int gsp_bar1_walk(const struct gsp_bar1 *b, uint64_t bar1_va,
                  struct gsp_bar1_step *out, unsigned max)
{
    uint64_t table;
    unsigned lvl;
    unsigned n = 0;

    if (!b || !b->ready || !out || !max) {
        return -1;
    }
    if (!gsp_pramin_alive()) {
        return -1;
    }

    table = b->pd3;
    for (lvl = GSP_BAR1_LEVELS; lvl-- > 0 && n < max;) {
        struct gsp_bar1_step *s = &out[n];
        uint32_t idx = (uint32_t)((bar1_va >> g_lvl[lvl].shift) &
                                  ((1u << g_lvl[lvl].bits) - 1u));
        uint64_t off = (uint64_t)idx * g_lvl[lvl].esize;
        uint64_t raw;

        /* PD0 son 16 B por entrada y la mitad que traduce páginas de 4 KiB es
         * la ALTA; la baja es la de páginas grandes, que aquí no se usan. */
        if (g_lvl[lvl].esize == 16u) {
            off += 8u;
        }
        raw = vram_rd64(table + off);

        s->level = lvl;
        s->table = table;
        s->index = idx;
        s->entry = raw;
        s->next = raw & BAR1_ADDR_MASK;
        if (lvl == 0u) {
            /* Hoja: el bit 0 es VALID de verdad y la APERTURE se codifica al
             * revés que en un PDE (VRAM = 0). Se deja el campo crudo y quien
             * lea el volcado ya sabe que en este nivel se lee distinto. */
            s->aperture = (unsigned)((raw >> 1) & 3u);
            n++;
            break;
        }
        s->aperture = (unsigned)((raw >> 1) & 3u);
        n++;
        if (s->aperture == 0u) {
            break;      /* rama sin construir: no hay nada más abajo */
        }
        if (s->aperture != 1u) {
            /* Vive en sysmem: PRAMIN no llega. Pararse aquí y decirlo es el
             * resultado — leer la tabla de abajo por una ventana de VRAM daría
             * números con buena pinta y sin ninguna relación con la realidad. */
            break;
        }
        table = s->next;
    }
    return (int)n;
}

void gsp_bar1_dump(const struct gsp_bar1 *b, uint64_t bar1_va)
{
    struct gsp_bar1_step steps[GSP_BAR1_LEVELS];
    int n, i;

    if (!b || !b->ready) {
        return;
    }
    lx_printk("nouveau-lx: BAR1 apertura=0x%llx (%llu MiB según RM) raíz PD3 en "
              "VRAM 0x%llx\n",
              (unsigned long long)b->aperture_phys,
              (unsigned long long)(b->aperture_size >> 20),
              (unsigned long long)b->pd3);

    n = gsp_bar1_walk(b, bar1_va, steps, GSP_BAR1_LEVELS);
    if (n <= 0) {
        lx_printk("nouveau-lx: BAR1 — no se pudo recorrer la tabla (PRAMIN=%d)\n",
                  gsp_pramin_alive());
        return;
    }
    for (i = 0; i < n; i++) {
        const struct gsp_bar1_step *s = &steps[i];

        lx_printk("nouveau-lx: BAR1 va=0x%llx %s[%u] @0x%llx = 0x%016llx "
                  "ap=%u (%s) → 0x%llx\n",
                  (unsigned long long)bar1_va, g_lvl[s->level].name, s->index,
                  (unsigned long long)s->table, (unsigned long long)s->entry,
                  s->aperture,
                  s->level == 0u ? "PTE, VRAM=0" : aperture_name(s->aperture),
                  (unsigned long long)s->next);
    }
    if ((unsigned)n < GSP_BAR1_LEVELS) {
        lx_printk("nouveau-lx: BAR1 — el recorrido se paró en %s: por debajo no "
                  "hay tablas que la CPU pueda leer hoy\n",
                  g_lvl[steps[n - 1].level].name);
    }
}
