#ifndef _LX_LINUX_NOTIFIER_H
#define _LX_LINUX_NOTIFIER_H
struct notifier_block;
typedef int (*notifier_fn_t)(struct notifier_block *, unsigned long, void *);
struct notifier_block { notifier_fn_t notifier_call; struct notifier_block *next; int priority; };
#endif
