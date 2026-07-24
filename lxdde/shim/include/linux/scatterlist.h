#ifndef _LX_LINUX_SCATTERLIST_H
#define _LX_LINUX_SCATTERLIST_H
#include <linux/types.h>
struct scatterlist { unsigned long page_link; unsigned int offset, length; dma_addr_t dma_address; };
struct sg_table { struct scatterlist *sgl; unsigned int nents, orig_nents; };
#endif
