/* SASS del kernel `gprim` (escena de primitivas del escritorio, G1). Un juego por arquitectura,
 * aparte de sass_sets.h para no tocar los blobs ya validados de los demás kernels.
 * Lo generan tools/gpu-prim/cubin.py (NVRTC) + scripts/l6-g4f-emit-sass.py. */
#ifndef SASS_GPRIM_H
#define SASS_GPRIM_H

#include "gsp_sass.h"

extern const struct gsp_sass_set gsp_sass_set_gprim_sm86;
extern const struct gsp_sass_set gsp_sass_set_gprim_sm120;

#define GSP_SASS_GPRIM_SETS { &gsp_sass_set_gprim_sm86, &gsp_sass_set_gprim_sm120, }
#define GSP_SASS_GPRIM_COUNT 2u

#endif
