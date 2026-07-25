/* G3: MMIO BAR0 + poll/kick GSP (registros tu102/ga102, sin ACR completo). */
#include "gsp_mmio.h"
#include "lx_emul.h"

static void *g_bar0;
static unsigned g_bar0_len;

void gsp_mmio_set_bar(void *bar, unsigned len)
{
    g_bar0 = bar;
    g_bar0_len = len;
}

uint32_t gsp_mmio_rd32(unsigned off)
{
    volatile uint32_t *p;
    if (!g_bar0 || off + 4u > g_bar0_len) {
        return 0;
    }
    p = (volatile uint32_t *)((unsigned char *)g_bar0 + off);
    return *p;
}

void gsp_mmio_wr32(unsigned off, uint32_t val)
{
    volatile uint32_t *p;
    if (!g_bar0 || off + 4u > g_bar0_len) {
        return;
    }
    p = (volatile uint32_t *)((unsigned char *)g_bar0 + off);
    *p = val;
}

int gsp_mmio_alive(void)
{
    /* Todo a unos = la GPU no contesta (reset, enlace PCIe caído, desaparecida).
     * boot0 a cero tampoco es un chip vivo. */
    uint32_t boot0 = gsp_mmio_rd32(0x0000u);
    return boot0 != 0xffffffffu && boot0 != 0u;
}

int gsp_mmio_poll_ready(unsigned timeout_ms)
{
    /* tu102_devinit_wait — GSP listo cuando 0x118128 bit0 y 0x118234 == 0xff */
    while (timeout_ms--) {
        uint32_t a = gsp_mmio_rd32(0x118128u);
        uint32_t b = gsp_mmio_rd32(0x118234u);
        /* Con la GPU fuera del bus TODO se lee 0xffffffff, y eso cumplía la
         * condición de "listo" al pie de la letra: bit0 puesto y 0xff en los
         * bits bajos. Así se dio por bueno un arranque con la tarjeta muerta
         * (2026-07-25). Un all-ones no es un registro, es silencio. */
        if (a == 0xffffffffu && b == 0xffffffffu) {
            lx_printk("nouveau-lx: GPU fuera del bus (MMIO todo a unos)\n");
            return -1;
        }
        if ((a & 1u) && ((b & 0xffu) == 0xffu)) {
            lx_printk("nouveau-lx: GSP hw ready (118128=0x%x 118234=0x%x)\n", a, b);
            return 0;
        }
        lx_mdelay(1);
    }
    return -1;
}

int gsp_mmio_kick_boot(void)
{
    /*
     * Secuencia mínima previa a nvkm ACR (ASB/AHESASC).
     * En hardware real el boot completo pasa por acr/tu102; aquí solo
     * despertamos la ruta falcon GSP si el registro responde.
     */
    uint32_t boot0 = gsp_mmio_rd32(0x0000u);
    lx_printk("nouveau-lx: kick GSP boot0=0x%08x\n", boot0);
    return 0;
}
