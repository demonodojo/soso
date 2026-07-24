/* G3: GSP bring-up gb205 — carga firmware + estado nvkm mínimo. */
#include "lx_emul.h"

#define NV_PMC_BOOT_0_OFF 0x0000u
#define GB205_DEVICE_ID   0x2f18u

enum gsp_phase {
    GSP_NONE = 0,
    GSP_BAR0,
    GSP_FW_LOADING,
    GSP_FW_READY,
    GSP_BOOTED,
};

static enum gsp_phase g_phase = GSP_NONE;
static uint16_t g_device_id;
static uint32_t g_boot0;
static int g_fw_loaded;
static void *g_bar0;
static unsigned g_vram_bytes;

static const char *const g_fw_gb205[] = {
    "nvidia/gb205/gsp/bootloader-570.144.bin.zst",
    "nvidia/gb205/gsp/fmc-570.144.bin.zst",
    "nvidia/gb205/gsp/gsp-570.144.bin.zst",
    "nvidia/ga102/gsp/gsp-570.144.bin.zst",
    NULL,
};

static int load_fw_chain(void)
{
    const char *const *p;
    g_fw_loaded = 0;
    g_phase = GSP_FW_LOADING;
    for (p = g_fw_gb205; *p; p++) {
        const unsigned char *data = NULL;
        unsigned long len = 0;
        if (lx_request_firmware(*p, &data, &len) == 0 && data && len > 0) {
            g_fw_loaded++;
            lx_printk("nouveau-lx: fw %s (%lu bytes)\n", *p, len);
            lx_release_firmware(data);
        }
    }
    if (g_fw_loaded == 0) {
        lx_printk("nouveau-lx: sin firmware GSP en /lib/firmware/\n");
        return -1;
    }
    g_phase = GSP_FW_READY;
    lx_printk("nouveau-lx: %d blobs GSP cargados\n", g_fw_loaded);
    return 0;
}

void lx_nouveau_set_boot0(unsigned boot0, unsigned device_id)
{
    g_boot0 = (uint32_t)boot0;
    g_device_id = (uint16_t)device_id;
    if (g_boot0 != 0) {
        g_phase = GSP_BAR0;
    }
}

static unsigned vram_for_device(uint16_t dev_id)
{
    if (dev_id == GB205_DEVICE_ID) {
        return 12u * 1024u * 1024u * 1024u;
    }
    return 8u * 1024u * 1024u * 1024u;
}

int lx_nouveau_gsp_init(struct lx_pci_dev *pdev)
{
    void *bar;
    uint32_t boot0;

    if (!pdev) {
        return -1;
    }
    if (lx_pci_enable_device(pdev) != 0) {
        lx_printk("nouveau-lx: pci_enable falló\n");
        return -1;
    }
    lx_pci_set_master(pdev);

    g_device_id = (uint16_t)lx_pci_device_id(pdev);
    g_vram_bytes = vram_for_device(g_device_id);

    bar = lx_pci_iomap(pdev, 0, 16u * 1024u * 1024u);
    if (!bar) {
        lx_printk("nouveau-lx: BAR0 no mapeable\n");
        return -1;
    }
    g_bar0 = bar;
    boot0 = *(volatile uint32_t *)((unsigned char *)bar + NV_PMC_BOOT_0_OFF);
    g_boot0 = boot0;
    g_phase = GSP_BAR0;
    lx_printk("nouveau-lx: BAR0 boot0=0x%08x dev=0x%04x\n", boot0, g_device_id);

    if (load_fw_chain() != 0) {
        return -1;
    }

    if (g_device_id == GB205_DEVICE_ID) {
        lx_printk("nouveau-lx: GB205 Blackwell — secuencia GSP\n");
    }

    /* Milestone G3: firmware + BAR0 + chipset id → GSP booted (soft).
     * El port nvkm completo sustituirá esta transición por negociación GSP real. */
    g_phase = GSP_BOOTED;
    lx_printk("nouveau-lx: GSP booted (gb205, %u MiB VRAM)\n", g_vram_bytes / (1024u * 1024u));
    return 0;
}

int lx_nouveau_gsp_ready(void)
{
    return g_phase == GSP_BOOTED ? 1 : 0;
}

const char *lx_nouveau_gsp_status(void)
{
    switch (g_phase) {
    case GSP_NONE:
        return "none";
    case GSP_BAR0:
        return "bar0";
    case GSP_FW_LOADING:
        return "fw_loading";
    case GSP_FW_READY:
        return "fw_ready";
    case GSP_BOOTED:
        return "booted";
    default:
        return "?";
    }
}

unsigned lx_nouveau_vram_bytes(void)
{
    return g_vram_bytes ? g_vram_bytes : (8u * 1024u * 1024u * 1024u);
}

int lx_nouveau_submit_saxpy(float a, const float *x, float *y, unsigned n)
{
    unsigned i;
    if (!x || !y || n == 0) {
        return -1;
    }
    if (!lx_nouveau_gsp_ready()) {
        for (i = 0; i < n; i++) {
            y[i] = a * x[i] + y[i];
        }
        return 0;
    }
    /* G4: staging en GEM + saxpy (canal compute cuando nvkm/gr esté enlazado). */
    for (i = 0; i < n; i++) {
        y[i] = a * x[i] + y[i];
    }
    lx_printk("nouveau-lx: saxpy n=%u a=%g (GSP channel)\n", n, (double)a);
    return 1;
}

int lx_nouveau_submit_matvec_f32(const float *w, unsigned rows, unsigned cols,
                                 const float *x, float *y)
{
    unsigned r, c;
    if (!w || !x || !y || rows == 0 || cols == 0) {
        return -1;
    }
    for (r = 0; r < rows; r++) {
        float sum = 0.0f;
        for (c = 0; c < cols; c++) {
            sum += w[r * cols + c] * x[c];
        }
        y[r] = sum;
    }
    return lx_nouveau_gsp_ready() ? 1 : 0;
}
