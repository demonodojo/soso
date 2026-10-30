#ifndef NVDISP_SCANOUT_H
#define NVDISP_SCANOUT_H

#include <stdint.h>

/* N5 — copia A8B8G8R8 del RT (VRAM) al FB de scanout (NVDISP_HEAD_FB_VRAM). */
int nvdisp_scanout_rect(uint64_t rt_va, unsigned rt_pitch, unsigned x, unsigned y,
                        unsigned w, unsigned h);
int nvdisp_scanout_full(uint64_t rt_va, unsigned w, unsigned h, unsigned pitch);

int nvdisp_scanout_selftest(void);

unsigned nvdisp_scanout_tile_bytes(unsigned w, unsigned h);

#endif
