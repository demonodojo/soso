#ifndef GSP_MMIO_H
#define GSP_MMIO_H

#include <stdint.h>

void gsp_mmio_set_bar(void *bar, unsigned len);
uint32_t gsp_mmio_rd32(unsigned off);
void gsp_mmio_wr32(unsigned off, uint32_t val);
/* 0 si la GPU no contesta en el bus (MMIO todo a unos o boot0 a cero). */
int gsp_mmio_alive(void);

struct lx_pci_dev;
void gsp_mmio_set_pci(struct lx_pci_dev *pdev);

/* Cuando el MMIO se lee todo a unos hay dos historias muy distintas: que la GPU
 * se haya ido del bus, o que siga ahí y solo haya perdido el decode de memoria /
 * las BAR (lo que deja un reset de función). El espacio de configuración las
 * separa. Vuelca id, command y BAR0, reactiva memory+bus-master si hacía falta,
 * y devuelve 0 si el dispositivo responde en configuración. */
int gsp_mmio_pci_recover(void);
int gsp_mmio_poll_ready(unsigned timeout_ms);
int gsp_mmio_kick_boot(void);

/* 1 si el valor leído es un error del anillo PRI (`0xbadfxxxx`) y no un dato: el
 * bloque de destino no contesta o el acceso no está permitido. */
int gsp_mmio_pri_error(uint32_t v);

/* Espera a que el firmware de la GPU (GFW) acabe su devinit: 0x118128 bit 0 y
 * 0x118234 PROGRESS = 0xff, como `tu102_devinit_wait`. `when` es una etiqueta para
 * el log. Devuelve 0 si completó; en caso contrario deja los valores dichos y sigue
 * siendo el llamante quien decide. */
int gsp_mmio_gfw_wait(unsigned timeout_ms, const char *when);

#endif
