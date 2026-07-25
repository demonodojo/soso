/* G3b: ruta GSP-FMC de Blackwell.
 *
 * GB20x no arranca el GSP por el ACR de SEC2 (eso es Ampere): el FSP recibe un
 * mensaje COT sobre EMEM con la imagen GSP-FMC y su cadena de firma, y es él
 * quien lanza el GSP. Referencia: Linux upstream `nvkm/subdev/fsp/{gh100,gb202}.c`
 * y `nvkm/subdev/gsp/gh100.c` (el árbol pinneado en lxdde/linux es 6.6 y no los
 * tiene). Aquí están los dos primeros escalones: validar el ELF firmado y leer
 * el estado del FSP. Los pasos 3 y 4 están en `gsp_rm.c` (radix3) y `gsp_wpr.c`
 * (WPR meta); el envío del COT espera al 5, los libos boot args.
 */
#include "fmc_lx.h"
#include "gsp_mmio.h"

/* --- Registros (offsets de gb202, no de gh100) ------------------------------
 * NV_THERM_I2CS_SCRATCH: gh100 lo tiene en 0x000200bc; en gb202 es 0x00ad00bc.
 * Los dos caen dentro de los 16 MiB de BAR0 que mapea el bring-up. */
#define NV_THERM_I2CS_SCRATCH_GB202   0x00ad00bcu
#define FSP_BOOT_COMPLETE_SUCCESS     0x000000ffu

#define NV_PFSP_QUEUE_HEAD0           0x008f2c00u
#define NV_PFSP_QUEUE_TAIL0           0x008f2c04u
#define NV_PFSP_MSGQ_HEAD0            0x008f2c80u
#define NV_PFSP_MSGQ_TAIL0            0x008f2c84u

/* --- ELF32 mínimo -----------------------------------------------------------
 * El ELF FMC es de 32 bits y con una cabecera fija; no arrastramos <linux/elf.h>. */
struct fmc_elf32_hdr {
    unsigned char e_ident[16];
    uint16_t e_type;
    uint16_t e_machine;
    uint32_t e_version;
    uint32_t e_entry;
    uint32_t e_phoff;
    uint32_t e_shoff;
    uint32_t e_flags;
    uint16_t e_ehsize;
    uint16_t e_phentsize;
    uint16_t e_phnum;
    uint16_t e_shentsize;
    uint16_t e_shnum;
    uint16_t e_shstrndx;
};

struct fmc_elf32_shdr {
    uint32_t sh_name;
    uint32_t sh_type;
    uint32_t sh_flags;
    uint32_t sh_addr;
    uint32_t sh_offset;
    uint32_t sh_size;
    uint32_t sh_link;
    uint32_t sh_info;
    uint32_t sh_addralign;
    uint32_t sh_entsize;
};

#define FMC_SHT_PROGBITS   1u
#define FMC_SHT_STRTAB     3u
#define FMC_SHF_ALLOC              0x002u
#define FMC_SHF_STRINGS            0x020u
#define FMC_SHF_OS_NONCONFORMING   0x100u
#define FMC_SHF_MASKOS      0x0ff00000u
#define FMC_SHF_MASKPROC    0xf0000000u
/* Combinación exacta que llevan los blobs binarios del ELF FMC. */
#define FMC_SHF_BLOB (FMC_SHF_MASKPROC | FMC_SHF_MASKOS | FMC_SHF_OS_NONCONFORMING | FMC_SHF_ALLOC)

/* Tamaños que la cabecera fija declara (ELF32: 52 y 40 bytes). */
#define FMC_ELF_HDR_SIZE   52u
#define FMC_ELF_SHDR_SIZE  40u

/* La cabecera del ELF FMC tiene que ser byte a byte esta: e_shoff justo tras la
 * cabecera, 6 secciones, tabla de nombres en la 1. */
static const unsigned char fmc_elf_header[] = {
    0x7f, 'E', 'L', 'F', 1, 1, 1, 0,
    0, 0, 0, 0, 0, 0, 0, 0,

    0, 0, 0, 0, 1, 0, 0, 0,               /* e_type, e_machine, e_version */
    0, 0, 0, 0, 0, 0, 0, 0,               /* e_entry, e_phoff */

    FMC_ELF_HDR_SIZE, 0, 0, 0, 0, 0, 0, 0, /* e_shoff, e_flags */
    FMC_ELF_HDR_SIZE, 0, 0, 0,             /* e_ehsize, e_phentsize */
    0, 0, FMC_ELF_SHDR_SIZE, 0,            /* e_phnum, e_shentsize */

    6, 0, 1, 0,                            /* e_shnum, e_shstrndx */
};

/* CRC-32 (IEEE, reflejado) con pre/post-xor, como el crc32_le del kernel. */
static uint32_t fmc_crc32(const unsigned char *p, unsigned long n)
{
    uint32_t crc = 0xffffffffu;
    unsigned long i;
    unsigned k;

    for (i = 0; i < n; i++) {
        crc ^= p[i];
        for (k = 0; k < 8; k++) {
            crc = (crc >> 1) ^ (0xedb88320u & (uint32_t)(-(int32_t)(crc & 1u)));
        }
    }
    return crc ^ 0xffffffffu;
}

static int fmc_streq(const char *a, const char *b)
{
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

static int fmc_hdr_ok(const unsigned char *elf, unsigned long len)
{
    unsigned i;

    if (len < sizeof(fmc_elf_header)) {
        return 0;
    }
    for (i = 0; i < sizeof(fmc_elf_header); i++) {
        if (elf[i] != fmc_elf_header[i]) {
            lx_printk("nouveau-lx: FMC ELF cabecera inesperada en byte %u (0x%02x)\n",
                      i, elf[i]);
            return 0;
        }
    }
    return 1;
}

/* Cada sección: tipo/flags esperados, dentro de la imagen y CRC bueno si lo trae. */
static int fmc_sections_ok(const unsigned char *elf, unsigned long len)
{
    const struct fmc_elf32_hdr *eh = (const struct fmc_elf32_hdr *)elf;
    const struct fmc_elf32_shdr *sh = (const struct fmc_elf32_shdr *)(elf + eh->e_shoff);
    unsigned long first = (unsigned long)eh->e_shoff +
                          (unsigned long)eh->e_shnum * (unsigned long)eh->e_shentsize;
    unsigned i;

    if (first > len) {
        return 0;
    }
    /* La sección 0 es la nula. */
    for (i = 1; i < eh->e_shnum; i++) {
        if (i == eh->e_shstrndx) {
            if (sh[i].sh_type != FMC_SHT_STRTAB || sh[i].sh_flags != FMC_SHF_STRINGS) {
                lx_printk("nouveau-lx: FMC ELF strtab %u inválida\n", i);
                return 0;
            }
        } else if (sh[i].sh_type != FMC_SHT_PROGBITS || sh[i].sh_flags != FMC_SHF_BLOB) {
            lx_printk("nouveau-lx: FMC ELF sección %u tipo/flags inesperados\n", i);
            return 0;
        }
        if (sh[i].sh_offset < first ||
            (unsigned long)sh[i].sh_offset + (unsigned long)sh[i].sh_size > len) {
            lx_printk("nouveau-lx: FMC ELF sección %u fuera de la imagen\n", i);
            return 0;
        }
        /* sh_info distinto de cero es el CRC32 de la sección. */
        if (sh[i].sh_info) {
            uint32_t crc = fmc_crc32(elf + sh[i].sh_offset, sh[i].sh_size);
            if (crc != sh[i].sh_info) {
                lx_printk("nouveau-lx: FMC ELF sección %u CRC 0x%08x != 0x%08x\n",
                          i, crc, sh[i].sh_info);
                return 0;
            }
        }
    }
    return 1;
}

static const unsigned char *fmc_section(const unsigned char *elf, const char *name,
                                        unsigned long *out_len)
{
    const struct fmc_elf32_hdr *eh = (const struct fmc_elf32_hdr *)elf;
    const struct fmc_elf32_shdr *sh = (const struct fmc_elf32_shdr *)(elf + eh->e_shoff);
    const char *names = (const char *)(elf + sh[eh->e_shstrndx].sh_offset);
    unsigned i;

    for (i = 1; i < eh->e_shnum; i++) {
        if (fmc_streq(&names[sh[i].sh_name], name)) {
            *out_len = sh[i].sh_size;
            return elf + sh[i].sh_offset;
        }
    }
    return NULL;
}

int fmc_lx_parse(const unsigned char *elf, unsigned long len, struct fmc_image *out)
{
    if (!elf || !out) {
        return -1;
    }
    out->image = NULL;
    out->image_len = 0;
    out->hash = NULL;
    out->hash_len = 0;
    out->pkey = NULL;
    out->pkey_len = 0;
    out->sig = NULL;
    out->sig_len = 0;

    if (!fmc_hdr_ok(elf, len) || !fmc_sections_ok(elf, len)) {
        return -1;
    }

    out->hash = fmc_section(elf, "hash", &out->hash_len);
    out->sig = fmc_section(elf, "signature", &out->sig_len);
    out->pkey = fmc_section(elf, "publickey", &out->pkey_len);
    out->image = fmc_section(elf, "image", &out->image_len);

    if (!out->hash || !out->sig || !out->pkey || !out->image) {
        lx_printk("nouveau-lx: FMC ELF sin hash/signature/publickey/image\n");
        return -1;
    }
    lx_printk("nouveau-lx: FMC ELF OK — image=%lu hash=%lu pkey=%lu sig=%lu\n",
              out->image_len, out->hash_len, out->pkey_len, out->sig_len);
    return 0;
}

/* El COT de gb20x lleva hash de 48 B, clave pública de 97 y firma de 96
 * (gh100 usa 384/384: si vemos esos tamaños, el blob no es de esta familia). */
int fmc_lx_verify_sizes(const struct fmc_image *img)
{
    if (!img) {
        return -1;
    }
    if (img->hash_len != 48u || img->pkey_len != 97u || img->sig_len != 96u) {
        lx_printk("nouveau-lx: FMC firma %lu/%lu/%lu — gb20x espera 48/97/96\n",
                  img->hash_len, img->pkey_len, img->sig_len);
        return -1;
    }
    return 0;
}

void fmc_lx_fsp_probe(void)
{
    uint32_t boot = gsp_mmio_rd32(NV_THERM_I2CS_SCRATCH_GB202);
    uint32_t qh = gsp_mmio_rd32(NV_PFSP_QUEUE_HEAD0);
    uint32_t qt = gsp_mmio_rd32(NV_PFSP_QUEUE_TAIL0);
    uint32_t mh = gsp_mmio_rd32(NV_PFSP_MSGQ_HEAD0);
    uint32_t mt = gsp_mmio_rd32(NV_PFSP_MSGQ_TAIL0);

    lx_printk("nouveau-lx: FSP secure boot=0x%08x (%s)\n", boot,
              boot == FSP_BOOT_COMPLETE_SUCCESS ? "completo" : "no completo");
    lx_printk("nouveau-lx: FSP queue head=0x%08x tail=0x%08x msgq head=0x%08x tail=0x%08x\n",
              qh, qt, mh, mt);
    if (qh == qt) {
        lx_printk("nouveau-lx: FSP EMEM libre — listo para COT (pendiente WPR/libos)\n");
    }
}
