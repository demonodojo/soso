/* Stub kconfig.h — autoconf.h ya define CONFIG_*. */
#ifndef __LINUX_KCONFIG_H
#define __LINUX_KCONFIG_H

#define __ARG_PLACEHOLDER_1 0,
#define config_enabled(cfg) _config_enabled(cfg)
#define _config_enabled(value) __config_enabled(__ARG_PLACEHOLDER_##value)
#define __config_enabled(arg1_or_junk) ___config_enabled(arg1_or_junk 1, 0)
#define ___config_enabled(arg1, arg2, ...) arg2

/* Familia IS_ENABLED/IS_BUILTIN (preprocesador puro, idéntica a la real). */
#define __take_second_arg(__ignored, val, ...) val
#define __is_defined(x)          ___is_defined(x)
#define ___is_defined(val)       ____is_defined(__ARG_PLACEHOLDER_##val)
#define ____is_defined(arg1_or_junk) __take_second_arg(arg1_or_junk 1, 0)
#define IS_BUILTIN(option)       __is_defined(option)
#define IS_MODULE(option)        __is_defined(option##_MODULE)
#define IS_REACHABLE(option)     IS_BUILTIN(option)
#define IS_ENABLED(option)       (IS_BUILTIN(option) || IS_MODULE(option))

#endif
