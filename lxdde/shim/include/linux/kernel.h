#ifndef _LX_LINUX_KERNEL_H
#define _LX_LINUX_KERNEL_H
#include <linux/types.h>
#include <linux/compiler.h>
#include <linux/bug.h>

#ifndef PAGE_SHIFT
#define PAGE_SHIFT 12
#define PAGE_SIZE  (1UL << PAGE_SHIFT)
#define PAGE_MASK  (~(PAGE_SIZE - 1))
#endif

#define ALIGN(x, a)        (((x) + ((a) - 1)) & ~((typeof(x))(a) - 1))
#define ALIGN_DOWN(x, a)   ((x) & ~((typeof(x))(a) - 1))
#define IS_ALIGNED(x, a)   (((x) & ((typeof(x))(a) - 1)) == 0)
#define PAGE_ALIGN(x)      ALIGN((x), PAGE_SIZE)
#define DIV_ROUND_UP(n, d) (((n) + (d) - 1) / (d))
#define roundup(x, y)      (DIV_ROUND_UP((x), (y)) * (y))
#define rounddown(x, y)    (((x) / (y)) * (y))

#ifndef min
#define min(a, b) ((a) < (b) ? (a) : (b))
#define max(a, b) ((a) > (b) ? (a) : (b))
#endif
#define min_t(t, a, b) ((t)(a) < (t)(b) ? (t)(a) : (t)(b))
#define max_t(t, a, b) ((t)(a) > (t)(b) ? (t)(a) : (t)(b))
#define clamp(v, lo, hi) max((lo), min((v), (hi)))
#define abs(x) ((x) < 0 ? -(x) : (x))
#define swap(a, b) do { typeof(a) __t = (a); (a) = (b); (b) = __t; } while (0)

#define ARRAY_SIZE(a) (sizeof(a) / sizeof((a)[0]))
#define struct_size(p, member, n) (sizeof(*(p)) + (n) * sizeof(*(p)->member))
#define array3_size(a, b, c) ((size_t)(a) * (b) * (c))
#define array_size(a, b) ((size_t)(a) * (b))

#define lower_32_bits(n) ((u32)((n) & 0xffffffff))
#define upper_32_bits(n) ((u32)(((n) >> 16) >> 16))
#define BITS_PER_TYPE(t) (sizeof(t) * 8)

static inline unsigned long __ffs(unsigned long word) { return __builtin_ctzl(word); }
static inline unsigned long __fls(unsigned long word) { return 63 - __builtin_clzl(word); }
static inline int ffs(int x) { return x ? __builtin_ctz(x) + 1 : 0; }
static inline int fls(unsigned int x) { return x ? 32 - __builtin_clz(x) : 0; }
static inline int fls64(u64 x) { return x ? 64 - __builtin_clzll(x) : 0; }
#endif
