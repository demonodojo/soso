/* Ventana BAR0 → VRAM (PRAMIN), como `gh100_instmem_set_bar0_window_addr` +
 * `nv50_instmem` en nouveau: sin BAR1 la CPU accede a FB por `0x10fd40` y
 * `0x700000 + (addr & 0xffff)`. */
#ifndef GSP_PRAMIN_H
#define GSP_PRAMIN_H

#include <stdint.h>

/* gb202/gb100: `NV_XAL_EP_BAR0_WINDOW` @ 0x0010fd40, BASE en bits 22:0,
 * valor = addr >> 16 (`NV_XAL_EP_BAR0_WINDOW_BASE_SHIFT`). */
#define GSP_PRAMIN_WINDOW_REG   0x0010fd40u
#define GSP_PRAMIN_WINDOW_SHIFT 16u
#define GSP_PRAMIN_MMIO_BASE    0x00700000u

uint32_t gsp_pramin_rd32(uint64_t addr_vram);
void gsp_pramin_wr32(uint64_t addr_vram, uint32_t val);

/* 1 si la ventana responde (readback del registro y lectura de FB sin
 * `0xbadfxxxx`). Se cachea tras la primera prueba. */
int gsp_pramin_alive(void);

/* Fuerza reprogramación de la ventana en la siguiente operación. */
void gsp_pramin_invalidate(void);

#endif
