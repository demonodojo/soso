/* 25000 en iwl_poll_bit son microsegundos (25 ms), no 25 s. */
#include <stdio.h>
#include <stdint.h>

static unsigned long g_us;
static uint32_t g_val;
static int g_reads;
static int g_ready_after; /* lectura N (1-based) en adelante ve el bit */

void lx_udelay(unsigned long us)
{
    g_us += us;
}

static uint32_t read_reg(void *ctx, uint32_t off)
{
    (void)ctx;
    (void)off;
    g_reads++;
    if (g_ready_after && g_reads >= g_ready_after)
        return 1u;
    return g_val;
}

#include "iwl_poll.h"

int main(void)
{
    g_val = 0;
    g_us = 0;
    if (iwl_poll_bit_us(read_reg, NULL, 0, 1u, 1u, 25000) != -1) {
        printf("FALLO: un poll de 25000 µs sin el bit debía agotarse\n");
        return 1;
    }
    if (g_us != 25000ul) {
        printf("FALLO: 25000 µs esperó %lu µs\n", g_us);
        return 1;
    }

    g_val = 1;
    g_us = 0;
    g_reads = 0;
    g_ready_after = 0;
    if (iwl_poll_bit_us(read_reg, NULL, 0, 1u, 1u, 25000) != 0 || g_us != 0) {
        printf("FALLO: con el bit ya listo no hay que esperar\n");
        return 1;
    }

    /* El bit llega en la segunda lectura: éxito con tiempo >0. Un llamador
     * que trate distinto de cero como fallo aborta el MAC con el reloj ya
     * en marcha (ROG 2026-11-06, GP=0x08040005). */
    g_val = 0;
    g_us = 0;
    g_reads = 0;
    g_ready_after = 2;
    if (iwl_poll_bit_us(read_reg, NULL, 0, 1u, 1u, 25000) != 10) {
        printf("FALLO: el bit en la segunda lectura debe devolver 10 µs\n");
        return 1;
    }

    printf("OK: poll MAC 25000 µs = 25 ms\n");
    return 0;
}
