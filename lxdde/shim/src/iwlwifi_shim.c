/* Registro de initcalls y estado global iwlwifi. */
#include "lx_emul.h"

extern void *memset(void *dst, int c, unsigned long n);
extern char *strncpy(char *dst, const char *src, unsigned long n);

/* Estado ALIVE expuesto al puente Rust. */
static volatile int g_fw_alive;
static char g_phase[64] = "init";

void lx_iwlwifi_set_alive(int alive)
{
    g_fw_alive = alive;
}

void lx_iwlwifi_set_phase(const char *phase)
{
    if (!phase)
        return;
    strncpy(g_phase, phase, sizeof(g_phase) - 1);
    g_phase[sizeof(g_phase) - 1] = '\0';
}

/* CRC32 LE simplificado (Ethernet/IEEE). */
uint32_t lx_crc32_le(uint32_t crc, const void *p, unsigned int len)
{
    const unsigned char *data = (const unsigned char *)p;
    crc = ~crc;
    for (unsigned int i = 0; i < len; i++) {
        crc ^= data[i];
        for (int b = 0; b < 8; b++)
            crc = (crc >> 1) ^ (0xedb88320u & (~(crc & 1u) + 1u));
    }
    return ~crc;
}

/* Mutex simple para código portado. */
struct lx_mutex {
    int locked;
};

void lx_mutex_init(struct lx_mutex *m)
{
    if (m)
        m->locked = 0;
}

void lx_mutex_lock(struct lx_mutex *m)
{
    while (m && __sync_lock_test_and_set(&m->locked, 1))
        lx_yield();
}

void lx_mutex_unlock(struct lx_mutex *m)
{
    if (m)
        __sync_lock_release(&m->locked);
}
