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
#define likely_notrace(x)   likely(x)
#define unlikely_notrace(x) unlikely(x)
#ifndef barrier_data
#define barrier_data(ptr) __asm__ __volatile__("" : : "r"(ptr) : "memory")
#endif

#define READ_ONCE(x) (*(const volatile typeof(x) *)&(x))
#define WRITE_ONCE(x, val) (*(volatile typeof(x) *)&(x) = (val))

/* nvkm arrastra cabeceras reales que usan estos builtins/atributos. */
#ifndef barrier
#define barrier() __asm__ __volatile__("" : : : "memory")
#endif
#ifndef RELOC_HIDE
#define RELOC_HIDE(ptr, off)                    \
	({                                      \
		unsigned long __ptr;            \
		__ptr = (unsigned long)(ptr);   \
		(typeof(ptr))(__ptr + (off));   \
	})
#endif
#ifndef OPTIMIZER_HIDE_VAR
#define OPTIMIZER_HIDE_VAR(var) __asm__ __volatile__("" : "=r"(var) : "0"(var))
#endif
#ifndef __always_inline
#define __always_inline inline __attribute__((__always_inline__))
#endif
#ifndef noinline
#define noinline __attribute__((__noinline__))
#endif
#ifndef __maybe_unused
#define __maybe_unused __attribute__((__unused__))
#endif
#ifndef __always_unused
#define __always_unused __attribute__((__unused__))
#endif
#ifndef __used
#define __used __attribute__((__used__))
#endif
#ifndef __section
#define __section(s) __attribute__((__section__(s)))
#endif
#ifndef __aligned
#define __aligned(x) __attribute__((__aligned__(x)))
#endif
#ifndef __packed
#define __packed __attribute__((__packed__))
#endif
#ifndef fallthrough
#define fallthrough __attribute__((__fallthrough__))
#endif

#endif
