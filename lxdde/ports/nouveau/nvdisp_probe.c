/* N1 — lectura VBIOS (PROM), tabla DCB y log de conectores (sin link training). */
#include "nvdisp_probe.h"
#include "gsp_rom.h"
#include "gsp_mmio.h"
#include "lx_emul.h"

#define DCB_ENTRY_CONNECTOR 0x04u

static int g_edid_ok;

typedef struct {
    const uint8_t *buf;
    unsigned len;
} rom_mem;

static uint8_t mem_rd8(unsigned off, void *ctx)
{
    rom_mem *m = ctx;

    if (!m || !m->buf || off >= m->len) {
        return 0;
    }
    return m->buf[off];
}

static uint8_t hw_rd8(unsigned off, void *ctx)
{
    (void)ctx;
    return gsp_rom_rd8(off);
}

static int rom_match_fn(unsigned off, const char *sig, unsigned n,
                      uint8_t (*rd8)(unsigned, void *), void *ctx)
{
    unsigned i;

    for (i = 0; i < n; i++) {
        if (rd8(off + i, ctx) != (uint8_t)sig[i]) {
            return 0;
        }
    }
    return 1;
}

static const char *connector_name(uint8_t ctype)
{
    switch (ctype) {
    case 0x00:
        return "VGA";
    case 0x06:
        return "HDMI-A";
    case 0x08:
        return "DisplayPort";
    case 0x0d:
        return "LVDS";
    case 0x0e:
        return "eDP";
    default:
        return "desconocido";
    }
}

static int find_bit(unsigned *bit_off, uint8_t (*rd8)(unsigned, void *), void *ctx)
{
    unsigned off;

    for (off = 0; off + 8u < 0x20000u; off += 4u) {
        if (rom_match_fn(off, "BIT", 3, rd8, ctx)) {
            *bit_off = off;
            return 0;
        }
    }
    return -1;
}

static int bit_find_dcb(unsigned bit_off, unsigned *dcb_off, unsigned *dcb_len,
                        uint8_t (*rd8)(unsigned, void *), void *ctx)
{
    unsigned hdr;
    unsigned tok_count;
    unsigned tok_size;
    unsigned i;

    hdr = (unsigned)rd8(bit_off + 4, ctx) | ((unsigned)rd8(bit_off + 5, ctx) << 8);
    if (hdr < 8u) {
        return -1;
    }
    tok_count = rd8(bit_off + 6, ctx);
    tok_size = rd8(bit_off + 7, ctx);
    if (tok_size < 4u) {
        return -1;
    }
    for (i = 0; i < tok_count; i++) {
        unsigned t = bit_off + hdr + i * tok_size;
        char id = (char)rd8(t, ctx);
        unsigned len = (unsigned)rd8(t + 2, ctx) | ((unsigned)rd8(t + 3, ctx) << 8);

        if (id != 'D') {
            continue;
        }
        if (len < 4u) {
            return -1;
        }
        *dcb_off = t + 4u;
        *dcb_len = len;
        return 0;
    }
    return -1;
}

static int edid_try_connector(unsigned idx, uint8_t (*rd8)(unsigned, void *), void *ctx)
{
    (void)idx;
    (void)rd8;
    (void)ctx;
    /* N1: sin motor I2C/DDC; N3+ leerá EDID por aux. */
    return -1;
}

static int parse_dcb(unsigned dcb_off, unsigned dcb_len,
                     uint8_t (*rd8)(unsigned, void *), void *ctx)
{
    unsigned ver;
    unsigned hlen;
    unsigned count;
    unsigned esize;
    unsigned i;
    int connectors = 0;

    if (dcb_len < 4u) {
        return -1;
    }
    ver = rd8(dcb_off, ctx);
    hlen = rd8(dcb_off + 1, ctx);
    count = rd8(dcb_off + 2, ctx);
    esize = rd8(dcb_off + 3, ctx);
    if (hlen < 4u || esize < 2u || hlen + count * esize > dcb_len) {
        lx_printk("NVDisplay: DCB inválido ver=0x%02x hlen=%u count=%u esize=%u\n",
                  ver, hlen, count, esize);
        return -1;
    }
    lx_printk("NVDisplay: DCB ver=0x%02x entradas=%u\n", ver, count);
    for (i = 0; i < count; i++) {
        unsigned e = dcb_off + hlen + i * esize;
        uint8_t etype = rd8(e, ctx);
        uint8_t ctype;

        if (etype != DCB_ENTRY_CONNECTOR) {
            continue;
        }
        ctype = rd8(e + 1, ctx);
        connectors++;
        if (edid_try_connector(i, rd8, ctx) == 0) {
            lx_printk("NVDisplay: conector %s idx=%u EDID ok\n",
                      connector_name(ctype), i);
            g_edid_ok = 1;
        } else {
            lx_printk("NVDisplay: conector %s idx=%u EDID ENXIO (sin DDC N1)\n",
                      connector_name(ctype), i);
        }
    }
    if (connectors == 0) {
        lx_printk("NVDisplay: DCB sin entradas CONNECTOR\n");
        return -1;
    }
    return 0;
}

static int probe_reader(uint8_t (*rd8)(unsigned, void *), void *ctx)
{
    unsigned bit_off;
    unsigned dcb_off;
    unsigned dcb_len;

    g_edid_ok = 0;
    if (rd8(0, ctx) != 0x55u || rd8(1, ctx) != 0xaau) {
        lx_printk("NVDisplay: ROM sin firma 0x55AA\n");
        return -1;
    }
    if (find_bit(&bit_off, rd8, ctx) != 0) {
        lx_printk("NVDisplay: tabla BIT no encontrada\n");
        return -1;
    }
    if (bit_find_dcb(bit_off, &dcb_off, &dcb_len, rd8, ctx) != 0) {
        lx_printk("NVDisplay: entrada BIT 'D' (DCB) no encontrada\n");
        return -1;
    }
    return parse_dcb(dcb_off, dcb_len, rd8, ctx);
}

int nvdisp_probe_edid_ok(void)
{
    return g_edid_ok;
}

int nvdisp_probe(void)
{
    if (!gsp_mmio_alive()) {
        lx_printk("NVDisplay: BAR0 inactivo\n");
        return -1;
    }
    gsp_rom_init();
    return probe_reader(hw_rd8, NULL);
}

int nvdisp_probe_selftest(void)
{
    static const uint8_t rom[] = {
        0x55, 0xaa, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        /* BIT @ 0x10: hdr_size=12 → tokens @ 0x1c */
        'B', 'I', 'T', 0x01, 0x0c, 0x00, 0x01, 0x08,
        0x00, 0x00, 0x00, 0x00,
        /* token 'D' @ 0x1c: data len=12 (DCB header+entry) */
        'D', 0x00, 0x0c, 0x00,
        0x40, 0x04, 0x01, 0x08,
        DCB_ENTRY_CONNECTOR, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    };
    rom_mem m = { rom, sizeof(rom) };

    g_edid_ok = 0;
    return probe_reader(mem_rd8, &m);
}
