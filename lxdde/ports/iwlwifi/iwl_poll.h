#ifndef IWL_POLL_H
#define IWL_POLL_H

#include <stdint.h>

/* Linux `iwl-io.c`: el timeout de `iwl_poll_bit` va en microsegundos y el
 * paso es 10 µs. 25000 son 25 ms, no 25 s. */
#define IWL_POLL_INTERVAL_US 10

static inline int iwl_poll_bit_us(uint32_t (*read32)(void *ctx, uint32_t off),
                                  void *ctx, uint32_t off, uint32_t mask,
                                  uint32_t want, int timeout_us)
{
    int t = 0;

    do {
        if ((read32(ctx, off) & mask) == want)
            return t;
        lx_udelay(IWL_POLL_INTERVAL_US);
        t += IWL_POLL_INTERVAL_US;
    } while (t < timeout_us);
    return -1;
}

#endif
