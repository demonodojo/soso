#ifndef _LX_LINUX_VMALLOC_H
#define _LX_LINUX_VMALLOC_H
#include <linux/types.h>
void *lx_vmalloc(unsigned long size);
void lx_vfree(void *ptr);
#define vmalloc(s) lx_vmalloc(s)
#define vzalloc(s) lx_vmalloc(s)
#define vfree(p) lx_vfree((void *)(p))
#endif
