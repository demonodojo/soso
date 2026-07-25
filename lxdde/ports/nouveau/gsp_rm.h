/* G3b: imagen GSP-RM en sysmem + tabla radix3 que se la describe al GSP.
 *
 * El ucode `gsp-570.144.bin` es un ELF64 (REL, RISC-V) cuyo `.fwimage` son ~60 MiB
 * de firmware. El GSP no lo lee linealmente: recibe la dirección física de la raíz
 * de una tabla de 3 niveles (radix3) cuyas hojas listan la física de cada página de
 * la imagen — así el firmware no necesita ser físicamente contiguo. Referencia:
 * `nvkm_gsp_radix3_sg` en `nvkm/subdev/gsp/r535.c`.
 */
#ifndef GSP_RM_H
#define GSP_RM_H

#include "gsp_fw.h"
#include "lx_emul.h"

#define GSP_PAGE_SHIFT 12u
#define GSP_PAGE_SIZE  (1ul << GSP_PAGE_SHIFT)

struct gsp_radix3_level {
    void *va;             /* vista del kernel (coherente, contigua) */
    uint64_t phys;        /* lo que ve el GSP */
    unsigned long size;   /* múltiplo de página */
};

/* mem[0] = raíz (la que va al WPR meta), mem[2] = hojas con la imagen. */
struct gsp_radix3 {
    struct gsp_radix3_level mem[3];
};

struct gsp_rm_fw {
    unsigned char *img;      /* copia de `.fwimage` alineada a página */
    unsigned long img_len;
    unsigned char *sig;      /* copia de `.fwsignature_<familia>` (coherente) */
    unsigned long sig_len;
    uint64_t sig_phys;
    struct gsp_radix3 rx3;
    int ready;
};

/* Extrae `.fwimage` + la firma de esta familia del ucode ya cargado, los copia a
 * memoria apta para DMA y construye + verifica la radix3. No toca ningún registro.
 * Devuelve 0 si `out` queda listo para el WPR meta. */
int gsp_rm_prepare(enum gsp_fw_chip chip, struct gsp_rm_fw *out);

void gsp_rm_release(struct gsp_rm_fw *fw);

#endif
