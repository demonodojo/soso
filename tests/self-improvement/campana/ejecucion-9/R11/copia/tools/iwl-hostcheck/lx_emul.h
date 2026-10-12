/* Stub mínimo para compilar iwl_mvm_init.c en hostcheck (sin shim stdarg). */
#ifndef LX_EMUL_H
#define LX_EMUL_H

#include <stddef.h>
#include <stdint.h>

#define GFP_KERNEL 0

void lx_printk(const char *fmt, ...);
void lx_mdelay(unsigned int ms);
void lx_udelay(unsigned int us);
void *lx_dma_alloc_coherent(void *dev, size_t size, uint64_t *dma, unsigned gfp);
void lx_dma_free_coherent(void *dev, size_t size, void *cpu, uint64_t dma);
typedef void (*lx_pci_each_fn)(uint16_t vendor, uint16_t device,
                               uint8_t pci_class, uint8_t subclass, void *ctx);
void lx_pci_for_each(lx_pci_each_fn fn, void *ctx);

#endif
