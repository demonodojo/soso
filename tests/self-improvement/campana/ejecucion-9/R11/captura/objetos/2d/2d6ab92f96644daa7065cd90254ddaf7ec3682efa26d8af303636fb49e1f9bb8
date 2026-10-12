/* Reemplazo mínimo estilo lx_emul de <linux/slab.h>.
 *
 * El slab.h real arrastra gfp.h→mmzone.h→spinlock.h→preempt.h→thread_info.h→
 * processor.h (core arch x86: per_cpu, current, cpufeature). nvkm no necesita
 * nada de eso: solo asignación de memoria. Mapeamos a las primitivas lx_*.
 */
#ifndef _LX_LINUX_SLAB_H
#define _LX_LINUX_SLAB_H

#include <linux/types.h>
#include <linux/stddef.h>
#include <linux/errno.h>
#include <linux/err.h>
#include <linux/string.h>
#include <linux/spinlock.h>
#include <linux/refcount.h>
#include <linux/kref.h>
#include <linux/bug.h>
#include <linux/notifier.h>
#include <linux/scatterlist.h>
#include <linux/list.h>
#include <linux/rbtree.h>
#include <linux/ktime.h>
#include <linux/kernel.h>
#include <linux/atomic.h>
#include <linux/ctype.h>
#include <linux/dma-mapping.h>
#include <linux/mm.h>
#include <linux/completion.h>
#include <linux/workqueue.h>
#include <linux/wait.h>

#ifndef container_of
#define container_of(ptr, type, member) \
	((type *)((char *)(ptr) - offsetof(type, member)))
#endif

void *lx_kmalloc(size_t size, unsigned flags);
void *lx_kzalloc(size_t size, unsigned flags);
void *lx_krealloc(void *ptr, size_t size, unsigned flags);
void lx_kfree(void *ptr);
void *lx_vmalloc(unsigned long size);
void lx_vfree(void *ptr);

#ifndef GFP_KERNEL
#define GFP_KERNEL 0x40u
#define GFP_ATOMIC 0x20u
#define __GFP_ZERO 0x8000u
#endif
#ifndef GFP_NOWAIT
#define GFP_NOWAIT 0u
#endif
#ifndef GFP_USER
#define GFP_USER 0x100u
#define GFP_HIGHUSER 0x110u
#define GFP_DMA32 0x04u
#define __GFP_HIGH 0x20u
#define __GFP_NOWARN 0x200u
#endif

#define kmalloc(size, flags)        lx_kmalloc((size), (flags))
#define kzalloc(size, flags)        lx_kzalloc((size), (flags))
#define krealloc(ptr, size, flags)  lx_krealloc((ptr), (size), (flags))
#define kfree(ptr)                  lx_kfree((void *)(ptr))
#define kvfree(ptr)                 lx_kfree((void *)(ptr))

#define kcalloc(n, size, flags)     lx_kzalloc((size_t)(n) * (size), (flags))
#define kmalloc_array(n, size, fl)  lx_kmalloc((size_t)(n) * (size), (fl))
#define kzalloc_node(size, fl, nd)  lx_kzalloc((size), (fl))
#define kmalloc_node(size, fl, nd)  lx_kmalloc((size), (fl))
#define kvmalloc(size, flags)       lx_vmalloc((unsigned long)(size))
#define kvzalloc(size, flags)       lx_kzalloc((size), (flags) | __GFP_ZERO)

static inline char *kstrndup(const char *s, size_t max, unsigned flags)
{
	size_t len = 0;
	char *out;
	while (len < max && s[len])
		len++;
	out = (char *)lx_kmalloc(len + 1, flags);
	if (out) {
		size_t i;
		for (i = 0; i < len; i++)
			out[i] = s[i];
		out[len] = '\0';
	}
	return out;
}

static inline char *kstrdup(const char *s, unsigned flags)
{
	size_t len = 0;
	while (s[len])
		len++;
	return kstrndup(s, len, flags);
}

static inline void *kmemdup(const void *src, size_t len, unsigned flags)
{ void *p = lx_kmalloc(len, flags); if (p) { size_t i; const char *s = src; char *d = p; for (i = 0; i < len; i++) d[i] = s[i]; } return p; }
#endif /* _LX_LINUX_SLAB_H */
