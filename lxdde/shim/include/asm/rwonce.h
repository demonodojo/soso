#ifndef _ASM_RWONCE_H
#define _ASM_RWONCE_H

#define READ_ONCE(x) (*(const volatile typeof(x) *)&(x))
#define WRITE_ONCE(x, val) (*(volatile typeof(x) *)&(x) = (val))

#endif
