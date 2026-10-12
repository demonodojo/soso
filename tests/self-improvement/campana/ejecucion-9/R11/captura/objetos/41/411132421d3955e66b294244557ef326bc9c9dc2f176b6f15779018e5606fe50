/* Reemplazo mínimo estilo lx_emul de <linux/pci.h>.
 *
 * El pci.h real arrastra ioport/kobject/sysfs/idr/xarray/cpumask/mmzone y el
 * core arch x86. El subdev GSP nvkm solo necesita punteros opacos a pci_dev.
 * El acceso PCI real vive en la capa lx_ (kernel/src/lxdde/pci.rs).
 */
#ifndef _LX_LINUX_PCI_H
#define _LX_LINUX_PCI_H

#include <linux/types.h>

struct pci_dev;
struct pci_bus;
struct pci_driver;
struct pci_device_id {
	u32 vendor, device, subvendor, subdevice, class, class_mask;
	unsigned long driver_data;
};

#define PCI_VENDOR_ID_NVIDIA 0x10de
#define PCI_ANY_ID (~0)

#define PCI_DEVICE(vend, dev) \
	.vendor = (vend), .device = (dev), \
	.subvendor = PCI_ANY_ID, .subdevice = PCI_ANY_ID

static inline int pci_enable_msi(struct pci_dev *d) { (void)d; return -1; }
static inline void pci_disable_msi(struct pci_dev *d) { (void)d; }
static inline int pci_is_pcie(struct pci_dev *d) { (void)d; return 1; }
#endif /* _LX_LINUX_PCI_H */
