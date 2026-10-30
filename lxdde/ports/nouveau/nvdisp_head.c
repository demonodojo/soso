/* N4 — devinit (GFW) + head0 640×480 + barras SMPTE en VRAM. */
#include "nvdisp_head.h"
#include "nvdisp_probe.h"
#include "gsp_chip.h"
#include "gsp_mmio.h"
#include "gsp_pramin.h"
#include "lx_emul.h"

static int g_head_ready;

uint32_t nvdisp_head_fb_vram(void)
{
    return (uint32_t)NVDISP_HEAD_FB_VRAM;
}

unsigned nvdisp_head_pitch(void)
{
    return NVDISP_HEAD_PITCH;
}

int nvdisp_ready(void)
{
    return g_head_ready;
}

static int mmio_ok(unsigned off)
{
    uint32_t v = gsp_mmio_rd32(off);

    return !gsp_mmio_pri_error(v);
}

static int nvdisp_fill_smpte_bars(void)
{
    static const uint32_t colors[8] = {
        0xffffffffu, 0xffffff00u, 0xff00ffffu, 0xff00ff00u,
        0xffff00ffu, 0xffff0000u, 0xff0000ffu, 0xff000000u,
    };
    unsigned x;
    unsigned y;
    unsigned bar_w = NVDISP_HEAD_WIDTH / 8u;

    if (!gsp_pramin_alive()) {
        lx_printk("NVDisplay: N4 sin PRAMIN para patrón\n");
        return -1;
    }

    for (y = 0; y < NVDISP_HEAD_HEIGHT; y++) {
        for (x = 0; x < NVDISP_HEAD_WIDTH; x++) {
            unsigned bar = x / bar_w;
            if (bar > 7u) {
                bar = 7u;
            }
            gsp_pramin_wr32(NVDISP_HEAD_FB_VRAM + (uint64_t)y * NVDISP_HEAD_PITCH +
                                (uint64_t)x * 4u,
                            colors[bar]);
        }
    }
    return 0;
}

static int nvdisp_head0_timings(void)
{
    /* VESA 640×480 @ ~60 Hz — empaquetado como gf119_head_state lee. */
    const unsigned hoff = 0u;
    const unsigned htotal = 800u;
    const unsigned vtotal = 525u;
    const unsigned hsynce = 752u;
    const unsigned vsynce = 492u;
    const unsigned hblanke = 800u;
    const unsigned vblanke = 525u;
    const unsigned hblanks = 656u;
    const unsigned vblanks = 490u;

    gsp_mmio_wr32(0x640414u + hoff, (vtotal << 16) | htotal);
    if (!mmio_ok(0x640414u + hoff)) {
        return -1;
    }
    gsp_mmio_wr32(0x640418u + hoff, (vsynce << 16) | hsynce);
    if (!mmio_ok(0x640418u + hoff)) {
        return -1;
    }
    gsp_mmio_wr32(0x64041cu + hoff, (vblanke << 16) | hblanke);
    if (!mmio_ok(0x64041cu + hoff)) {
        return -1;
    }
    gsp_mmio_wr32(0x640420u + hoff, (vblanks << 16) | hblanks);
    if (!mmio_ok(0x640420u + hoff)) {
        return -1;
    }
    /* 24 bpp, layout similar a gf119_head_state case 5. */
    gsp_mmio_wr32(0x640404u + hoff, 0x00000140u);
    if (!mmio_ok(0x640404u + hoff)) {
        return -1;
    }
    return 0;
}

static int nvdisp_head0_enable(enum nv_family fam)
{
    (void)fam;
    if (nvdisp_fill_smpte_bars() != 0) {
        return -1;
    }
    if (nvdisp_head0_timings() != 0) {
        lx_printk("NVDisplay: N4 timings head0 rechazados por PRI\n");
        return -1;
    }
    lx_printk("NVDisplay: N4 patrón SMPTE @ VRAM 0x%x pitch=%u\n",
              (unsigned)NVDISP_HEAD_FB_VRAM, NVDISP_HEAD_PITCH);
    lx_printk("NVDisplay: N4 head0 640x480 (scanout canal disp → N5)\n");
    return 0;
}

int nvdisp_head_bringup(void)
{
    enum nv_family fam;

    g_head_ready = 0;
    if (!gsp_mmio_alive()) {
        return -1;
    }

    fam = gsp_nv_family_current();
    if (fam != NV_FAM_AMPERE && fam != NV_FAM_ADA && fam != NV_FAM_BLACKWELL) {
        lx_printk("NVDisplay: N4 omitido (familia %s)\n", gsp_nv_family_name(fam));
        return -1;
    }

    if (fam != NV_FAM_BLACKWELL) {
        if (gsp_mmio_gfw_wait(500u, "N4") != 0) {
            lx_printk("NVDisplay: N4 GFW/devinit no confirmado (best-effort)\n");
        }
    }

    lx_printk("NVDisplay: N8 familia=%s chip=%s (disp ga102/gf119; canal HW pendiente)\n",
              gsp_nv_family_name(fam),
              gsp_nv_chip_name(gsp_nv_family_device_id(), fam));

    if (nvdisp_head0_enable(fam) != 0) {
        return -1;
    }

    g_head_ready = 1;
    return 0;
}

int nvdisp_head_selftest(void)
{
    if (NVDISP_HEAD_WIDTH / 8u == 0) {
        return -1;
    }
    if (NVDISP_HEAD_PITCH != NVDISP_HEAD_WIDTH * 4u) {
        return -1;
    }
    if (NVDISP_HEAD_FB_VRAM < 0x01000000ull) {
        return -1;
    }
    return 0;
}
