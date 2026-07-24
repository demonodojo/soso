#ifndef _LX_LINUX_IO_H
#define _LX_LINUX_IO_H
#include <linux/types.h>
static inline u32 ioread32(const volatile void *p) { return *(const volatile u32 *)p; }
static inline u16 ioread16(const volatile void *p) { return *(const volatile u16 *)p; }
static inline u8 ioread8(const volatile void *p) { return *(const volatile u8 *)p; }
static inline void iowrite32(u32 v, volatile void *p) { *(volatile u32 *)p = v; }
static inline void iowrite16(u16 v, volatile void *p) { *(volatile u16 *)p = v; }
static inline void iowrite8(u8 v, volatile void *p) { *(volatile u8 *)p = v; }
static inline void memcpy_toio(volatile void *d, const void *s, size_t n) { volatile char *dd=d; const char *ss=s; size_t i; for(i=0;i<n;i++) dd[i]=ss[i]; }
static inline void memcpy_fromio(void *d, const volatile void *s, size_t n) { char *dd=d; const volatile char *ss=s; size_t i; for(i=0;i<n;i++) dd[i]=ss[i]; }
static inline void memset_io(volatile void *d, int c, size_t n) { volatile char *dd=d; size_t i; for(i=0;i<n;i++) dd[i]=(char)c; }
#endif
