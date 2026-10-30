#include "gsp_rom.h"
#include "gsp_mmio.h"

static unsigned g_rom_len = GSP_ROM_PROM_MAX;

void gsp_rom_init(void)
{
    g_rom_len = GSP_ROM_PROM_MAX;
}

void gsp_rom_set_limit(unsigned len)
{
    if (len > GSP_ROM_PROM_MAX) {
        len = GSP_ROM_PROM_MAX;
    }
    g_rom_len = len;
}

unsigned gsp_rom_limit(void)
{
    return g_rom_len;
}

uint8_t gsp_rom_rd8(unsigned off)
{
    unsigned word = off & ~3u;
    uint32_t v;

    if (off >= g_rom_len) {
        return 0;
    }
    v = gsp_mmio_rd32(GSP_ROM_PROM_BASE + word);
    return (uint8_t)(v >> ((off & 3u) * 8u));
}

uint16_t gsp_rom_rd16(unsigned off)
{
    return (uint16_t)gsp_rom_rd8(off) | ((uint16_t)gsp_rom_rd8(off + 1) << 8);
}

uint32_t gsp_rom_rd32(unsigned off)
{
    return (uint32_t)gsp_rom_rd8(off) |
           ((uint32_t)gsp_rom_rd8(off + 1) << 8) |
           ((uint32_t)gsp_rom_rd8(off + 2) << 16) |
           ((uint32_t)gsp_rom_rd8(off + 3) << 24);
}

int gsp_rom_match(unsigned off, const char *sig, unsigned n)
{
    unsigned i;

    for (i = 0; i < n; i++) {
        if (gsp_rom_rd8(off + i) != (uint8_t)sig[i]) {
            return 0;
        }
    }
    return 1;
}

int gsp_rom_copy(unsigned off, void *dst, unsigned len)
{
    unsigned char *d = dst;
    unsigned i;

    if (off + len > g_rom_len) {
        return -1;
    }
    for (i = 0; i < len; i++) {
        d[i] = gsp_rom_rd8(off + i);
    }
    return 0;
}
