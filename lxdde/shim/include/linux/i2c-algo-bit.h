#ifndef _LX_LINUX_I2C_ALGO_BIT_H
#define _LX_LINUX_I2C_ALGO_BIT_H

#include <linux/i2c.h>

extern const struct i2c_algorithm i2c_bit_algo;

static inline int i2c_bit_add_bus(struct i2c_adapter *adap)
{
	(void)adap;
	return 0;
}

#endif
