/* Banco de pruebas en host para gsp_rm.c: el mismo código, con la capa lx
 * simulada, sobre el blob real. Comprueba el parseo del ELF64 y la radix3. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdarg.h>
#include <stddef.h>

#define GFP_KERNEL 0x40u
struct lx_pci_dev;

static int lx_printk(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int n = vprintf(fmt, ap);
    va_end(ap);
    return n;
}

/* "Física" simulada: base ficticia + offset del puntero, alineada a página. */
#define FAKE_PHYS_BASE 0x100000000ull
static uint64_t lx_virt_to_phys(const void *p)
{
    return p ? FAKE_PHYS_BASE + ((uintptr_t)p & 0xffffffffull) : 0;
}

static void *lx_alloc_pages_exact(size_t size)
{
    void *p = NULL;
    if (posix_memalign(&p, 4096, (size + 4095) & ~(size_t)4095))
        return NULL;
    memset(p, 0, (size + 4095) & ~(size_t)4095);
    return p;
}
static void lx_free_pages_exact(void *p, size_t size) { (void)size; free(p); }

static void *lx_dma_alloc_coherent(struct lx_pci_dev *d, size_t size, uint64_t *dma, unsigned gfp)
{
    (void)d; (void)gfp;
    void *p = lx_alloc_pages_exact(size);
    if (p && dma)
        *dma = lx_virt_to_phys(p);
    return p;
}
static void lx_dma_free_coherent(struct lx_pci_dev *d, size_t size, void *va, uint64_t dma)
{ (void)d; (void)dma; lx_free_pages_exact(va, size); }

/* gsp_fw.h simulado. */
enum gsp_fw_kind { GSP_FW_BOOTLOADER = 0, GSP_FW_FMC, GSP_FW_UCODE, GSP_FW_BOOTER_LOAD,
                   GSP_FW_BOOTER_UNLOAD, GSP_FW_COUNT };
enum gsp_fw_chip { GSP_FW_CHIP_BLACKWELL = 0, GSP_FW_CHIP_AMPERE };
struct gsp_fw_blob { const char *path; unsigned char *data; unsigned long len;
                     unsigned gem_handle; int valid; };
static struct gsp_fw_blob g_ucode;
static const struct gsp_fw_blob *gsp_fw_get(enum gsp_fw_kind k)
{ return k == GSP_FW_UCODE ? &g_ucode : NULL; }

#include "gsp_rm_body.inc"

int main(int argc, char **argv)
{
    if (argc < 2) { fprintf(stderr, "uso: %s <gsp-570.144.bin>\n", argv[0]); return 2; }
    FILE *f = fopen(argv[1], "rb");
    if (!f) { perror("open"); return 2; }
    fseek(f, 0, SEEK_END);
    long len = ftell(f);
    fseek(f, 0, SEEK_SET);
    g_ucode.data = malloc(len);
    if (fread(g_ucode.data, 1, len, f) != (size_t)len) { perror("read"); return 2; }
    fclose(f);
    g_ucode.len = len;
    g_ucode.valid = 1;
    g_ucode.path = argv[1];
    printf("blob %ld bytes\n", len);

    /* Las dos familias: el ucode es el mismo blob y trae una firma por cada una
     * (en linux-firmware el de gb205 es un symlink al de ga102). */
    const enum gsp_fw_chip chips[] = { GSP_FW_CHIP_BLACKWELL, GSP_FW_CHIP_AMPERE };
    for (unsigned c = 0; c < 2; c++) {
        struct gsp_rm_fw fw;
        if (gsp_rm_prepare(chips[c], &fw) != 0) {
            printf("FALLO: gsp_rm_prepare chip=%u\n", chips[c]);
            return 1;
        }
        /* Comprobaciones externas al propio módulo. */
        unsigned long pages = (fw.img_len + 4095) / 4096;
        if (fw.rx3.mem[2].size != ((pages * 8 + 4095) & ~4095ul)) { printf("FALLO: tamaño hoja\n"); return 1; }
        if (fw.rx3.mem[1].size != 4096 || fw.rx3.mem[0].size != 4096) { printf("FALLO: tamaño l1/raíz\n"); return 1; }
        if ((uintptr_t)fw.img & 4095) { printf("FALLO: imagen no alineada\n"); return 1; }
        const uint64_t *l2 = fw.rx3.mem[2].va;
        for (unsigned long j = 0; j < pages; j++) {
            if (l2[j] != lx_virt_to_phys(fw.img + j * 4096)) { printf("FALLO: hoja %lu\n", j); return 1; }
        }
        /* Las entradas sobrantes de la hoja quedan a cero (página parcial). */
        for (unsigned long j = pages; j < fw.rx3.mem[2].size / 8; j++) {
            if (l2[j] != 0) { printf("FALLO: relleno hoja %lu no nulo\n", j); return 1; }
        }
        printf("OK: %lu páginas, todas las hojas cuadran\n", pages);
        gsp_rm_release(&fw);
        if (fw.img || fw.rx3.mem[0].va) { printf("FALLO: release deja punteros\n"); return 1; }
    }
    printf("OK: release limpio\n");
    return 0;
}
