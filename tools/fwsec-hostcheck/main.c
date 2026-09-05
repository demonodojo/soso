/* Host check: parseo VBIOS → FWSEC sin GPU. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdarg.h>

#define GFP_KERNEL 0x40u

struct lx_pci_dev;

struct gsp_dma_buf {
    void *va;
    uint64_t phys;
    unsigned long size;
};

struct falcon_lx_raw {
    const unsigned char *img;
    unsigned dma_handle;
    unsigned imem_src;
    unsigned imem_dst;
    unsigned imem_len;
    unsigned dmem_src;
    unsigned dmem_dst;
    unsigned dmem_len;
    unsigned pkc_data_offset;
    unsigned engine_id_mask;
    unsigned ucode_id;
    unsigned boot_addr;
    unsigned mbox0;
    unsigned mbox1;
    int check_mbox0;
    unsigned timeout_ms;
    const char *name;
};

static unsigned char *rom_buf;
static unsigned rom_len;
static uint16_t fake_fuse_gsp = 0x0004u;

static int lx_printk(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int n = vprintf(fmt, ap);
    va_end(ap);
    return n;
}

static void *lx_kmalloc(unsigned long size, unsigned gfp)
{
    (void)gfp;
    return malloc(size);
}

static void lx_kfree(void *p)
{
    free(p);
}

uint32_t gsp_mmio_rd32(unsigned off)
{
    if (off >= 0x008241c0u && off < 0x008241c0u + 16u * 4u) {
        unsigned idx = (off - 0x008241c0u) / 4u;
        if (idx == 1u) {
            return fake_fuse_gsp;
        }
        return 0;
    }
    if (off >= 0x00300000u) {
        unsigned roff = off - 0x00300000u;
        if (roff + 4u <= rom_len) {
            const unsigned char *p = rom_buf + roff;
            return (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
                   ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
        }
        return 0;
    }
    return 0;
}

void gsp_mmio_wr32(unsigned off, uint32_t val)
{
    (void)off;
    (void)val;
}

int gsp_mmio_alive(void)
{
    return 1;
}

int gsp_dma_alloc_copy(struct gsp_dma_buf *b, const void *src,
                       unsigned long size, const char *what)
{
    void *p;
    (void)what;
    p = malloc(size);
    if (!p) {
        return -1;
    }
    memcpy(p, src, size);
    b->va = p;
    b->phys = 0x20000000ull;
    b->size = size;
    return 0;
}

void gsp_dma_free(struct gsp_dma_buf *b)
{
    free(b->va);
    b->va = NULL;
    b->size = 0;
}

int falcon_lx_raw_boot(unsigned falcon_base, const struct falcon_lx_raw *raw)
{
    (void)falcon_base;
    (void)raw;
    return 0;
}

#define LX_FLCN_GSP_BASE 0x00110000u

#include "gsp_fwsec_body.inc"

static int load_rom(const char *path)
{
    FILE *f;
    long sz;

    f = fopen(path, "rb");
    if (!f) {
        perror(path);
        return -1;
    }
    if (fseek(f, 0, SEEK_END) != 0) {
        fclose(f);
        return -1;
    }
    sz = ftell(f);
    if (sz <= 0 || sz > 0x100000) {
        fprintf(stderr, "tamaño ROM inválido: %ld\n", sz);
        fclose(f);
        return -1;
    }
    rewind(f);
    rom_buf = malloc((size_t)sz);
    if (!rom_buf) {
        fclose(f);
        return -1;
    }
    if (fread(rom_buf, 1, (size_t)sz, f) != (size_t)sz) {
        free(rom_buf);
        fclose(f);
        return -1;
    }
    fclose(f);
    rom_len = (unsigned)sz;
    return 0;
}

int main(int argc, char **argv)
{
    const char *rom;

    if (argc < 2) {
        fprintf(stderr, "uso: %s vbios.rom\n", argv[0]);
        return 2;
    }
    rom = argv[1];
    if (load_rom(rom) != 0) {
        return 1;
    }

    printf("=== fwsec hostcheck: %s (%u bytes) ===\n", rom, rom_len);
    if (gsp_fwsec_probe(0x3ffe00000ull, 0x100000ull) != 0) {
        fprintf(stderr, "FAIL: gsp_fwsec_probe\n");
        return 1;
    }
    printf("OK: parseo + parche FWSEC-FRTS\n");
    free(rom_buf);
    return 0;
}
