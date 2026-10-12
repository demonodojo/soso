/* Shims C — printk y trazas. */
#include "lx_emul.h"
#include <stdarg.h>

extern void lx_puts(const char *s);
extern void lx_putchar(unsigned char c);

void lx_emul_trace(const char *func)
{
    lx_printk("lxdde: stub trace: %s\n", func ? func : "?");
}

void lx_emul_trace_and_stop(const char *func)
{
    lx_emul_trace(func);
    for (;;)
        ;
}

static void emit_str(const char *s)
{
    if (s)
        lx_puts(s);
}

/* printk delega en vsnprintf (más abajo): un único formateador para toda la capa.
 *
 * Antes había aquí un formateador propio que NO entendía flags de anchura: con
 * "%08x" veía el '0', caía en default, emitía '?' y —lo grave— no consumía el
 * va_arg. Eso desincronizaba los argumentos y el siguiente "%s" sacaba un entero
 * como puntero: page fault al desreferenciar NV_PMC_BOOT_0 (0x1b5000a1) durante el
 * bring-up de la GPU, 2026-07-24. */
int vsnprintf(char *buf, unsigned long size, const char *fmt, va_list ap);

int lx_vprintk(const char *fmt, va_list ap)
{
    /* Línea de log típica de nvkm: holgado. Si se truncara, vsnprintf termina en
     * NUL dentro del buffer, nunca desborda. */
    char line[1024];

    if (!fmt)
        return 0;
    emit_str("lx: ");
    int n = vsnprintf(line, sizeof(line), fmt, ap);
    emit_str(line);
    return n;
}

int lx_printk(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int r = lx_vprintk(fmt, ap);
    va_end(ap);
    return r;
}

void *memcpy(void *dst, const void *src, unsigned long n)
{
    unsigned char *d = (unsigned char *)dst;
    const unsigned char *s = (const unsigned char *)src;
    for (unsigned long i = 0; i < n; i++)
        d[i] = s[i];
    return dst;
}

void *memset(void *dst, int c, unsigned long n)
{
    unsigned char *d = (unsigned char *)dst;
    for (unsigned long i = 0; i < n; i++)
        d[i] = (unsigned char)c;
    return dst;
}

char *strncpy(char *dst, const char *src, unsigned long n)
{
    unsigned long i;
    for (i = 0; i < n && src[i]; i++)
        dst[i] = src[i];
    for (; i < n; i++)
        dst[i] = '\0';
    return dst;
}

int strncmp(const char *a, const char *b, unsigned long n)
{
    for (unsigned long i = 0; i < n; i++) {
        unsigned char ca = (unsigned char)a[i];
        unsigned char cb = (unsigned char)b[i];
        if (ca != cb)
            return (int)ca - (int)cb;
        if (ca == '\0')
            return 0;
    }
    return 0;
}

void *memmove(void *dst, const void *src, unsigned long n)
{
    unsigned char *d = (unsigned char *)dst;
    const unsigned char *s = (const unsigned char *)src;
    if (d < s) {
        for (unsigned long i = 0; i < n; i++)
            d[i] = s[i];
    } else {
        for (unsigned long i = n; i > 0; i--)
            d[i - 1] = s[i - 1];
    }
    return dst;
}

int memcmp(const void *a, const void *b, unsigned long n)
{
    const unsigned char *p = (const unsigned char *)a;
    const unsigned char *q = (const unsigned char *)b;
    for (unsigned long i = 0; i < n; i++) {
        if (p[i] != q[i])
            return (int)p[i] - (int)q[i];
    }
    return 0;
}

/* --- Página física para nvkm (flush_page de fb): memoria real de PAGE_SIZE. --- */
extern void *lx_kzalloc(unsigned long size, unsigned flags);
extern void lx_kfree(void *ptr);
struct page { void *virt; };

struct page *alloc_page(unsigned gfp)
{
    struct page *p = (struct page *)lx_kzalloc(sizeof(*p), gfp);
    if (!p)
        return 0;
    p->virt = lx_kzalloc(4096, gfp);
    if (!p->virt) {
        lx_kfree(p);
        return 0;
    }
    return p;
}

void __free_page(struct page *p)
{
    if (p) {
        lx_kfree(p->virt);
        lx_kfree(p);
    }
}

void *page_address(const struct page *p)
{
    return p ? p->virt : 0;
}

unsigned long page_to_pfn(const struct page *p)
{
    return p ? ((unsigned long)p->virt >> 12) : 0;
}

/* --- snprintf mínimo (buffer) para nvkm: %s %d/%i %u %x/%X %p %c %%,
 *     con ancho, relleno con cero y modificadores de longitud l/ll/z. --- */
struct lx_sbuf {
    char *buf;
    unsigned long pos;
    unsigned long cap; /* incluye hueco para NUL */
};

static void sb_putc(struct lx_sbuf *s, char c)
{
    if (s->pos + 1 < s->cap)
        s->buf[s->pos] = c;
    s->pos++;
}

static void sb_puts(struct lx_sbuf *s, const char *str)
{
    if (!str)
        str = "(null)";
    while (*str)
        sb_putc(s, *str++);
}

static void sb_num(struct lx_sbuf *s, unsigned long long v, unsigned base,
                   int upper, int width, int zero, int neg)
{
    char tmp[24];
    const char *digs = upper ? "0123456789ABCDEF" : "0123456789abcdef";
    int i = 0;
    if (v == 0)
        tmp[i++] = '0';
    while (v) {
        tmp[i++] = digs[v % base];
        v /= base;
    }
    int len = i + (neg ? 1 : 0);
    char pad = zero ? '0' : ' ';
    if (!zero && neg)
        sb_putc(s, '-');
    for (; len < width; len++)
        sb_putc(s, pad);
    if (zero && neg)
        sb_putc(s, '-');
    while (i > 0)
        sb_putc(s, tmp[--i]);
}

int vsnprintf(char *buf, unsigned long size, const char *fmt, va_list ap)
{
    struct lx_sbuf s = { buf, 0, size };
    for (const char *p = fmt; p && *p; p++) {
        if (*p != '%') {
            sb_putc(&s, *p);
            continue;
        }
        p++;
        int zero = 0, width = 0, longs = 0, prec = -1;
        while (*p == '0' || *p == '-' || *p == '+' || *p == ' ' || *p == '#') {
            if (*p == '0')
                zero = 1;
            p++;
        }
        /* Anchura: dígitos o '*' (toma un int de la lista). El '*' hay que
         * consumirlo siempre, aunque no lo usemos: si no, se desincronizan los
         * argumentos restantes. */
        if (*p == '*') {
            width = va_arg(ap, int);
            if (width < 0)
                width = -width;
            p++;
        } else {
            while (*p >= '0' && *p <= '9') {
                width = width * 10 + (*p - '0');
                p++;
            }
        }
        if (*p == '.') {
            p++;
            prec = 0;
            if (*p == '*') {
                prec = va_arg(ap, int);
                p++;
            } else {
                while (*p >= '0' && *p <= '9') {
                    prec = prec * 10 + (*p - '0');
                    p++;
                }
            }
            if (prec < 0)
                prec = -1;
        }
        while (*p == 'l' || *p == 'z' || *p == 'h' || *p == 't') {
            if (*p == 'l')
                longs++;
            p++;
        }
        switch (*p) {
        case 's': {
            const char *str = va_arg(ap, const char *);
            if (prec < 0) {
                sb_puts(&s, str);
            } else {
                if (!str)
                    str = "(null)";
                for (int i = 0; i < prec && str[i]; i++)
                    sb_putc(&s, str[i]);
            }
            break;
        }
        case 'c':
            sb_putc(&s, (char)va_arg(ap, int));
            break;
        case 'd':
        case 'i': {
            long long v = longs ? va_arg(ap, long long) : va_arg(ap, int);
            int neg = v < 0;
            unsigned long long uv = neg ? (unsigned long long)(-v) : (unsigned long long)v;
            sb_num(&s, uv, 10, 0, width, zero, neg);
            break;
        }
        case 'u': {
            unsigned long long v = longs ? va_arg(ap, unsigned long long) : va_arg(ap, unsigned);
            sb_num(&s, v, 10, 0, width, zero, 0);
            break;
        }
        case 'x':
        case 'X': {
            unsigned long long v = longs ? va_arg(ap, unsigned long long) : va_arg(ap, unsigned);
            sb_num(&s, v, 16, *p == 'X', width, zero, 0);
            break;
        }
        case 'p':
            sb_puts(&s, "0x");
            sb_num(&s, (unsigned long long)(unsigned long)va_arg(ap, void *), 16, 0, width, zero, 0);
            break;
        case '%':
            sb_putc(&s, '%');
            break;
        default:
            sb_putc(&s, '%');
            if (*p)
                sb_putc(&s, *p);
            break;
        }
        if (!*p)
            break;
    }
    if (s.cap)
        s.buf[s.pos < s.cap ? s.pos : s.cap - 1] = '\0';
    return (int)s.pos;
}

int snprintf(char *buf, unsigned long size, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int r = vsnprintf(buf, size, fmt, ap);
    va_end(ap);
    return r;
}

int scnprintf(char *buf, unsigned long size, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int r = vsnprintf(buf, size, fmt, ap);
    va_end(ap);
    if (size == 0)
        return 0;
    return (unsigned long)r < size ? r : (int)size - 1;
}

/* Puertos lxdde opcionales: el kernel referencia e1000e en poll_rx aunque el
 * perfil live sólo linkee nouveau+iwlwifi. Los .o del puerto real pisan estos weak. */
__attribute__((weak)) void lx_spike_run(void) {}
__attribute__((weak)) void lx_testdrv_run(void) {}
__attribute__((weak)) int lx_e1000e_init_module(void) { return 0; }
__attribute__((weak)) void lx_e1000e_exit_module(void) {}
__attribute__((weak)) void *lx_e1000e_adapter(void) { return 0; }
__attribute__((weak)) void lx_e1000_poll(void *ad) { (void)ad; }
