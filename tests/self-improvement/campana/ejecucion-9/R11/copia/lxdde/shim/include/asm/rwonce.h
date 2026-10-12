#ifndef _ASM_RWONCE_H
#define _ASM_RWONCE_H

#define __READ_ONCE(x) (*(const volatile typeof(x) *)&(x))
#define __WRITE_ONCE(x, val) do { *(volatile typeof(x) *)&(x) = (val); } while (0)
#define READ_ONCE(x) __READ_ONCE(x)
#define WRITE_ONCE(x, val) __WRITE_ONCE(x, val)

#endif
