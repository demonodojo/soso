/* Banco de pruebas en host para gsp_rm.c y gsp_wpr.c: los mismos fuentes, con
 * la capa lx y el MMIO simulados, sobre los blobs reales. Comprueba el parseo de
 * los ELF/cabeceras NVIDIA, la tabla radix3 y el GspFwWprMeta.
 *
 * No sustituye a la prueba en hardware: valida parseo y aritmética, no arranque. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdarg.h>
#include <stddef.h>

#define GFP_KERNEL 0x40u
struct lx_pci_dev;

/* VRAM que finge el registro 0x1183a4 (en MiB), como la RTX 5070 de la máquina. */
#define FAKE_VRAM_MB 12288u

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

/* gsp_mmio.h simulado: solo hace falta el registro de tamaño de VRAM. */
static uint32_t gsp_mmio_rd32(uint32_t off)
{ return off == 0x001183a4u ? FAKE_VRAM_MB : 0; }

/* gsp_fw.h simulado. */
enum gsp_fw_kind { GSP_FW_BOOTLOADER = 0, GSP_FW_FMC, GSP_FW_UCODE, GSP_FW_BOOTER_LOAD,
                   GSP_FW_BOOTER_UNLOAD, GSP_FW_COUNT };
enum gsp_fw_chip { GSP_FW_CHIP_BLACKWELL = 0, GSP_FW_CHIP_AMPERE };
struct gsp_fw_blob { const char *path; unsigned char *data; unsigned long len;
                     unsigned gem_handle; int valid; };
static struct gsp_fw_blob g_blobs[GSP_FW_COUNT];
static const struct gsp_fw_blob *gsp_fw_get(enum gsp_fw_kind k)
{ return k < GSP_FW_COUNT && g_blobs[k].valid ? &g_blobs[k] : NULL; }

#include "gsp_rm_body.inc"
#include "gsp_wpr_body.inc"

static int load_blob(enum gsp_fw_kind kind, const char *path)
{
    FILE *f = fopen(path, "rb");
    if (!f) { perror(path); return -1; }
    fseek(f, 0, SEEK_END);
    long len = ftell(f);
    fseek(f, 0, SEEK_SET);
    unsigned char *buf = malloc(len);
    if (fread(buf, 1, len, f) != (size_t)len) { perror("read"); return -1; }
    fclose(f);
    g_blobs[kind].data = buf;
    g_blobs[kind].len = len;
    g_blobs[kind].valid = 1;
    g_blobs[kind].path = path;
    printf("blob %s: %ld bytes\n", path, len);
    return 0;
}

/* Paso 3: imagen GSP-RM + radix3, en las dos familias. */
static int check_radix3(struct gsp_rm_fw *keep)
{
    const enum gsp_fw_chip chips[] = { GSP_FW_CHIP_BLACKWELL, GSP_FW_CHIP_AMPERE };

    for (unsigned c = 0; c < 2; c++) {
        struct gsp_rm_fw fw;
        if (gsp_rm_prepare(chips[c], &fw) != 0) {
            printf("FALLO: gsp_rm_prepare chip=%u\n", chips[c]);
            return -1;
        }
        unsigned long pages = (fw.img_len + 4095) / 4096;
        if (fw.rx3.mem[2].size != ((pages * 8 + 4095) & ~4095ul)) { printf("FALLO: tamaño hoja\n"); return -1; }
        if (fw.rx3.mem[1].size != 4096 || fw.rx3.mem[0].size != 4096) { printf("FALLO: tamaño l1/raíz\n"); return -1; }
        if ((uintptr_t)fw.img & 4095) { printf("FALLO: imagen no alineada\n"); return -1; }
        const uint64_t *l2 = fw.rx3.mem[2].va;
        for (unsigned long j = 0; j < pages; j++) {
            if (l2[j] != lx_virt_to_phys(fw.img + j * 4096)) { printf("FALLO: hoja %lu\n", j); return -1; }
        }
        /* Las entradas sobrantes de la hoja quedan a cero (página parcial). */
        for (unsigned long j = pages; j < fw.rx3.mem[2].size / 8; j++) {
            if (l2[j] != 0) { printf("FALLO: relleno hoja %lu no nulo\n", j); return -1; }
        }
        printf("OK: radix3 %lu páginas, todas las hojas cuadran\n", pages);
        if (c == 0)
            *keep = fw;          /* la de Blackwell alimenta el paso 4 */
        else
            gsp_rm_release(&fw);
    }
    return 0;
}

/* Paso 4: bootloader en sysmem + GspFwWprMeta. */
static int check_wpr(const struct gsp_rm_fw *rm)
{
    struct gsp_wpr w;

    if (sizeof(struct gsp_wpr_meta) != 256) { printf("FALLO: meta no mide 256 B\n"); return -1; }
    if (gsp_wpr_prepare(rm, &w) != 0) { printf("FALLO: gsp_wpr_prepare\n"); return -1; }

    const struct gsp_wpr_meta *m = w.meta;
    if (w.fb_bytes != (uint64_t)FAKE_VRAM_MB << 20) { printf("FALLO: VRAM\n"); return -1; }
    /* 22 + 14 MiB + ALIGN(96 KiB * 12 GiB, 1 MiB) + 96 MiB = 134 MiB. */
    uint64_t want_heap = (22ull << 20) + (14ull << 20) + (2ull << 20) + (96ull << 20);
    if (m->gspFwHeapSize != want_heap) {
        printf("FALLO: heap %llu != %llu\n", (unsigned long long)m->gspFwHeapSize,
               (unsigned long long)want_heap);
        return -1;
    }
    /* Offsets del bootloader: los del blob real, y dentro de la imagen copiada. */
    if (m->bootloaderManifestOffset != 0x0 || m->bootloaderDataOffset != 0xa00 ||
        m->bootloaderCodeOffset != 0xb200) {
        printf("FALLO: offsets del bootloader inesperados\n");
        return -1;
    }
    if (m->bootloaderCodeOffset >= m->sizeOfBootloader) { printf("FALLO: code fuera\n"); return -1; }
    /* Todo lo que debe fijar el FMC sigue a cero — lo comprueba también
     * wpr_meta_verify(); aquí se repite por si esa comprobación se relaja. */
    if (m->gspFwWprStart || m->gspFwOffset || m->frtsOffset || m->fbSize) {
        printf("FALLO: campos que fija el FMC vienen escritos\n");
        return -1;
    }
    if (m->pmuReservedSize != 0x1820000u) { printf("FALLO: pmuReservedSize\n"); return -1; }
    printf("OK: WPR meta 256 B, heap %llu MiB, offsets del bootloader correctos\n",
           (unsigned long long)(m->gspFwHeapSize >> 20));

    gsp_wpr_release(&w);
    if (w.meta || w.boot.va) { printf("FALLO: release deja punteros\n"); return -1; }
    printf("OK: release limpio\n");
    return 0;
}

int main(int argc, char **argv)
{
    if (argc < 3) {
        fprintf(stderr, "uso: %s <gsp-570.144.bin> <bootloader-570.144.bin>\n", argv[0]);
        return 2;
    }
    if (load_blob(GSP_FW_UCODE, argv[1]) || load_blob(GSP_FW_BOOTLOADER, argv[2]))
        return 2;

    struct gsp_rm_fw rm;
    if (check_radix3(&rm) != 0)
        return 1;
    if (check_wpr(&rm) != 0)
        return 1;
    gsp_rm_release(&rm);
    return 0;
}
