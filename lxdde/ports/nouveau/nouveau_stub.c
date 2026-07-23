/* G3: stub nouveau — probe PCI NVIDIA + log GSP pendiente. */
#include "lx_emul.h"

static int nouveau_probe(struct lx_pci_dev *pdev, const struct lx_pci_device_id *id)
{
    (void)id;
    lx_printk("nouveau-lx: probe NVIDIA dev=%u\n", (unsigned)lx_pci_device_id(pdev));
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
