#ifndef _LX_LINUX_MATH64_H
#define _LX_LINUX_MATH64_H

/* Suficiente para nvkm disp (nv50): divide n entre base y devuelve el resto. */
#define do_div(n, base) ({ \
	typeof(base) __base = (base); \
	typeof(n) __rem = (n) % __base; \
	(n) /= __base; \
	__rem; \
})

#endif
