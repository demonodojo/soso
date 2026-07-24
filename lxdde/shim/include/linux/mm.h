#ifndef _LX_LINUX_MM_H
#define _LX_LINUX_MM_H
#include <linux/types.h>
#include <linux/slab.h>
struct page;
struct page *alloc_page(unsigned gfp);
void __free_page(struct page *p);
void *page_address(const struct page *p);
unsigned long page_to_pfn(const struct page *p);
struct page *pfn_to_page(unsigned long pfn);
#define alloc_pages(gfp, order) alloc_page(gfp)
#define __free_pages(p, order) __free_page(p)
#define free_page(addr) lx_kfree((void *)(addr))
#define get_order(n) 0
#define kvcalloc(n, size, flags) lx_kzalloc((size_t)(n) * (size), (flags))
#define kvmalloc_array(n, size, flags) lx_kmalloc((size_t)(n) * (size), (flags))
#endif
