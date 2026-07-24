#ifndef _LINUX_MODULE_H
#define _LINUX_MODULE_H

#define MODULE_LICENSE(x)
#define MODULE_AUTHOR(x)
#define MODULE_DESCRIPTION(x)
#define MODULE_DEVICE_TABLE(type, x)
#define MODULE_PARM_DESC(x, y)
#define MODULE_FIRMWARE(x)
#define MODULE_ALIAS(x)
#define MODULE_VERSION(x)
#define THIS_MODULE ((struct module *)0)

#define module_param(name, type, perm) static type name
#define module_init(x) static void lx_modinit_##x(void) { (void)x; }
#define module_exit(x)
#define EXPORT_SYMBOL(x)
#define EXPORT_SYMBOL_GPL(x)

struct module;

#endif
