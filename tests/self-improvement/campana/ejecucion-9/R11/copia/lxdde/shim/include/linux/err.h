#ifndef _LX_LINUX_ERR_H
#define _LX_LINUX_ERR_H
#include <linux/types.h>
#define MAX_ERRNO 4095
#define IS_ERR_VALUE(x) ((unsigned long)(void *)(x) >= (unsigned long)-MAX_ERRNO)
static inline void *ERR_PTR(long e) { return (void *)e; }
static inline long PTR_ERR(const void *p) { return (long)p; }
static inline bool IS_ERR(const void *p) { return IS_ERR_VALUE((unsigned long)p); }
static inline bool IS_ERR_OR_NULL(const void *p) { return !p || IS_ERR(p); }
static inline void *ERR_CAST(const void *p) { return (void *)p; }
static inline int PTR_ERR_OR_ZERO(const void *p) { return IS_ERR(p) ? (int)PTR_ERR(p) : 0; }
#endif
