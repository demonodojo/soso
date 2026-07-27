/* G4f: lanzamiento compute (QMD inline + SASS).
 *
 * El SASS lo compila `scripts/l6-g4f-build-sass.sh` y llega por
 * `saxpy_sass_embed.c` junto con los metadatos que el cubin declara: dónde
 * espera el kernel sus parámetros dentro del constant bank 0 y cuántos
 * registros usa. Esos dos números NO se inventan aquí — si se hardcodean y el
 * .cu cambia, el lanzamiento lee basura sin decir nada.
 */
#ifndef GSP_COMPUTE_H
#define GSP_COMPUTE_H

#include "gsp_ce.h"
#include "gsp_chan.h"

#define G4F_SASS_VA   0x0000010000001000ull   /* VRAM: blob SASS (G4d+4KiB) */
#define G4F_QMD_VA    0x0000010000020000ull   /* VRAM: destino del QMD inline */

/* Sysmem coherente propia del compute, detrás de los búferes del canal. La CPU
 * escribe aquí directamente: sin BAR1 no hay ventana a la VRAM, así que el
 * constant bank y los vectores viven en sysmem y se leen por la VA space. */
#define G4F_DATA_VA       0x0000010030000000ull
#define G4F_DATA_SIZE     8192u
#define G4F_CBANK_OFF     0u        /* alineado a 64 B: lo exige ADDR_SHIFTED6 */
#define G4F_X_OFF         0x400u
#define G4F_Y_OFF         0x800u
#define G4F_SEM_OFF       0xc00u
#define G4F_MAX_N         256u      /* 1 KiB por vector */

#define G4F_CTA_THREADS   256u
#define G4F_SEM_PAYLOAD   0x5a5a0001u
#define G4F_WAIT_MS       2000u

struct gsp_compute {
    struct gsp_rm *rm;
    struct gsp_chan *chan;
    uint32_t handle;
    uint64_t sass_va;
    uint32_t sass_size;
    struct gsp_dma_buf data;   /* cbank0 + x + y + semáforo, en sysmem */
    uint64_t data_va;
    int mapped;
    int ready;
};

/* Blob SASS y los metadatos que el cubin declara sobre él. */
extern const unsigned char gsp_saxpy_sass[];
extern const unsigned gsp_saxpy_sass_len;
extern const unsigned gsp_saxpy_regcount;    /* EIATTR_REGCOUNT */
extern const unsigned gsp_saxpy_param_base;  /* EIATTR_PARAM_CBANK: offset en cbank0 */
extern const unsigned gsp_saxpy_param_size;  /* EIATTR_CBANK_PARAM_SIZE */
extern const unsigned gsp_saxpy_cbank_size;  /* tamaño de .nv.constant0 */
extern const unsigned gsp_saxpy_param_off[4]; /* a, x, y, n (relativos a base) */

int gsp_compute_init(struct gsp_rm *rm, struct gsp_chan *chan,
                     struct gsp_compute *cp);

/* Copia SASS a VRAM vía CE y espera al semáforo. */
int gsp_compute_stage_sass(struct gsp_compute *cp, struct gsp_ce *ce,
                           uint64_t scratch_va, void *scratch_cpu);

/* Codifica inline QMD + métodos compute en el pushbuffer. */
int gsp_compute_encode_qmd(struct gsp_compute *cp, const GspQmdV05 *qmd,
                           unsigned *pb_off, unsigned *pb_len);

/* Rellena el QMD para saxpy: programa, malla, registros y constant bank. */
void gsp_compute_fill_saxpy_qmd(struct gsp_compute *cp, GspQmdV05 *qmd,
                                uint64_t prog_va, unsigned grid_x);

/* Escribe los parámetros del kernel en el constant bank. */
void gsp_compute_set_params(struct gsp_compute *cp, float a, uint64_t x_va,
                            uint64_t y_va, unsigned n);

/* Lanza saxpy de verdad; devuelve 0 solo con el semáforo del QMD señalizado y
 * el resultado releído de memoria que escribió la GPU. */
int gsp_compute_saxpy(struct gsp_compute *cp, struct gsp_ce *ce,
                      float a, const float *x, float *y, unsigned n,
                      uint64_t scratch_va, void *scratch_cpu);

void gsp_compute_fini(struct gsp_compute *cp);

#endif
