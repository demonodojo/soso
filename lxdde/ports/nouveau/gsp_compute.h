/* G4f: lanzamiento compute (QMD inline + SASS). Ver gsp_compute.h. */
#ifndef GSP_COMPUTE_H
#define GSP_COMPUTE_H

#include "gsp_ce.h"
#include "gsp_chan.h"

#define G4F_SASS_VA   0x0000010000001000ull   /* VRAM: blob SASS (G4d+4KiB) */
#define G4F_PARAM_VA  0x0000010000020000ull   /* sysmem params (detrás scratch) */

struct gsp_compute {
    struct gsp_rm *rm;
    struct gsp_chan *chan;
    uint32_t handle;
    uint64_t sass_va;
    uint32_t sass_size;
    int ready;
};

extern const unsigned char gsp_saxpy_sass[];
extern const unsigned gsp_saxpy_sass_len;

int gsp_compute_init(struct gsp_rm *rm, struct gsp_chan *chan,
                     struct gsp_compute *cp);

/* Copia SASS a VRAM vía CE (best-effort). */
int gsp_compute_stage_sass(struct gsp_compute *cp, struct gsp_ce *ce,
                           uint64_t scratch_va, void *scratch_cpu);

/* Codifica inline QMD + métodos compute en el pushbuffer. */
int gsp_compute_encode_qmd(struct gsp_compute *cp, const GspQmdV05 *qmd,
                           unsigned *pb_off, unsigned *pb_len);

/* Rellena QMD mínimo para saxpy @ sass_va. */
void gsp_compute_fill_saxpy_qmd(GspQmdV05 *qmd, uint64_t prog_va, unsigned grid_x);

/* Intenta lanzar saxpy; devuelve 0 solo con readback HW verificado. */
int gsp_compute_saxpy(struct gsp_compute *cp, struct gsp_ce *ce,
                        float a, const float *x, float *y, unsigned n,
                        uint64_t scratch_va, void *scratch_cpu);

void gsp_compute_fini(struct gsp_compute *cp);

#endif
