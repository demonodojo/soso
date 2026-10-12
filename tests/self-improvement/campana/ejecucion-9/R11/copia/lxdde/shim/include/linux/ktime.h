#ifndef _LX_LINUX_KTIME_H
#define _LX_LINUX_KTIME_H
#include <linux/types.h>
typedef s64 ktime_t;
unsigned long long lx_ktime_get_ns(void);
static inline ktime_t ktime_get(void) { return (ktime_t)lx_ktime_get_ns(); }
static inline ktime_t ktime_get_raw(void) { return (ktime_t)lx_ktime_get_ns(); }
static inline s64 ktime_to_ns(ktime_t k) { return k; }
static inline s64 ktime_to_us(ktime_t k) { return k / 1000; }
static inline s64 ktime_to_ms(ktime_t k) { return k / 1000000; }
static inline ktime_t ktime_sub(ktime_t a, ktime_t b) { return a - b; }
static inline ktime_t ktime_add_ns(ktime_t a, s64 ns) { return a + ns; }
static inline s64 ktime_us_delta(ktime_t a, ktime_t b) { return (a - b) / 1000; }
#endif
