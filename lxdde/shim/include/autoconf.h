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

#endif /* AUTOCONF_H */
