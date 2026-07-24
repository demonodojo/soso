/* Incluido vía -include: desactiva trace/ptp para ports lxdde. */
#ifndef LX_LINUX_COMPAT_H
#define LX_LINUX_COMPAT_H

#define KBUILD_MODNAME "e1000e"
#define KBUILD_BASENAME "e1000e"

/* Tipos base disponibles en toda TU (p.ej. lib/rbtree.c no pasa por el prelude
 * de slab.h que arrastra <linux/types.h>). */
#include <linux/types.h>

#undef CREATE_TRACE_POINTS

#endif
