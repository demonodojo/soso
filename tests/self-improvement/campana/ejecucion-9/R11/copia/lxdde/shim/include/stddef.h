#ifndef LX_STDDEF_H
#define LX_STDDEF_H

typedef __SIZE_TYPE__ size_t;
typedef long ssize_t;
#define NULL ((void *)0)
#define offsetof(type, member) __builtin_offsetof(type, member)

#endif
