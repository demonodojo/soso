#ifndef _LINUX_CRC8_H
#define _LINUX_CRC8_H

#include <linux/types.h>

#define CRC8_TABLE_SIZE 256

void crc8_populate_msb(u8 table[CRC8_TABLE_SIZE], u8 polynomial);
void crc8_populate_lsb(u8 table[CRC8_TABLE_SIZE], u8 polynomial);
u8 crc8(const u8 table[CRC8_TABLE_SIZE], const u8 *pdata, size_t nbytes, u8 crc);

#endif
