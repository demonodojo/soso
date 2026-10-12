#ifndef _LX_LINUX_ATOMIC_H
#define _LX_LINUX_ATOMIC_H
#include <linux/types.h>
/* Monocore: operaciones atómicas = accesos simples. */
static inline int atomic_read(const atomic_t *v) { return v->counter; }
static inline void atomic_set(atomic_t *v, int i) { v->counter = i; }
static inline void atomic_inc(atomic_t *v) { v->counter++; }
static inline void atomic_dec(atomic_t *v) { v->counter--; }
static inline int atomic_add_return(int i, atomic_t *v) { v->counter += i; return v->counter; }
static inline int atomic_dec_and_test(atomic_t *v) { return --v->counter == 0; }
static inline int atomic_xchg(atomic_t *v, int n) { int o = v->counter; v->counter = n; return o; }
static inline int atomic_cmpxchg(atomic_t *v, int o, int n) { int c = v->counter; if (c == o) v->counter = n; return c; }
static inline int atomic_inc_return(atomic_t *v) { return ++v->counter; }
static inline int atomic_dec_return(atomic_t *v) { return --v->counter; }
static inline int atomic_fetch_inc(atomic_t *v) { return v->counter++; }
#endif
