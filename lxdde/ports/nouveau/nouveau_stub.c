/* G3: probe NVIDIA + GSP bring-up gb205. */
#include "lx_emul.h"

extern void lx_nouveau_set_boot0(unsigned boot0, unsigned device_id);
extern int lx_nouveau_gsp_init(struct lx_pci_dev *pdev);
extern int lx_nouveau_gsp_ready(void);
extern const char *lx_nouveau_gsp_status(void);
extern int lx_nouveau_submit_saxpy(float a, const float *x, float *y, unsigned n);
extern int lx_nouveau_submit_matvec_f32(const float *w, unsigned rows, unsigned cols,
                                        const float *x, float *y);
extern unsigned lx_nouveau_vram_bytes(void);

static int nouveau_probe(struct lx_pci_dev *pdev, const struct lx_pci_device_id *id)
{
    (void)id;
    unsigned dev = (unsigned)lx_pci_device_id(pdev);
    lx_printk("nouveau-lx: probe NVIDIA dev=0x%04x\n", dev);
    if (lx_nouveau_gsp_init(pdev) != 0) {
        lx_printk("nouveau-lx: GSP init falló (status=%s)\n", lx_nouveau_gsp_status());
        return -1;
    }
    return 0;
}

static void nouveau_remove(struct lx_pci_dev *pdev)
{
    (void)pdev;
}

static const struct lx_pci_device_id nouveau_ids[] = {
    { 0x10de, 0, 0, 0, 0, 0, 0 },
    { 0, 0, 0, 0, 0, 0, 0 },
};

int lx_nouveau_init_module(void)
{
    return lx_pci_register_driver("nouveau", nouveau_ids, nouveau_probe, nouveau_remove);
}

int lx_nouveau_gsp_is_ready(void)
{
    return lx_nouveau_gsp_ready();
}

const char *lx_nouveau_gsp_phase(void)
{
    return lx_nouveau_gsp_status();
}

int lx_nouveau_compute_saxpy(float a, const float *x, float *y, unsigned n)
{
    return lx_nouveau_submit_saxpy(a, x, y, n);
}

int lx_nouveau_compute_matvec_f32(const float *w, unsigned rows, unsigned cols,
                                  const float *x, float *y)
{
    return lx_nouveau_submit_matvec_f32(w, rows, cols, x, y);
}

unsigned lx_nouveau_vram_total(void)
{
    return lx_nouveau_vram_bytes();
}
