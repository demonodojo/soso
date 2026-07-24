#ifndef GSP_MMIO_H
#define GSP_MMIO_H

#include <stdint.h>

void gsp_mmio_set_bar(void *bar, unsigned len);
uint32_t gsp_mmio_rd32(unsigned off);
void gsp_mmio_wr32(unsigned off, uint32_t val);
int gsp_mmio_poll_ready(unsigned timeout_ms);
int gsp_mmio_kick_boot(void);

#endif
