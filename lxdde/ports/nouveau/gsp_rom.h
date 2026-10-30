#ifndef GSP_ROM_H
#define GSP_ROM_H

#include <stdint.h>

/* VBIOS en BAR0 (mismo mapa que gsp_fwsec.c). */
#define GSP_ROM_PROM_BASE  0x00300000u
#define GSP_ROM_PROM_MAX   0x00100000u

void gsp_rom_init(void);
void gsp_rom_set_limit(unsigned len);
unsigned gsp_rom_limit(void);

uint8_t gsp_rom_rd8(unsigned off);
uint16_t gsp_rom_rd16(unsigned off);
uint32_t gsp_rom_rd32(unsigned off);
int gsp_rom_match(unsigned off, const char *sig, unsigned n);
int gsp_rom_copy(unsigned off, void *dst, unsigned len);

#endif
