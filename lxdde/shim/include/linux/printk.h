#ifndef _LINUX_PRINTK_H
#define _LINUX_PRINTK_H

#include <stdarg.h>

int lx_printk(const char *fmt, ...);

#define printk lx_printk
#define pr_info(fmt, ...) lx_printk(fmt, ##__VA_ARGS__)
#define pr_err(fmt, ...) lx_printk(fmt, ##__VA_ARGS__)
#define pr_warn(fmt, ...) lx_printk(fmt, ##__VA_ARGS__)
#define pr_debug(fmt, ...) lx_printk(fmt, ##__VA_ARGS__)

#define no_printk(fmt, ...)             \
	({                              \
		if (0)                  \
			lx_printk(fmt, ##__VA_ARGS__); \
		0;                      \
	})

#endif
