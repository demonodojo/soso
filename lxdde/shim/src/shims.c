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

static void emit_char(char c)
{
    lx_putchar((unsigned char)c);
}

static void emit_str(const char *s)
{
    if (s)
        lx_puts(s);
}

static void emit_u32(unsigned v)
{
    char buf[16];
    int i = 15;
    buf[i--] = 0;
    if (v == 0) {
        emit_char('0');
        return;
    }
    while (v && i >= 0) {
        buf[i--] = (char)('0' + (v % 10));
        v /= 10;
    }
    emit_str(buf + i + 1);
}

static void emit_hex_byte(unsigned v)
{
    static const char hex[] = "0123456789abcdef";
    emit_char(hex[(v >> 4) & 0xf]);
    emit_char(hex[v & 0xf]);
}

static void emit_hex(unsigned long v)
{
    emit_str("0x");
    int started = 0;
    for (int shift = 60; shift >= 0; shift -= 4) {
        unsigned n = (unsigned)((v >> shift) & 0xf);
        if (n || started || shift == 0) {
            emit_char("0123456789abcdef"[n]);
            started = 1;
        }
    }
}

static void emit_hex_fixed2(unsigned v)
{
    emit_hex_byte(v & 0xff);
}

int lx_vprintk(const char *fmt, va_list ap)
{
    if (!fmt)
        return 0;
    emit_str("lx: ");
    for (const char *p = fmt; *p; p++) {
        if (*p != '%' || !p[1]) {
            emit_char(*p);
            continue;
        }
        p++;
        switch (*p) {
        case 's':
            emit_str(va_arg(ap, const char *));
            break;
        case 'd':
        case 'i':
            emit_u32((unsigned)va_arg(ap, int));
            break;
        case 'u':
            emit_u32(va_arg(ap, unsigned));
            break;
        case 'x':
        case 'p':
            emit_hex(va_arg(ap, unsigned long));
            break;
        case '2':
            if (p[1] == 'x' || p[1] == 'X') {
                p++;
                emit_hex_fixed2(va_arg(ap, unsigned));
            } else {
                emit_char('?');
            }
            break;
        case '%':
            emit_char('%');
            break;
        default:
            emit_char('?');
            break;
        }
    }
    return 0;
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
        int zero = 0, width = 0, longs = 0;
        while (*p == '0' || *p == '-' || *p == '+' || *p == ' ' || *p == '#') {
            if (*p == '0')
                zero = 1;
            p++;
        }
        while (*p >= '0' && *p <= '9') {
            width = width * 10 + (*p - '0');
            p++;
        }
        while (*p == 'l' || *p == 'z' || *p == 'h') {
            if (*p == 'l')
                longs++;
            p++;
        }
        switch (*p) {
        case 's':
            sb_puts(&s, va_arg(ap, const char *));
            break;
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
