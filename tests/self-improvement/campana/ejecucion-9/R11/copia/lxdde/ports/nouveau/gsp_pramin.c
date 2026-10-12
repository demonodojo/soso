/* PRAMIN: ventana BAR0 a VRAM para leer lo que GSP-RM escribió en FB. */
#include "gsp_pramin.h"
#include "gsp_chip.h"
#include "gsp_mmio.h"
#include "lx_emul.h"

static uint64_t g_win_base = ~(uint64_t)0;
static int g_alive = -1; /* -1 = sin probar; 0 = SCPM/muerto; 1 = vivo */

void gsp_pramin_invalidate(void)
{
    g_win_base = ~(uint64_t)0;
    g_alive = -1;
}

static int pramin_is_blackwell(void)
{
    return gsp_nv_family_current() == NV_FAM_BLACKWELL;
}

static uint32_t pramin_window_reg(void)
{
    return pramin_is_blackwell() ? GSP_PRAMIN_WINDOW_REG_GB
                                 : GSP_PRAMIN_WINDOW_REG_NV50;
}

static uint64_t pramin_align_mask(void)
{
    return pramin_is_blackwell() ? (uint64_t)GSP_PRAMIN_OFF_MASK_GB
                                 : (uint64_t)GSP_PRAMIN_OFF_MASK_NV50;
}

static unsigned pramin_offset_mask(void)
{
    return pramin_is_blackwell() ? GSP_PRAMIN_OFF_MASK_GB
                                 : GSP_PRAMIN_OFF_MASK_NV50;
}

static int pramin_set_window(uint64_t addr)
{
    uint32_t win = (uint32_t)(addr >> GSP_PRAMIN_WINDOW_SHIFT);
    uint32_t rb;
    uint32_t reg = pramin_window_reg();

    gsp_mmio_wr32(reg, win);
    rb = gsp_mmio_rd32(reg);
    if (gsp_mmio_pri_error(rb)) {
        lx_printk("nouveau-lx: PRAMIN ventana 0x%08x → readback 0x%08x "
                  "(SCPM o registro muerto)\n", win, rb);
        return -1;
    }
    /* Blackwell exige readback exacto; nv50_instmem no siempre lo devuelve. */
    if (pramin_is_blackwell() && rb != win) {
        lx_printk("nouveau-lx: PRAMIN ventana 0x%08x → readback 0x%08x "
                  "(registro incoherente)\n", win, rb);
        return -1;
    }
    g_win_base = (uint64_t)win << GSP_PRAMIN_WINDOW_SHIFT;
    return 0;
}

static int pramin_ensure(uint64_t addr)
{
    uint64_t want = addr & ~pramin_align_mask();

    if (want == g_win_base) {
        return 0;
    }
    return pramin_set_window(addr);
}

uint32_t gsp_pramin_rd32(uint64_t addr_vram)
{
    unsigned off;

    if (!gsp_mmio_alive()) {
        return 0xffffffffu;
    }
    if (pramin_ensure(addr_vram) != 0) {
        return 0xffffffffu;
    }
    off = (unsigned)(addr_vram & pramin_offset_mask());
    if (GSP_PRAMIN_MMIO_BASE + off + 4u > 0x1000000u) {
        return 0xffffffffu;
    }
    return gsp_mmio_rd32(GSP_PRAMIN_MMIO_BASE + off);
}

void gsp_pramin_wr32(uint64_t addr_vram, uint32_t val)
{
    unsigned off;

    if (!gsp_mmio_alive()) {
        return;
    }
    if (pramin_ensure(addr_vram) != 0) {
        return;
    }
    off = (unsigned)(addr_vram & pramin_offset_mask());
    if (GSP_PRAMIN_MMIO_BASE + off + 4u > 0x1000000u) {
        return;
    }
    gsp_mmio_wr32(GSP_PRAMIN_MMIO_BASE + off, val);
}

void gsp_pramin_memset32(uint64_t addr_vram, uint32_t val, unsigned bytes)
{
    unsigned off;

    if (bytes == 0 || (bytes & 3u) != 0) {
        return;
    }
    for (off = 0; off < bytes; off += 4u) {
        gsp_pramin_wr32(addr_vram + off, val);
    }
}

int gsp_pramin_alive(void)
{
    uint32_t w0;

    if (g_alive >= 0) {
        return g_alive;
    }
    g_alive = 0;
    if (!gsp_mmio_alive()) {
        return 0;
    }
    gsp_pramin_invalidate();
    if (pramin_set_window(0x1000ull) != 0) {
        lx_printk("nouveau-lx: PRAMIN ventana BAR0 no programable\n");
        return 0;
    }
    w0 = gsp_mmio_rd32(GSP_PRAMIN_MMIO_BASE + 0x1000u);
    if (gsp_mmio_pri_error(w0)) {
        lx_printk("nouveau-lx: PRAMIN lee 0x%08x @ VRAM+0x1000 — ventana "
                  "bloqueada por SCPM\n", w0);
        return 0;
    }
    g_alive = 1;
    lx_printk("nouveau-lx: PRAMIN ventana BAR0 viva (reg=0x%06x, 0x%06x + "
              "offset)\n", pramin_window_reg(), GSP_PRAMIN_MMIO_BASE);
    return 1;
}
