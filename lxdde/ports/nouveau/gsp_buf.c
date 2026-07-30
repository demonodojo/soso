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

uint64_t gsp_buf_vram_free(const struct gsp_buf *b)
{
    if (!b || !b->vram || !b->vram->ready) {
        return 0;
    }
    return b->vram->total - b->vram->used;
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
    const unsigned char *p;
    uint64_t off, chunk;

    if (!b || !b->ready || !src || size == 0) {
        return -1;
    }
    if (!slot_find_va(va)) {
        return -1;
    }

    p = (const unsigned char *)src;
    for (off = 0; off < size; off += chunk) {
        chunk = size - off;
        if (chunk > b->scratch_bytes) {
            chunk = b->scratch_bytes;
        }
        memcpy(b->scratch_cpu, p + off, (unsigned long)chunk);
        __asm__ __volatile__("mfence" ::: "memory");
        if (gsp_ce_copy_sync(b->ce, va + off, b->scratch_va, (uint32_t)chunk,
                             GSP_CE_WAIT_MS) != 0) {
            lx_printk("nouveau-lx: G6 — subida CE falló en offset %llu\n",
                      (unsigned long long)off);
            return -1;
        }
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
