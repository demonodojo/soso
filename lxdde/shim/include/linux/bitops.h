#ifndef _LX_LINUX_BITOPS_H
#define _LX_LINUX_BITOPS_H
#include <linux/types.h>
#include <asm/bitsperlong.h>
#ifndef BIT
#define BIT(nr) (1UL << (nr))
#endif
#define BIT_ULL(nr) (1ULL << (nr))
#define BITS_PER_BYTE 8
#define GENMASK(h, l) (((~0UL) << (l)) & (~0UL >> (BITS_PER_LONG - 1 - (h))))
#define GENMASK_ULL(h, l) (((~0ULL) << (l)) & (~0ULL >> (64 - 1 - (h))))
#define BITS_TO_LONGS(nr) (((nr) + BITS_PER_LONG - 1) / BITS_PER_LONG)
static inline unsigned int hweight32(u32 w)
{ w -= (w >> 1) & 0x55555555; w = (w & 0x33333333) + ((w >> 2) & 0x33333333);
  w = (w + (w >> 4)) & 0x0f0f0f0f; return (w * 0x01010101) >> 24; }
#endif
