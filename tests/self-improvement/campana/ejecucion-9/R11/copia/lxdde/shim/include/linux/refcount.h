#ifndef _LX_LINUX_REFCOUNT_H
#define _LX_LINUX_REFCOUNT_H
typedef struct refcount_struct { int refs; } refcount_t;
static inline void refcount_set(refcount_t *r, int n) { r->refs = n; }
static inline int refcount_read(const refcount_t *r) { return r->refs; }
static inline void refcount_inc(refcount_t *r) { r->refs++; }
static inline int refcount_dec_and_test(refcount_t *r) { return --r->refs == 0; }
static inline int refcount_inc_not_zero(refcount_t *r) { if (r->refs == 0) return 0; r->refs++; return 1; }
static inline int refcount_dec_and_test_(refcount_t *r) { return --r->refs == 0; }
#include <linux/mutex.h>
static inline int refcount_dec_and_mutex_lock(refcount_t *r, struct mutex *m) { mutex_lock(m); if (--r->refs == 0) return 1; mutex_unlock(m); return 0; }
#endif
