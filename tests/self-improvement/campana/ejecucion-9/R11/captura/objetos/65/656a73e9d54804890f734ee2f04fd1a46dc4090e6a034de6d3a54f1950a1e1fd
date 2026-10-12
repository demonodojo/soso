/* G3b: ruta GSP-FMC de Blackwell — parseo del ELF firmado + sondeo del FSP. */
#ifndef FMC_LX_H
#define FMC_LX_H

#include "gsp_dma.h"
#include "lx_emul.h"

/* Secciones del ELF `fmc-*.bin` que el FSP necesita para el mensaje COT. */
struct fmc_image {
    const unsigned char *image;
    unsigned long image_len;
    const unsigned char *hash;
    unsigned long hash_len;
    const unsigned char *pkey;
    unsigned long pkey_len;
    const unsigned char *sig;
    unsigned long sig_len;
};

/* Valida cabecera, secciones y CRCs del ELF FMC y localiza las 4 secciones.
 * Devuelve 0 si la imagen sirve para arrancar el GSP. */
int fmc_lx_parse(const unsigned char *elf, unsigned long len, struct fmc_image *out);

/* Comprueba que los tamaños casan con lo que espera el FSP de esta familia
 * (gb20x: hash 48, pkey 97, sig 96). */
int fmc_lx_verify_sizes(const struct fmc_image *img);

/* Lectura pura de los registros del FSP: ¿ha terminado su secure boot y está
 * el buzón EMEM libre? No escribe nada. */
void fmc_lx_fsp_probe(void);

/* Las cuatro secciones donde el FSP las va a leer: por DMA, no desde el heap.
 * El COT lleva la física de la imagen y las tres partes de la cadena de firma. */
struct fmc_staged {
    struct gsp_dma_buf img;
    struct gsp_dma_buf hash;
    struct gsp_dma_buf pkey;
    struct gsp_dma_buf sig;
};

int fmc_lx_stage(const struct fmc_image *img, struct fmc_staged *out);
void fmc_lx_stage_release(struct fmc_staged *s);

#endif
