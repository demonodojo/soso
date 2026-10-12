#ifndef _LX_LINUX_KREF_H
#define _LX_LINUX_KREF_H
#include <linux/refcount.h>
struct kref { refcount_t refcount; };
static inline void kref_init(struct kref *k) { refcount_set(&k->refcount, 1); }
static inline void kref_get(struct kref *k) { refcount_inc(&k->refcount); }
static inline int kref_put(struct kref *k, void (*rel)(struct kref *))
{ if (refcount_dec_and_test(&k->refcount)) { rel(k); return 1; } return 0; }
#endif
