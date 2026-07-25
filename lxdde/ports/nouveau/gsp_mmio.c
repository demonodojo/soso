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

static struct lx_pci_dev *g_pdev;

void gsp_mmio_set_pci(struct lx_pci_dev *pdev)
{
    g_pdev = pdev;
}

int gsp_mmio_pci_recover(void)
{
    uint32_t id;
    uint32_t cmd;
    uint32_t bar0_lo;
    uint32_t bar0_hi;

    if (!g_pdev) {
        return -1;
    }
    id = lx_pci_read_config(g_pdev, 0x00, 4);
    cmd = lx_pci_read_config(g_pdev, 0x04, 4);
    bar0_lo = lx_pci_read_config(g_pdev, 0x10, 4);
    bar0_hi = lx_pci_read_config(g_pdev, 0x14, 4);

    lx_printk("nouveau-lx: PCI cfg id=0x%08x cmd=0x%04x sts=0x%04x bar0=0x%08x%08x "
              "(mem=%u bm=%u)\n",
              id, cmd & 0xffffu, (cmd >> 16) & 0xffffu, bar0_hi, bar0_lo,
              (cmd & 0x2u) ? 1u : 0u, (cmd & 0x4u) ? 1u : 0u);

    if (id == 0xffffffffu || id == 0u) {
        return -1;   /* no está: enlace caído o dispositivo retirado */
    }
    /* Un reset de función deja memory y bus-master apagados y las BAR a cero. */
    if ((cmd & 0x6u) != 0x6u) {
        lx_pci_write_config(g_pdev, 0x04, (cmd & 0xffffu) | 0x6u, 2);
        lx_printk("nouveau-lx: memory+bus-master estaban apagados — reactivados\n");
    }
    return 0;
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
