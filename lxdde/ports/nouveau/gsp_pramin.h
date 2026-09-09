/* Ventana BAR0 → VRAM (PRAMIN), como `nv50_instmem` (Ampere/Ada) o
 * `gh100_instmem` (Hopper/Blackwell) en nouveau. */
#ifndef GSP_PRAMIN_H
#define GSP_PRAMIN_H

#include <stdint.h>

/* Ampere/Ada (nv50_instmem): `0x001700` ← `base >> 16`, datos en
 * `0x700000 + (addr & 0xfffff)` (ventana 1 MiB). */
#define GSP_PRAMIN_WINDOW_REG_NV50  0x001700u
#define GSP_PRAMIN_OFF_MASK_NV50    0xfffffu

/* Hopper/Blackwell (gh100_instmem): `NV_XAL_EP_BAR0_WINDOW` @ 0x0010fd40,
 * valor = addr >> 16, datos en `0x700000 + (addr & 0xffff)` (64 KiB). */
#define GSP_PRAMIN_WINDOW_REG_GB    0x0010fd40u
#define GSP_PRAMIN_OFF_MASK_GB      0xffffu

#define GSP_PRAMIN_WINDOW_SHIFT 16u
#define GSP_PRAMIN_MMIO_BASE    0x00700000u

uint32_t gsp_pramin_rd32(uint64_t addr_vram);
void gsp_pramin_wr32(uint64_t addr_vram, uint32_t val);

/* Pone a cero `bytes` (múltiplo de 4) en FB vía PRAMIN. */
void gsp_pramin_memset32(uint64_t addr_vram, uint32_t val, unsigned bytes);

/* 1 si la ventana responde (readback del registro y lectura de FB sin
 * `0xbadfxxxx`). Se cachea tras la primera prueba. */
int gsp_pramin_alive(void);

/* Fuerza reprogramación de la ventana en la siguiente operación. */
void gsp_pramin_invalidate(void);

#endif
