/* stub lx_emul: i2c para nvkm disp/outp + subdev/i2c (N3 ola 2). */
#ifndef _LX_LINUX_I2C_H
#define _LX_LINUX_I2C_H

#include <linux/types.h>
#include <stdbool.h>

#define I2C_M_RD 0x0001
#define I2C_NAME_SIZE 20

#define I2C_FUNC_I2C 0x00000001
#define I2C_FUNC_SMBUS_QUICK 0x00010000
#define I2C_FUNC_SMBUS_READ_BYTE 0x00020000
#define I2C_FUNC_SMBUS_WRITE_BYTE 0x00040000
#define I2C_FUNC_SMBUS_READ_BYTE_DATA 0x00080000
#define I2C_FUNC_SMBUS_WRITE_BYTE_DATA 0x00100000
#define I2C_FUNC_SMBUS_READ_WORD_DATA 0x00200000
#define I2C_FUNC_SMBUS_WRITE_WORD_DATA 0x00400000
#define I2C_FUNC_SMBUS_PROC_CALL 0x00800000
#define I2C_FUNC_SMBUS_READ_BLOCK_DATA 0x01000000
#define I2C_FUNC_SMBUS_WRITE_BLOCK_DATA 0x02000000
#define I2C_FUNC_SMBUS_BYTE (I2C_FUNC_SMBUS_READ_BYTE | I2C_FUNC_SMBUS_WRITE_BYTE)
#define I2C_FUNC_SMBUS_BYTE_DATA \
	(I2C_FUNC_SMBUS_READ_BYTE_DATA | I2C_FUNC_SMBUS_WRITE_BYTE_DATA)
#define I2C_FUNC_SMBUS_WORD_DATA \
	(I2C_FUNC_SMBUS_READ_WORD_DATA | I2C_FUNC_SMBUS_WRITE_WORD_DATA)
#define I2C_FUNC_SMBUS_BLOCK_DATA \
	(I2C_FUNC_SMBUS_READ_BLOCK_DATA | I2C_FUNC_SMBUS_WRITE_BLOCK_DATA)
#define I2C_FUNC_SMBUS_EMUL \
	(I2C_FUNC_SMBUS_QUICK | I2C_FUNC_SMBUS_BYTE | I2C_FUNC_SMBUS_BYTE_DATA | \
	 I2C_FUNC_SMBUS_WORD_DATA | I2C_FUNC_SMBUS_PROC_CALL | I2C_FUNC_SMBUS_BLOCK_DATA)

struct module;
struct device;
struct i2c_algorithm;

struct i2c_msg {
	u16 addr;
	u16 flags;
	u16 len;
	u8 *buf;
};

struct i2c_adapter {
	char name[48];
	struct module *owner;
	void *algo_data;
	const struct i2c_algorithm *algo;
	struct device dev;
};

struct i2c_algorithm {
	int (*master_xfer)(struct i2c_adapter *adap, struct i2c_msg *msgs, int num);
	u32 (*functionality)(struct i2c_adapter *adap);
};

struct i2c_algo_bit_data {
	void *data;
	void (*setsda)(void *data, int state);
	void (*setscl)(void *data, int state);
	int (*getsda)(void *data);
	int (*getscl)(void *data);
	int (*pre_xfer)(struct i2c_adapter *);
	void (*post_xfer)(struct i2c_adapter *);
	int udelay;
	int timeout;
	bool can_do_atomic;
};

struct i2c_board_info {
	char type[I2C_NAME_SIZE];
	unsigned short flags;
	unsigned short addr;
	const char *dev_name;
	void *platform_data;
};

static inline int i2c_transfer(struct i2c_adapter *adap, struct i2c_msg *msgs, int num)
{
	(void)adap;
	(void)msgs;
	(void)num;
	return -1;
}

static inline int i2c_add_adapter(struct i2c_adapter *adap)
{
	(void)adap;
	return 0;
}

static inline void i2c_del_adapter(struct i2c_adapter *adap)
{
	(void)adap;
}

#endif
