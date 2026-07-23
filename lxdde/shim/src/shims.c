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
