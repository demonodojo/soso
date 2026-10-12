#ifndef _LX_LINUX_SCATTERLIST_H
#define _LX_LINUX_SCATTERLIST_H
#include <linux/types.h>
struct scatterlist { unsigned long page_link; unsigned int offset, length; dma_addr_t dma_address; };
struct sg_table { struct scatterlist *sgl; unsigned int nents, orig_nents; };
#define sg_dma_address(sg) ((sg)->dma_address)
#define sg_dma_len(sg) ((sg)->length)
static inline void sg_init_one(struct scatterlist *sg, const void *buf, unsigned int len)
{ sg->page_link = (unsigned long)buf; sg->offset = 0; sg->length = len; sg->dma_address = 0; }
static inline void *sg_virt(struct scatterlist *sg) { return (void *)sg->page_link; }
static inline struct scatterlist *sg_next(struct scatterlist *sg) { return sg + 1; }
#define for_each_sg(sglist, sg, nr, i) for ((i) = 0, (sg) = (sglist); (i) < (nr); (i)++, (sg) = sg_next(sg))
#endif
