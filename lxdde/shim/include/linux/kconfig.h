/* Stub kconfig.h — autoconf.h ya define CONFIG_*. */
#ifndef __LINUX_KCONFIG_H
#define __LINUX_KCONFIG_H

#define __ARG_PLACEHOLDER_1 0,
#define config_enabled(cfg) _config_enabled(cfg)
#define _config_enabled(value) __config_enabled(__ARG_PLACEHOLDER_##value)
#define __config_enabled(arg1_or_junk) ___config_enabled(arg1_or_junk 1, 0)
#define ___config_enabled(arg1, arg2, ...) arg2

#endif
