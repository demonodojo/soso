#ifndef _LX_LINUX_FIRMWARE_H
#define _LX_LINUX_FIRMWARE_H
#include <linux/types.h>
struct device;
struct firmware { size_t size; const u8 *data; void *priv; };
int lx_request_firmware(const struct firmware **fw, const char *name, void *dev);
void lx_release_firmware(const struct firmware *fw);
#define request_firmware(fw, name, dev) lx_request_firmware((fw), (name), (dev))
#define release_firmware(fw) lx_release_firmware(fw)
#endif
