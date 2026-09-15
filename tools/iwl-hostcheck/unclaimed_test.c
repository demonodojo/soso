/* Host: 8086:24fd se nombra como familia 8000 y no se trata como AX211. */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdarg.h>

#include "lx_emul.h"
#include "iwl_ax211.h"

static char g_log[512];
static int g_inject_8265;

void lx_printk(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(g_log, sizeof(g_log), fmt, ap);
    va_end(ap);
}

void lx_pci_for_each(lx_pci_each_fn fn, void *ctx)
{
    if (!fn)
        return;
    if (g_inject_8265)
        fn(0x8086, 0x24f3, 0x02, 0x80, ctx);
}

int iwl_ax211_id_supported(uint16_t device_id)
{
    return device_id == 0x7f70u || device_id == 0x51f0u ||
           device_id == 0x54f0u || device_id == 0x2723u ||
           device_id == 0x24fdu;
}

int main(void)
{
    if (iwl_ax211_family_from_id(0x24fd) != IWL_DEVICE_FAMILY_8000 ||
        iwl_ax211_family_from_id(0x2723) != IWL_DEVICE_FAMILY_22000 ||
        iwl_ax211_family_from_id(0x7f70) != IWL_DEVICE_FAMILY_AX210) {
        printf("FALLO: family_from_id 8265/AX200/AX211\n");
        return 1;
    }
    if (strcmp(iwl_ax211_family_name(IWL_DEVICE_FAMILY_8000), "8000") != 0) {
        printf("FALLO: family_name 8000\n");
        return 1;
    }
    if (strcmp(iwl_ax211_unclaimed_family(0x24fd), "familia 8000") != 0) {
        printf("FALLO: 0x24fd debía ser familia 8000\n");
        return 1;
    }
    if (strcmp(iwl_ax211_unclaimed_family(0x24f3), "familia 8000") != 0) {
        printf("FALLO: 0x24f3 debía ser familia 8000\n");
        return 1;
    }
    if (!iwl_ax211_id_supported(0x24fd) || !iwl_ax211_id_supported(0x2723) ||
        !iwl_ax211_id_supported(0x7f70)) {
        printf("FALLO: 24fd/AX200/AX211 deben estar en la tabla\n");
        return 1;
    }

    g_inject_8265 = 0;
    g_log[0] = 0;
    iwl_ax211_log_missing_devices();
    if (strstr(g_log, "sin adaptador AX211/AX200") == 0) {
        printf("FALLO: sin PCI debía decir sin adaptador (%s)\n", g_log);
        return 1;
    }

    g_inject_8265 = 1;
    g_log[0] = 0;
    iwl_ax211_log_missing_devices();
    if (strstr(g_log, "8086:24f3") == 0 || strstr(g_log, "familia 8000") == 0) {
        printf("FALLO: 8260 debía loguearse como 8000 no portada (%s)\n", g_log);
        return 1;
    }
    if (strstr(g_log, "so-a0-gf") != 0 || strstr(g_log, "gen3") != 0) {
        printf("FALLO: no debe sugerir gen3/AX211 (%s)\n", g_log);
        return 1;
    }

    printf("OK: 8086:24fd familia 8000 (probe); 24f3 sigue sin reclamar como gen3\n");
    return 0;
}
