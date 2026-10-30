/* N5 — presentación VRAM → FB head (sin shadow GOP). */
#include "nvdisp_scanout.h"
#include "nvdisp_head.h"
#include "gsp_pramin.h"
#include "gsp_vram_copy.h"
#include "lx_emul.h"

static int clip_rect(unsigned *x, unsigned *y, unsigned *w, unsigned *h)
{
    if (*x >= NVDISP_HEAD_WIDTH || *y >= NVDISP_HEAD_HEIGHT) {
        return -1;
    }
    if (*x + *w > NVDISP_HEAD_WIDTH) {
        *w = NVDISP_HEAD_WIDTH - *x;
    }
    if (*y + *h > NVDISP_HEAD_HEIGHT) {
        *h = NVDISP_HEAD_HEIGHT - *y;
    }
    if (*w == 0 || *h == 0) {
        return -1;
    }
    return 0;
}

int nvdisp_scanout_rect(uint64_t rt_va, unsigned rt_pitch, unsigned x, unsigned y,
                        unsigned w, unsigned h)
{
    unsigned row;
    unsigned col;

    if (!nvdisp_ready()) {
        return -1;
    }
    if (!gsp_pramin_alive() || rt_va == 0 || rt_pitch < 4) {
        return -1;
    }
    if (clip_rect(&x, &y, &w, &h) != 0) {
        return -1;
    }

    {
        uint64_t src0 = rt_va + (uint64_t)y * rt_pitch + (uint64_t)x * 4u;
        uint64_t dst0 =
            NVDISP_HEAD_FB_VRAM + (uint64_t)y * NVDISP_HEAD_PITCH + (uint64_t)x * 4u;
        uint32_t line_bytes = w * 4u;

        if (line_bytes <= rt_pitch && line_bytes <= NVDISP_HEAD_PITCH &&
            gsp_vram_blit_pitch(dst0, NVDISP_HEAD_PITCH, src0, rt_pitch, line_bytes,
                                h) == 0) {
            return 0;
        }
    }

    for (row = 0; row < h; row++) {
        uint64_t src_row = rt_va + (uint64_t)(y + row) * rt_pitch + (uint64_t)x * 4u;
        uint64_t dst_row =
            NVDISP_HEAD_FB_VRAM + (uint64_t)(y + row) * NVDISP_HEAD_PITCH + (uint64_t)x * 4u;

        for (col = 0; col < w; col++) {
            uint32_t pix = gsp_pramin_rd32(src_row + (uint64_t)col * 4u);

            if ((pix & 0xffff0000u) == 0xbadf0000u) {
                return -1;
            }
            gsp_pramin_wr32(dst_row + (uint64_t)col * 4u, pix);
        }
    }
    return 0;
}

int nvdisp_scanout_full(uint64_t rt_va, unsigned w, unsigned h, unsigned pitch)
{
    unsigned cw = w;
    unsigned ch = h;

    if (cw > NVDISP_HEAD_WIDTH) {
        cw = NVDISP_HEAD_WIDTH;
    }
    if (ch > NVDISP_HEAD_HEIGHT) {
        ch = NVDISP_HEAD_HEIGHT;
    }
    if (pitch < cw * 4u) {
        return -1;
    }
    return nvdisp_scanout_rect(rt_va, pitch, 0, 0, cw, ch);
}

unsigned nvdisp_scanout_tile_bytes(unsigned w, unsigned h)
{
    return w * h * 4u;
}

int nvdisp_scanout_selftest(void)
{
    unsigned w = 100;
    unsigned h = 50;
    unsigned x = 600;
    unsigned y = 400;

    if (clip_rect(&x, &y, &w, &h) != 0) {
        return -1;
    }
    if (w != 40 || h != 50) {
        return -1;
    }
    if (nvdisp_scanout_tile_bytes(w, h) != 40u * 50u * 4u) {
        return -1;
    }
    return 0;
}
