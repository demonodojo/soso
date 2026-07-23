#ifndef __LINUX_COMPILER_H
#define __LINUX_COMPILER_H

#define __user
#define __kernel
#define __force
#define __iomem
#define __must_check
#define __init
#define __exit
#define __devinit
#define __devinitdata
#define __devinitconst
#define __devexit
#define __devexitdata
#define __devexitconst
#define __meminit
#define __meminitdata
#define __ref
#define __refdata
#define __refconst

#define likely(x)   __builtin_expect(!!(x), 1)
#define unlikely(x) __builtin_expect(!!(x), 0)

#define READ_ONCE(x) (*(const volatile typeof(x) *)&(x))
#define WRITE_ONCE(x, val) (*(volatile typeof(x) *)&(x) = (val))

#endif
