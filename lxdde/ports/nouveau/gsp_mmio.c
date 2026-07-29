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
    /* BAR0 de NVIDIA es de 32 bits; 0x14 ya es BAR1. */
    bar0_lo = lx_pci_read_config(g_pdev, 0x10, 4);
    bar0_hi = lx_pci_read_config(g_pdev, 0x14, 4);

    lx_printk("nouveau-lx: PCI cfg id=0x%08x cmd=0x%04x sts=0x%04x bar0=0x%08x bar1lo=0x%08x "
              "(mem=%u bm=%u)\n",
              id, cmd & 0xffffu, (cmd >> 16) & 0xffffu, bar0_lo, bar0_hi,
              (cmd & 0x2u) ? 1u : 0u, (cmd & 0x4u) ? 1u : 0u);

    if (id == 0xffffffffu || id == 0u) {
        return -1;   /* no está: enlace caído o dispositivo retirado */
    }
    /* Y un `id` válido tampoco prueba que la tarjeta conteste: bajo VFIO,
     * vfio-pci **emula** los primeros registros desde la copia que guardó al
     * abrir el dispositivo. El 2026-07-28, con el MMIO muerto, esto leyó
     * id=0x2f1810de (correcto) junto a sts=0xffff y cmd=0xfbff (silencio). El
     * status a unos es la señal fiable de que detrás no hay nadie; se dice para
     * que el log no invite a pensar que la tarjeta está bien. */
    if (((cmd >> 16) & 0xffffu) == 0xffffu) {
        lx_printk("nouveau-lx: cfg: sts a unos — el id lo emula vfio, la tarjeta "
                  "no contesta\n");
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

int gsp_mmio_pri_error(uint32_t v)
{
    /* Los `0xbadfxxxx` son la respuesta del anillo PRI cuando el bloque de
     * destino no contesta (sin alimentación, sin reloj, sin devinit) o el acceso
     * está prohibido: `0xbadf1000`, `0xbadf4100` —el que dio el falcon de
     * Blackwell con el ACR de Ampere—, `0xbadf5040`… No es un valor: es un error
     * con forma de valor, y leerlo como dato es cómo se sacan conclusiones de
     * registros que no existen. */
    return (v & 0xffff0000u) == 0xbadf0000u;
}

int gsp_mmio_gfw_wait(unsigned timeout_ms, const char *when)
{
    unsigned waited = 0;
    uint32_t a = 0;
    uint32_t b = 0;

    /* El arranque del firmware de la GPU **en Turing/Ampere**: nouveau lo espera en
     * `tu102_devinit_wait` con dos registros de la isla GC6, y los dos tienen nombre
     * verificado en `dev_gc6_island[_addendum].h` de tu102 (2026-07-29):
     *
     *   0x118128  `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_PRIV_LEVEL_MASK` — el bit 0
     *             es READ_PROTECTION_LEVEL0_ENABLE, o sea "se nos permite leer el
     *             scratch". Es un permiso, NO un progreso.
     *   0x118234  `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05(0)`, alias
     *             `..._GFW_BOOT`: PROGRESS 7:0, COMPLETED = 0xff.
     *
     * **Esto NO vale para gb20x** y confundirlo costó un ciclo (2026-07-29): en
     * Blackwell el registro se lee 0 siempre —NVIDIA no publica ni
     * `dev_gc6_island.h` para gb202— y allí el indicador es
     * `NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE` (0x00ad00bc, SUCCESS = 0xff), que
     * es el que ya exige `fsp_ready_to_send()` antes del COT. Por eso el llamante
     * sólo entra aquí cuando la familia NO es Blackwell. */
    for (;;) {
        a = gsp_mmio_rd32(0x118128u);
        b = gsp_mmio_rd32(0x118234u);
        if (a == 0xffffffffu && b == 0xffffffffu) {
            lx_printk("nouveau-lx: GFW boot (%s): MMIO todo a unos — la GPU no "
                      "está en el bus\n", when);
            return -1;
        }
        if (gsp_mmio_pri_error(a) || gsp_mmio_pri_error(b)) {
            lx_printk("nouveau-lx: GFW boot (%s): error de PRI "
                      "(118128=0x%08x 118234=0x%08x) — el bloque no contesta\n",
                      when, a, b);
            return -1;
        }
        if ((a & 1u) && ((b & 0xffu) == 0xffu)) {
            lx_printk("nouveau-lx: GFW boot COMPLETED (%s) tras %u ms "
                      "(118128=0x%08x 118234=0x%08x)\n", when, waited, a, b);
            return 0;
        }
        if (waited >= timeout_ms) {
            break;
        }
        lx_mdelay(1);
        waited++;
    }
    /* Informativo, no veredicto: quien decide si se puede arrancar el GSP es la
     * comprobación de la familia que corresponda. */
    lx_printk("nouveau-lx: GFW boot NO completado (%s) en %u ms "
              "(118128=0x%08x lectura-permitida=%u, 118234=0x%08x progress=0x%02x, "
              "hace falta 0xff)\n",
              when, timeout_ms, a, a & 1u, b, b & 0xffu);
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
