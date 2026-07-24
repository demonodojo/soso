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

/* dev_*: ignoran el `struct device*` y delegan en lx_printk. */
#define dev_printk(level, dev, fmt, ...) lx_printk(fmt, ##__VA_ARGS__)
#define dev_crit(dev, fmt, ...)   lx_printk(fmt, ##__VA_ARGS__)
#define dev_err(dev, fmt, ...)    lx_printk(fmt, ##__VA_ARGS__)
#define dev_warn(dev, fmt, ...)   lx_printk(fmt, ##__VA_ARGS__)
#define dev_notice(dev, fmt, ...) lx_printk(fmt, ##__VA_ARGS__)
#define dev_info(dev, fmt, ...)   lx_printk(fmt, ##__VA_ARGS__)
#define dev_dbg(dev, fmt, ...)    lx_printk(fmt, ##__VA_ARGS__)
#define dev_err_ratelimited(dev, fmt, ...) lx_printk(fmt, ##__VA_ARGS__)

#define pr_cont(fmt, ...)   lx_printk(fmt, ##__VA_ARGS__)
#define pr_notice(fmt, ...) lx_printk(fmt, ##__VA_ARGS__)
#define pr_crit(fmt, ...)   lx_printk(fmt, ##__VA_ARGS__)

#define KERN_SOH ""
#define KERN_EMERG ""
#define KERN_ALERT ""
#define KERN_CRIT ""
#define KERN_ERR ""
#define KERN_WARNING ""
#define KERN_NOTICE ""
#define KERN_INFO ""
#define KERN_DEBUG ""
#define KERN_CONT ""
#define dev_WARN(dev, fmt, ...) ({ lx_printk(fmt, ##__VA_ARGS__); 1; })
#endif
