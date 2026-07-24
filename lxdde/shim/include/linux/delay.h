#ifndef _LX_LINUX_DELAY_H
#define _LX_LINUX_DELAY_H
void lx_udelay(unsigned long us);
void lx_mdelay(unsigned long ms);
void lx_msleep(unsigned int ms);
#define udelay(u) lx_udelay(u)
#define mdelay(m) lx_mdelay(m)
#define msleep(m) lx_msleep(m)
#define usleep_range(a, b) lx_udelay(a)
#endif
