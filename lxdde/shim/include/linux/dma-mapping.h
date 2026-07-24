#ifndef _LX_LINUX_DMA_MAPPING_H
#define _LX_LINUX_DMA_MAPPING_H
#include <linux/types.h>
struct device;
void *lx_dma_alloc_coherent(unsigned long size, dma_addr_t *dma);
void lx_dma_free_coherent(void *va, unsigned long size, dma_addr_t dma);
#define dma_alloc_coherent(dev, size, dma, gfp) lx_dma_alloc_coherent((size), (dma))
#define dma_free_coherent(dev, size, va, dma)   lx_dma_free_coherent((va), (size), (dma))
#define dma_map_page(dev, p, off, size, dir)    ((dma_addr_t)0)
#define dma_mapping_error(dev, addr)            (0)
enum dma_data_direction { DMA_BIDIRECTIONAL=0, DMA_TO_DEVICE=1, DMA_FROM_DEVICE=2, DMA_NONE=3 };
#define dma_unmap_page(dev, addr, size, dir) do { (void)(addr); } while (0)
#define dma_map_single(dev, ptr, size, dir) ((dma_addr_t)(unsigned long)(ptr))
#define dma_unmap_single(dev, addr, size, dir) do { (void)(addr); } while (0)
#define dma_sync_single_for_device(dev, addr, size, dir) do {} while (0)
#define dma_sync_single_for_cpu(dev, addr, size, dir) do {} while (0)
#define dma_set_mask_and_coherent(dev, mask) (0)
#endif
