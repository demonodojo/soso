/* C-038: el `new` de LLVM llama a `malloc` de glibc. Ese asignador pide
 * memoria con `brk` (syscall 12, dirección absoluta). En soso el 12 es
 * `sbrk` y el incremento es un delta, así que la reserva falla y LLVM
 * informa out of memory. Esta envoltura reserva con SYS_MMAP anónimo
 * (fd = -1) y devuelve el bloque con SYS_MUNMAP.
 *
 * Los objetos pequeños salen de losas de 1 MiB reutilizables. Los grandes
 * tienen un mapeo propio. No llama a la libc.
 */
typedef unsigned long u64;

struct hdr {
    u64 base;
    u64 span;
    u64 cap;
    u64 size;
    u64 magic;
    u64 pad;
};

struct free_node {
    struct free_node *next;
};

enum { HDR = 48, NCLASS = 13, SLAB = 1024 * 1024 };

static const u64 MAGIC = 0xC038C038C038C038ul;
static const u64 CLASS[NCLASS] = {
    16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536,
};

static struct free_node *bins[NCLASS];
static u64 slab_base, slab_off, slab_end;
static int lock;

static long syscall4(long n, long a1, long a2, long a3, long a4) {
    long ret;
    register long r10 __asm__("r10") = a4;
    __asm__ volatile("syscall"
                     : "=a"(ret)
                     : "a"(n), "D"(a1), "S"(a2), "d"(a3), "r"(r10)
                     : "rcx", "r11", "memory");
    return ret;
}

static void take(void) {
    while (__sync_lock_test_and_set(&lock, 1)) {
    }
}

static void give(void) { __sync_lock_release(&lock); }

static void *mmap_anon(u64 len) {
    long r = syscall4(15, 0, (long)len, -1, 0);
    if (r <= 0) {
        return 0;
    }
    return (void *)r;
}

static void munmap_sys(u64 addr, u64 len) { syscall4(16, (long)addr, (long)len, 0, 0); }

static u64 align_up(u64 x, u64 a) { return (x + a - 1) & ~(a - 1); }

static int class_for(u64 n) {
    int i;
    for (i = 0; i < NCLASS; i++) {
        if (n <= CLASS[i]) {
            return i;
        }
    }
    return -1;
}

static void zero(void *p, u64 n) {
    unsigned char *d = p;
    u64 i;
    for (i = 0; i < n; i++) {
        d[i] = 0;
    }
}

static void copy(void *dst, const void *src, u64 n) {
    unsigned char *d = dst;
    const unsigned char *s = src;
    u64 i;
    for (i = 0; i < n; i++) {
        d[i] = s[i];
    }
}

static void *dedicated(u64 n, u64 align) {
    u64 span, base, user;
    struct hdr *h;
    void *map;
    if (align < 16) {
        align = 16;
    }
    if (n > ~(u64)0 - (u64)HDR - align) {
        return 0;
    }
    span = n + (u64)HDR + align;
    map = mmap_anon(span);
    if (!map) {
        return 0;
    }
    base = (u64)map;
    user = align_up(base + (u64)HDR, align);
    h = (struct hdr *)(user - (u64)HDR);
    h->base = base;
    h->span = span;
    h->cap = span - (user - base);
    h->size = n;
    h->magic = MAGIC;
    return (void *)user;
}

static void *slab_new(int cls) {
    u64 stride, user;
    struct hdr *h;
    void *map;
    if (bins[cls]) {
        struct free_node *node = bins[cls];
        bins[cls] = node->next;
        return node;
    }
    stride = align_up((u64)HDR + CLASS[cls], 16);
    if (slab_off + stride > slab_end) {
        map = mmap_anon(SLAB);
        if (!map) {
            return 0;
        }
        slab_base = (u64)map;
        slab_off = 0;
        slab_end = SLAB;
    }
    user = slab_base + slab_off + (u64)HDR;
    slab_off += stride;
    h = (struct hdr *)(user - (u64)HDR);
    h->base = slab_base;
    h->span = 0;
    h->cap = CLASS[cls];
    h->magic = MAGIC;
    return (void *)user;
}

static void *alloc_unlocked(u64 n, u64 align) {
    int cls;
    void *p;
    struct hdr *h;
    if (n == 0) {
        n = 1;
    }
    if (align < 16) {
        align = 16;
    }
    if ((align & (align - 1)) != 0) {
        return 0;
    }
    if (align <= 16) {
        cls = class_for(n);
        if (cls >= 0) {
            p = slab_new(cls);
            if (!p) {
                return 0;
            }
            h = (struct hdr *)((u64)p - (u64)HDR);
            h->size = n;
            h->magic = MAGIC;
            return p;
        }
    }
    return dedicated(n, align);
}

void __wrap_free(void *p);

static void free_unlocked(void *p) {
    struct hdr *h;
    int cls;
    struct free_node *node;
    if (!p) {
        return;
    }
    h = (struct hdr *)((u64)p - (u64)HDR);
    if (h->magic != MAGIC) {
        return;
    }
    h->magic = 0;
    if (h->span != 0) {
        munmap_sys(h->base, h->span);
        return;
    }
    cls = class_for(h->cap);
    if (cls < 0) {
        return;
    }
    node = p;
    node->next = bins[cls];
    bins[cls] = node;
}

void *__wrap_malloc(u64 n) {
    void *p;
    take();
    p = alloc_unlocked(n, 16);
    give();
    return p;
}

void *__wrap_calloc(u64 nm, u64 sz) {
    void *p;
    u64 n;
    if (nm && sz > ~(u64)0 / nm) {
        return 0;
    }
    n = nm * sz;
    take();
    p = alloc_unlocked(n, 16);
    if (p) {
        zero(p, n ? n : 1);
    }
    give();
    return p;
}

void *__wrap_realloc(void *p, u64 n) {
    struct hdr *h;
    void *q;
    u64 old;
    if (!p) {
        return __wrap_malloc(n);
    }
    if (n == 0) {
        __wrap_free(p);
        return 0;
    }
    take();
    h = (struct hdr *)((u64)p - (u64)HDR);
    if (h->magic != MAGIC) {
        give();
        return 0;
    }
    if (n <= h->cap) {
        h->size = n;
        give();
        return p;
    }
    old = h->size;
    q = alloc_unlocked(n, 16);
    if (q) {
        copy(q, p, old < n ? old : n);
        free_unlocked(p);
    }
    give();
    return q;
}

void __wrap_free(void *p) {
    take();
    free_unlocked(p);
    give();
}

void *__wrap_aligned_alloc(u64 align, u64 n) {
    void *p;
    take();
    p = alloc_unlocked(n, align ? align : 16);
    give();
    return p;
}

void *__wrap_memalign(u64 align, u64 n) { return __wrap_aligned_alloc(align, n); }

int __wrap_posix_memalign(void **out, u64 align, u64 n) {
    void *p;
    if (!out || align < sizeof(void *) || (align & (align - 1)) != 0) {
        return 22;
    }
    take();
    p = alloc_unlocked(n, align);
    give();
    if (!p) {
        return 12;
    }
    *out = p;
    return 0;
}

void *__wrap___libc_malloc(u64 n) { return __wrap_malloc(n); }
void *__wrap___libc_calloc(u64 nm, u64 sz) { return __wrap_calloc(nm, sz); }
void *__wrap___libc_realloc(void *p, u64 n) { return __wrap_realloc(p, n); }
void __wrap___libc_free(void *p) { __wrap_free(p); }
void *__wrap___libc_memalign(u64 align, u64 n) { return __wrap_aligned_alloc(align, n); }
