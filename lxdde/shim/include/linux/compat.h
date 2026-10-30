/* Incluido vía -include: desactiva trace/ptp para ports lxdde. */
#ifndef LX_LINUX_COMPAT_H
#define LX_LINUX_COMPAT_H

#define KBUILD_MODNAME "iwlwifi"
#define KBUILD_BASENAME "iwlwifi"

/* Tipos base disponibles en toda TU (p.ej. lib/rbtree.c no pasa por el prelude
 * de slab.h que arrastra <linux/types.h>). */
#include <linux/types.h>
#include <linux/math64.h>

struct device {
	struct device *parent;
};

static inline const char *dev_name(const struct device *dev)
{
	(void)dev;
	return "nouveau-lx";
}

#define usecs_to_jiffies(usec) ((unsigned long)(((usec) + 999) / 1000))

#undef CREATE_TRACE_POINTS

#endif
