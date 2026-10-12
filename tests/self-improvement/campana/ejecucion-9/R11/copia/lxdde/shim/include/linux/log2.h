#ifndef _LX_LINUX_LOG2_H
#define _LX_LINUX_LOG2_H
#include <linux/types.h>
static inline int is_power_of_2(unsigned long n) { return n != 0 && (n & (n - 1)) == 0; }
static inline unsigned long __ilog2_u64(u64 n) { unsigned long r = 0; while (n >>= 1) r++; return r; }
#define ilog2(n) __ilog2_u64((u64)(n))
static inline unsigned long roundup_pow_of_two(unsigned long n)
{ unsigned long p = 1; while (p < n) p <<= 1; return p; }
#define order_base_2(n) ilog2(roundup_pow_of_two(n))
#endif
