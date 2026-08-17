/* G6: reparto de buffers de usuario en VRAM. Ver gsp_buf.h. */
#include "gsp_buf.h"

void *memcpy(void *dst, const void *src, unsigned long n);
void *memset(void *dst, int c, unsigned long n);

#define VRAM_PAGE 4096ull

static struct gsp_buf_slot g_slots[G6_MAX_SLOTS];

static struct gsp_buf_slot *slot_find_va(uint64_t va)
{
    unsigned i;

    for (i = 0; i < G6_MAX_SLOTS; i++) {
        if (g_slots[i].in_use && g_slots[i].va == va) {
            return &g_slots[i];
        }
    }
    return NULL;
}

static struct gsp_buf_slot *slot_alloc_entry(void)
{
    unsigned i;

    for (i = 0; i < G6_MAX_SLOTS; i++) {
        if (!g_slots[i].in_use && g_slots[i].va == 0) {
            return &g_slots[i];
        }
    }
    /* Reutilizar una entrada liberada (va != 0). */
    for (i = 0; i < G6_MAX_SLOTS; i++) {
        if (!g_slots[i].in_use) {
            return &g_slots[i];
        }
    }
    return NULL;
}

static uint64_t align_up(uint64_t v, uint64_t a)
{
    return (v + a - 1ull) & ~(a - 1ull);
}

int gsp_buf_init(struct gsp_buf *b, struct gsp_vram *vram, struct gsp_vmm *vmm,
                 struct gsp_ce *ce, uint64_t scratch_va, void *scratch_cpu,
                 unsigned scratch_bytes)
{
    if (!b || !vram || !vmm || !ce || !scratch_cpu || scratch_bytes == 0) {
        return -1;
    }
    memset(b, 0, sizeof(*b));
    b->vram = vram;
    b->vmm = vmm;
    b->ce = ce;
    b->scratch_va = scratch_va;
    b->scratch_cpu = scratch_cpu;
    b->scratch_bytes = scratch_bytes;
    b->va_next = G6_VA_BASE;
    b->ready = 1;
    return 0;
}

/* Margen de tablas de nivel alto: una ventana nueva puede necesitar además de la
 * hoja alguna tabla intermedia. Cuatro cubre bajar los cinco niveles de sobra. */
#define G6_PT_MARGEN 4u

uint64_t gsp_buf_vram_free(const struct gsp_buf *b)
{
    uint64_t pool, ventana, tablas;
    unsigned libres;

    if (!b || !b->vram || !b->vram->ready) {
        return 0;
    }
    pool = b->vram->total - b->vram->used;

    /* MENTÍA, y caro: esto devolvía sólo el pool —11 902 MiB en la GB205— cuando
     * lo que muerde antes es la ventana de VA de G6 (256 MiB) y, antes todavía, el
     * presupuesto de tablas de página: con PTEs de 4 KiB cada hoja cubre 2 MiB y
     * `GSP_VMM_MAX_PT` son 96, de las que el bring-up y la promoción del grctx ya
     * gastan ~42. El techo real son ~108 MiB. Con planos f32 eso son DOS tensores:
     * `soso-llm` creía tener 11,9 GiB, no desalojaba nada, y a partir del tercer
     * peso `GPU_ALLOC_VRAM` fallaba y las 21 capas restantes se iban a CPU
     * contadas como «sin sitio» — indistinguible de un offload híbrido legítimo. */
    ventana = b->va_next < G6_VA_LIMIT ? G6_VA_LIMIT - b->va_next : 0ull;

    libres = b->vmm && b->vmm->pt_nr < GSP_VMM_MAX_PT
                 ? GSP_VMM_MAX_PT - b->vmm->pt_nr
                 : 0u;
    libres = libres > G6_PT_MARGEN ? libres - G6_PT_MARGEN : 0u;
    tablas = (uint64_t)libres * (2ull * 1024ull * 1024ull);

    if (ventana < pool) {
        pool = ventana;
    }
    if (tablas < pool) {
        pool = tablas;
    }
    return pool;
}

uint64_t gsp_buf_alloc(struct gsp_buf *b, uint64_t size)
{
    struct gsp_buf_slot *s;
    uint64_t phys, va, need;
    unsigned i;

    if (!b || !b->ready || size == 0) {
        return 0;
    }
    need = align_up(size, VRAM_PAGE);

    /* Primero reutilizar un slot liberado del mismo tamaño. */
    for (i = 0; i < G6_MAX_SLOTS; i++) {
        if (!g_slots[i].in_use && g_slots[i].va != 0 && g_slots[i].size >= need) {
            g_slots[i].in_use = 1;
            return g_slots[i].va;
        }
    }

    s = slot_alloc_entry();
    if (!s) {
        lx_printk("nouveau-lx: G6 — sin entradas de slot (%u)\n", G6_MAX_SLOTS);
        return 0;
    }

    phys = gsp_vram_alloc(b->vram, need, VRAM_PAGE);
    if (!phys) {
        return 0;
    }

    va = align_up(b->va_next, VRAM_PAGE);
    if (va >= G6_VA_LIMIT || need > G6_VA_LIMIT - va) {
        lx_printk("nouveau-lx: G6 — ventana de VA agotada\n");
        gsp_vram_return(b->vram, phys, need);
        return 0;
    }

    if (gsp_vmm_map(b->vmm, va, phys, need, GSP_VMM_VRAM) != 0) {
        lx_printk("nouveau-lx: G6 — fallo al mapear VRAM en VA 0x%llx\n",
                  (unsigned long long)va);
        gsp_vram_return(b->vram, phys, need);
        return 0;
    }

    b->va_next = va + need;
    s->va = va;
    s->phys = phys;
    s->size = need;
    s->in_use = 1;
    return va;
}

int gsp_buf_upload(struct gsp_buf *b, uint64_t va, const void *src, uint64_t size)
{
    return gsp_buf_upload_at(b, va, 0, src, size);
}

int gsp_buf_upload_at(struct gsp_buf *b, uint64_t va, uint64_t offset,
                      const void *src, uint64_t size)
{
    const unsigned char *p;
    uint64_t off, chunk;
    struct gsp_buf_slot *s;

    if (!b || !b->ready || !src || size == 0) {
        return -1;
    }
    s = slot_find_va(va);
    if (!s) {
        return -1;
    }
    /* Que el trozo quepa lo comprueba el dueño del slot, no el llamante: escribir
     * más allá de `size` pisa el tensor de al lado en la misma VRAM y el síntoma
     * aparecería capas más tarde, en otro peso. */
    if (offset > s->size || size > s->size - offset) {
        lx_printk("nouveau-lx: G6 — subida fuera del búfer (off=%llu size=%llu de %llu)\n",
                  (unsigned long long)offset, (unsigned long long)size,
                  (unsigned long long)s->size);
        return -1;
    }
    /* Un offset a mitad de página convertiría cada tramo en una copia sin alinear
     * por un camino que no ha visto silicio (ver el redondeo de abajo). */
    if (offset & (VRAM_PAGE - 1ull)) {
        lx_printk("nouveau-lx: G6 — offset de subida sin alinear (%llu)\n",
                  (unsigned long long)offset);
        return -1;
    }
    va += offset;

    p = (const unsigned char *)src;
    for (off = 0; off < size; off += chunk) {
        chunk = size - off;
        if (chunk > b->scratch_bytes) {
            chunk = b->scratch_bytes;
        }
        /* Cada tramo o es múltiplo de página (copia multilínea, la de upstream)
         * o es el rabo final de menos de una página (una línea, que es lo único
         * que estaba probado antes). Sin este redondeo un tramo de, digamos,
         * 1 MiB + 300 B saldría por el camino de una línea de más de 4 KiB, que
         * no ha visto silicio. */
        if (chunk > VRAM_PAGE) {
            chunk &= ~(VRAM_PAGE - 1ull);
        }
        memcpy(b->scratch_cpu, p + off, (unsigned long)chunk);
        /* El rebote es memoria cacheada: sin bajar las líneas a RAM, una lectura
         * no-snoop del CE se llevaría lo que hubiera antes. `mfence` ordena pero
         * no vacía. Con el rebote UC de antes esto no hacía falta, y por eso es
         * fácil olvidarlo al agrandarlo. */
        lx_dma_flush_range(b->scratch_cpu, (unsigned long)chunk);
        __asm__ __volatile__("mfence" ::: "memory");
        if (gsp_ce_copy_sync(b->ce, va + off, b->scratch_va, (uint32_t)chunk,
                             GSP_CE_WAIT_MS) != 0) {
            /* Una sola línea: si el CE quedó stuck, los reintentos son silenciosos. */
            if (!b->ce->stuck) {
                lx_printk("nouveau-lx: G6 — subida CE falló en offset %llu\n",
                          (unsigned long long)off);
            }
            return -1;
        }
    }
    return 0;
}

int gsp_buf_upload_dma(struct gsp_buf *b, uint64_t va, uint64_t offset,
                       const uint64_t *phys, unsigned npages, unsigned src_off,
                       uint64_t size)
{
    struct gsp_buf_slot *s;
    uint64_t esperadas;
    unsigned i;

    if (!b || !b->ready || !b->vmm || !phys || npages == 0 || size == 0) {
        return -1;
    }
    /* `src_off` es lo que le sobra al origen para empezar en frontera de página.
     * NO es un lujo: el payload de un shard `.som` empieza en el byte 64 del
     * fichero, así que el puntero de CUALQUIER tensor mapeado llega aquí en +64 y
     * sin esto el camino sin copias no se disparaba nunca con pesos de verdad. */
    if (src_off >= VRAM_PAGE) {
        return -1;
    }
    if (size > G6_SRC_MAX - (uint64_t)src_off) {
        return -1;
    }
    /* Las páginas tienen que cubrir exactamente `src_off + size` (la última puede
     * ir a medias): si sobran o faltan, el mapeo y la copia dirían cosas
     * distintas. */
    esperadas = ((uint64_t)src_off + size + VRAM_PAGE - 1ull) / VRAM_PAGE;
    if ((uint64_t)npages != esperadas) {
        return -1;
    }
    s = slot_find_va(va);
    if (!s) {
        return -1;
    }
    if (offset > s->size || size > s->size - offset) {
        lx_printk("nouveau-lx: G6 — DMA fuera del búfer (off=%llu size=%llu de %llu)\n",
                  (unsigned long long)offset, (unsigned long long)size,
                  (unsigned long long)s->size);
        return -1;
    }
    /* Sólo el camino probado: destino alineado y tamaño múltiplo de página, o un
     * único rabo de menos de una página. Cualquier otra cosa se la queda el
     * llamante, que tiene el rebote. */
    if (offset & (VRAM_PAGE - 1ull)) {
        return -1;
    }
    if (size > VRAM_PAGE && (size & (VRAM_PAGE - 1ull))) {
        return -1;
    }
    for (i = 0; i < npages; i++) {
        if (phys[i] & (VRAM_PAGE - 1ull)) {
            return -1;
        }
    }

    /* Mapear el origen: son páginas del proceso, dispersas por definición, y lo
     * que las vuelve contiguas para el CE es justo esta ventana. `gsp_vmm_map_pages`
     * invalida la TLB de la MMU UNA vez al final, así que remapear la misma VA en
     * el lote siguiente sigue siendo legítimo. (Antes era una llamada —y una
     * invalidación, con su sondeo de milisegundos— por página: ver el comentario
     * de `gsp_vmm_map_pages`.)
     *
     * Al terminar la ventana NO se desmapea (no hay `gsp_vmm_unmap`, y cada lote
     * reescribe las entradas que va a usar). Consecuencia a tener presente: entre
     * dos subidas quedan páginas de un proceso mapeadas en el espacio de la GPU.
     * Ninguna copia lee de ahí sin haberlas remapeado antes, así que no es un
     * error de datos; si algún día hace falta cerrarlo del todo, el sitio es aquí
     * con un unmap de verdad. */
    if (gsp_vmm_map_pages(b->vmm, G6_SRC_VA, phys, npages, GSP_VMM_SYSMEM) != 0) {
        lx_printk("nouveau-lx: G6 — DMA: fallo al mapear el origen (%u páginas)\n",
                  npages);
        return -1;
    }

    /* Nada de `lx_dma_flush_range` aquí: no hemos escrito nosotros esas páginas.
     * Quien las escribió (el proceso) lo hizo con sus propias barreras, y el
     * kernel ha pasado por la syscall —serializante— entre medias.
     *
     * El origen es `G6_SRC_VA + src_off`, no la base de la ventana: la copia
     * multilínea del CE lleva PITCH = LINE_LENGTH = 4096 en los dos lados, que es
     * una copia contigua de `size` bytes y no exige que las direcciones estén
     * alineadas a página (ver `gsp_ce_encode_copy`). Lo que sí tiene que estar
     * alineado es el DESTINO, comprobado arriba. */
    if (gsp_ce_copy_sync(b->ce, va + offset, G6_SRC_VA + (uint64_t)src_off,
                         (uint32_t)size, GSP_CE_WAIT_MS) != 0) {
        if (!b->ce->stuck) {
            lx_printk("nouveau-lx: G6 — DMA: el CE falló (off=%llu size=%llu)\n",
                      (unsigned long long)offset, (unsigned long long)size);
        }
        return -1;
    }
    return 0;
}

int gsp_buf_free(struct gsp_buf *b, uint64_t va)
{
    struct gsp_buf_slot *s;

    if (!b || !b->ready) {
        return -1;
    }
    s = slot_find_va(va);
    if (!s) {
        return -1;
    }
    gsp_vram_return(b->vram, s->phys, s->size);
    s->in_use = 0;
    return 0;
}

void gsp_buf_fini(struct gsp_buf *b)
{
    unsigned i;

    if (!b) {
        return;
    }
    for (i = 0; i < G6_MAX_SLOTS; i++) {
        if (g_slots[i].va && b->vram && b->vram->ready) {
            if (g_slots[i].in_use) {
                gsp_vram_return(b->vram, g_slots[i].phys, g_slots[i].size);
            }
        }
        g_slots[i].va = 0;
        g_slots[i].phys = 0;
        g_slots[i].size = 0;
        g_slots[i].in_use = 0;
    }
    b->ready = 0;
}
