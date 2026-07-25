/* G3b: imagen GSP-RM + radix3. Ver gsp_rm.h para el porqué de la tabla. */
#include "gsp_rm.h"

/* Definidos en el shim (lxdde/shim/src/shims.c). */
void *memcpy(void *dst, const void *src, unsigned long n);
void *memset(void *dst, int c, unsigned long n);

/* --- ELF64 mínimo ------------------------------------------------------------
 * El ucode es ELF64 little-endian sin program headers; solo nos hace falta la
 * tabla de secciones. No arrastramos <linux/elf.h> (avalancha de cabeceras). */
struct rm_elf64_hdr {
    unsigned char e_ident[16];
    uint16_t e_type;
    uint16_t e_machine;
    uint32_t e_version;
    uint64_t e_entry;
    uint64_t e_phoff;
    uint64_t e_shoff;
    uint32_t e_flags;
    uint16_t e_ehsize;
    uint16_t e_phentsize;
    uint16_t e_phnum;
    uint16_t e_shentsize;
    uint16_t e_shnum;
    uint16_t e_shstrndx;
};

struct rm_elf64_shdr {
    uint32_t sh_name;
    uint32_t sh_type;
    uint64_t sh_flags;
    uint64_t sh_addr;
    uint64_t sh_offset;
    uint64_t sh_size;
    uint32_t sh_link;
    uint32_t sh_info;
    uint64_t sh_addralign;
    uint64_t sh_entsize;
};

#define RM_ELF_CLASS64  2u
#define RM_ELF_DATA2LSB 1u
#define RM_ELF64_SHDR_SIZE 64u

static int rm_streq(const char *a, const char *b)
{
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

/* Localiza una sección por nombre dentro del ELF, con todos los límites
 * comprobados contra `len` (el blob viene de disco: nada se da por bueno). */
static int rm_elf_section(const unsigned char *elf, unsigned long len, const char *name,
                          const unsigned char **out, unsigned long *out_len)
{
    const struct rm_elf64_hdr *eh = (const struct rm_elf64_hdr *)elf;
    const struct rm_elf64_shdr *sh;
    const char *names;
    unsigned long shtab_end;
    unsigned i;

    if (len < sizeof(*eh)) {
        return -1;
    }
    if (eh->e_ident[0] != 0x7f || eh->e_ident[1] != 'E' || eh->e_ident[2] != 'L' ||
        eh->e_ident[3] != 'F' || eh->e_ident[4] != RM_ELF_CLASS64 ||
        eh->e_ident[5] != RM_ELF_DATA2LSB) {
        lx_printk("nouveau-lx: GSP-RM no es un ELF64 LSB\n");
        return -1;
    }
    if (eh->e_shentsize != RM_ELF64_SHDR_SIZE || eh->e_shnum == 0 ||
        eh->e_shstrndx >= eh->e_shnum) {
        lx_printk("nouveau-lx: GSP-RM tabla de secciones inválida\n");
        return -1;
    }
    shtab_end = (unsigned long)eh->e_shoff +
                (unsigned long)eh->e_shnum * (unsigned long)eh->e_shentsize;
    if (eh->e_shoff >= len || shtab_end > len) {
        lx_printk("nouveau-lx: GSP-RM tabla de secciones fuera del blob\n");
        return -1;
    }

    sh = (const struct rm_elf64_shdr *)(elf + eh->e_shoff);
    if (sh[eh->e_shstrndx].sh_offset >= len ||
        sh[eh->e_shstrndx].sh_offset + sh[eh->e_shstrndx].sh_size > len) {
        return -1;
    }
    names = (const char *)(elf + sh[eh->e_shstrndx].sh_offset);

    for (i = 1; i < eh->e_shnum; i++) {
        if (sh[i].sh_name >= sh[eh->e_shstrndx].sh_size) {
            continue;
        }
        if (!rm_streq(&names[sh[i].sh_name], name)) {
            continue;
        }
        if (sh[i].sh_offset >= len || sh[i].sh_offset + sh[i].sh_size > len ||
            sh[i].sh_size == 0) {
            lx_printk("nouveau-lx: GSP-RM sección %s fuera del blob\n", name);
            return -1;
        }
        *out = elf + sh[i].sh_offset;
        *out_len = sh[i].sh_size;
        return 0;
    }
    lx_printk("nouveau-lx: GSP-RM sin sección %s\n", name);
    return -1;
}

/* El ucode trae una firma por familia; cada chip carga la suya. */
static const char *rm_signature_name(enum gsp_fw_chip chip)
{
    return chip == GSP_FW_CHIP_AMPERE ? ".fwsignature_ga10x" : ".fwsignature_gb20x";
}

static unsigned long rm_page_align(unsigned long v)
{
    return (v + GSP_PAGE_SIZE - 1) & ~(GSP_PAGE_SIZE - 1);
}

static void radix3_free(struct gsp_radix3 *rx3)
{
    int i;
    for (i = 0; i < 3; i++) {
        if (rx3->mem[i].va) {
            lx_dma_free_coherent(NULL, rx3->mem[i].size, rx3->mem[i].va, rx3->mem[i].phys);
        }
        rx3->mem[i].va = NULL;
        rx3->mem[i].phys = 0;
        rx3->mem[i].size = 0;
    }
}

/* Construcción idéntica a `nvkm_gsp_radix3_sg`: de la hoja a la raíz, cada nivel
 * es una tabla de u64 con la física de cada página del nivel de abajo. Los tres
 * niveles sí son contiguos (los da dma_alloc_coherent); la imagen no hace falta. */
static int radix3_build(struct gsp_radix3 *rx3, const unsigned char *img, unsigned long img_len)
{
    unsigned long size = rm_page_align(img_len);
    int i;

    for (i = 2; i >= 0; i--) {
        unsigned long entries = size / GSP_PAGE_SIZE;
        unsigned long lvl_size = rm_page_align(entries * sizeof(uint64_t));
        uint64_t phys = 0;
        uint64_t *ptes;
        unsigned long j;

        ptes = lx_dma_alloc_coherent(NULL, lvl_size, &phys, GFP_KERNEL);
        if (!ptes || !phys) {
            lx_printk("nouveau-lx: radix3 nivel %d sin memoria (%lu bytes)\n", i, lvl_size);
            return -1;
        }
        rx3->mem[i].va = ptes;
        rx3->mem[i].phys = phys;
        rx3->mem[i].size = lvl_size;

        if (i == 2) {
            for (j = 0; j < entries; j++) {
                uint64_t pa = lx_virt_to_phys(img + j * GSP_PAGE_SIZE);
                if (!pa) {
                    lx_printk("nouveau-lx: radix3 página %lu sin traducción\n", j);
                    return -1;
                }
                ptes[j] = pa;
            }
        } else {
            for (j = 0; j < entries; j++) {
                ptes[j] = rx3->mem[i + 1].phys + j * GSP_PAGE_SIZE;
            }
        }
        size = lvl_size;
    }
    return 0;
}

/* Releer lo escrito antes de dárselo al GSP: un puntero mal puesto aquí es un
 * DMA a memoria ajena, y el fallo no se vería hasta el COT. */
static int radix3_verify(const struct gsp_radix3 *rx3, const unsigned char *img,
                         unsigned long img_len)
{
    unsigned long pages = rm_page_align(img_len) / GSP_PAGE_SIZE;
    const uint64_t *l0 = rx3->mem[0].va;
    const uint64_t *l1 = rx3->mem[1].va;
    const uint64_t *l2 = rx3->mem[2].va;
    unsigned long l1_entries = rx3->mem[1].size / sizeof(uint64_t);
    unsigned long probes[3];
    unsigned long j;
    int p;

    for (p = 0; p < 3; p++) {
        if (!rx3->mem[p].va || !rx3->mem[p].phys ||
            (rx3->mem[p].phys & (GSP_PAGE_SIZE - 1)) ||
            (rx3->mem[p].size & (GSP_PAGE_SIZE - 1))) {
            lx_printk("nouveau-lx: radix3 nivel %d mal alineado\n", p);
            return -1;
        }
    }
    /* La hoja tiene que caber en las páginas que describe el nivel 1, y estas en
     * la única entrada de la raíz. */
    if (rx3->mem[2].size / sizeof(uint64_t) < pages) {
        lx_printk("nouveau-lx: radix3 hoja corta (%lu < %lu páginas)\n",
                  rx3->mem[2].size / sizeof(uint64_t), pages);
        return -1;
    }
    if (l0[0] != rx3->mem[1].phys) {
        lx_printk("nouveau-lx: radix3 raíz no apunta al nivel 1\n");
        return -1;
    }
    for (j = 0; j < rx3->mem[2].size / GSP_PAGE_SIZE; j++) {
        if (j >= l1_entries || l1[j] != rx3->mem[2].phys + j * GSP_PAGE_SIZE) {
            lx_printk("nouveau-lx: radix3 nivel 1 entrada %lu incorrecta\n", j);
            return -1;
        }
    }
    /* Muestreo de las hojas: primera, media y última página de la imagen. */
    probes[0] = 0;
    probes[1] = pages / 2;
    probes[2] = pages - 1;
    for (p = 0; p < 3; p++) {
        uint64_t want = lx_virt_to_phys(img + probes[p] * GSP_PAGE_SIZE);
        if (!want || l2[probes[p]] != want) {
            lx_printk("nouveau-lx: radix3 hoja %lu = 0x%llx, esperada 0x%llx\n",
                      probes[p], (unsigned long long)l2[probes[p]],
                      (unsigned long long)want);
            return -1;
        }
    }
    return 0;
}

int gsp_rm_prepare(enum gsp_fw_chip chip, struct gsp_rm_fw *out)
{
    const struct gsp_fw_blob *blob = gsp_fw_get(GSP_FW_UCODE);
    const unsigned char *img = NULL;
    const unsigned char *sig = NULL;
    unsigned long img_len = 0;
    unsigned long sig_len = 0;

    if (!out) {
        return -1;
    }
    memset(out, 0, sizeof(*out));
    if (!blob || !blob->data) {
        lx_printk("nouveau-lx: GSP-RM sin ucode cargado\n");
        return -1;
    }
    if (rm_elf_section(blob->data, blob->len, ".fwimage", &img, &img_len) != 0 ||
        rm_elf_section(blob->data, blob->len, rm_signature_name(chip), &sig, &sig_len) != 0) {
        return -1;
    }

    /* La imagen se copia a un buffer alineado a página: radix3 la direcciona
     * página a página y el ucode vive a offset 0x40 dentro del ELF. */
    out->img = lx_alloc_pages_exact(img_len);
    if (!out->img) {
        lx_printk("nouveau-lx: GSP-RM sin memoria para la imagen (%lu bytes)\n", img_len);
        return -1;
    }
    memcpy(out->img, img, img_len);
    out->img_len = img_len;

    out->sig = lx_dma_alloc_coherent(NULL, sig_len, &out->sig_phys, GFP_KERNEL);
    if (!out->sig || !out->sig_phys) {
        lx_printk("nouveau-lx: GSP-RM sin memoria para la firma\n");
        gsp_rm_release(out);
        return -1;
    }
    memcpy(out->sig, sig, sig_len);
    out->sig_len = sig_len;

    if (radix3_build(&out->rx3, out->img, out->img_len) != 0 ||
        radix3_verify(&out->rx3, out->img, out->img_len) != 0) {
        gsp_rm_release(out);
        return -1;
    }

    out->ready = 1;
    lx_printk("nouveau-lx: GSP-RM %s imagen=%lu KiB (%lu páginas) firma=%lu B\n",
              rm_signature_name(chip), out->img_len / 1024u,
              rm_page_align(out->img_len) / GSP_PAGE_SIZE, out->sig_len);
    lx_printk("nouveau-lx: radix3 verificada raíz=0x%llx l1=0x%llx (%lu B) l2=0x%llx (%lu B) firma=0x%llx\n",
              (unsigned long long)out->rx3.mem[0].phys,
              (unsigned long long)out->rx3.mem[1].phys, out->rx3.mem[1].size,
              (unsigned long long)out->rx3.mem[2].phys, out->rx3.mem[2].size,
              (unsigned long long)out->sig_phys);
    return 0;
}

void gsp_rm_release(struct gsp_rm_fw *fw)
{
    if (!fw) {
        return;
    }
    radix3_free(&fw->rx3);
    if (fw->sig) {
        lx_dma_free_coherent(NULL, fw->sig_len, fw->sig, fw->sig_phys);
    }
    if (fw->img) {
        lx_free_pages_exact(fw->img, fw->img_len);
    }
    memset(fw, 0, sizeof(*fw));
}
