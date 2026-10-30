#ifndef NVDISP_HEAD_H
#define NVDISP_HEAD_H

#include <stdint.h>

/* N4 — modo fijo 640×480 A8B8G8R8 (contrato para N5 scanout). */
#define NVDISP_HEAD_WIDTH  640u
#define NVDISP_HEAD_HEIGHT 480u
#define NVDISP_HEAD_PITCH  (NVDISP_HEAD_WIDTH * 4u)
#define NVDISP_HEAD_FB_VRAM 0x01800000ull

/* Tras GSP-RM + VMM: devinit (GFW) + patrón SMPTE + timings head0. */
int nvdisp_head_bringup(void);

/* 1 si head0 quedó programado (Ampere/Ada con PRAMIN). */
int nvdisp_ready(void);

uint32_t nvdisp_head_fb_vram(void);
unsigned nvdisp_head_pitch(void);

/* Hostcheck: geometría del patrón SMPTE. */
int nvdisp_head_selftest(void);

#endif
