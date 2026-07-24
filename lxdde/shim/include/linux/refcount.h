#ifndef _LX_LINUX_REFCOUNT_H
#define _LX_LINUX_REFCOUNT_H
typedef struct refcount_struct { int refs; } refcount_t;
static inline void refcount_set(refcount_t *r, int n) { r->refs = n; }
static inline int refcount_read(const refcount_t *r) { return r->refs; }
static inline void refcount_inc(refcount_t *r) { r->refs++; }
static inline int refcount_dec_and_test(refcount_t *r) { return --r->refs == 0; }
#endif
