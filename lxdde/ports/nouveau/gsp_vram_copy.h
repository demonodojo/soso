/* N8 — copia VRAM→VRAM vía CE (tiles de scanout NVDisplay). */
#ifndef GSP_VRAM_COPY_H
#define GSP_VRAM_COPY_H

#include <stdint.h>

/* Rectángulo pitch-linear: `line_bytes` × `lines` (p. ej. daño GSCEN). */
int gsp_vram_blit_pitch(uint64_t dst_va, uint32_t dst_pitch, uint64_t src_va,
                        uint32_t src_pitch, uint32_t line_bytes, uint32_t lines);

#endif
