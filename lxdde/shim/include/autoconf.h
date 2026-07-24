/* Config mínima equivalente a .config de Linux para ports lxdde. */
#ifndef AUTOCONF_H
#define AUTOCONF_H

#define CONFIG_PCI 1
#define CONFIG_NET 1
#define CONFIG_ETHERNET 1
#define CONFIG_E1000E 1
#define CONFIG_SMP 0
#define CONFIG_PREEMPT_NONE 1
#define CONFIG_64BIT 1
#define CONFIG_X86_64 1
#define CONFIG_PHYS_ADDR_T_64BIT 1
#define CONFIG_DMA_DIRECT_REMAP 1
#define CONFIG_NETDEVICES 1
#define CONFIG_INET 1
#define CONFIG_DEBUG_KERNEL 1
#define CONFIG_DRM 1
#define CONFIG_DRM_NOUVEAU 1

/* nvkm (nouveau) arrastra la cadena real de cabeceras del kernel: define las
 * macros de .config que esas cabeceras esperan. Valores x86_64 estándar. */
#define CONFIG_X86_L1_CACHE_SHIFT 6
#define CONFIG_X86_INTERNODE_CACHE_SHIFT 6
#define CONFIG_NR_CPUS 1
#define CONFIG_BASE_SMALL 0
/* Paginación x86_64 de 4 niveles (evita el fallback pgtable-nopmd que choca). */
#define CONFIG_PGTABLE_LEVELS 4

#endif /* AUTOCONF_H */
