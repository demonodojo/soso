/* G3 ola 2: secuencia tu102_acr_init (AHESASC → ASB) en lx-native. */
#include "acr_lx.h"
#include "acr_fw.h"
#include "falcon_lx.h"
#include "lx_emul.h"

int acr_lx_load(void)
{
    return acr_fw_load_all();
}

int acr_lx_boot_ahesasc(void)
{
    const struct acr_fw_blob *ahesasc = acr_fw_get(ACR_FW_AHESASC);

    if (!ahesasc) {
        return -1;
    }
    lx_printk("nouveau-lx: ACR ola2 — AHESASC (SEC2 @0x%x)\n", LX_FLCN_SEC2_BASE);
    if (falcon_lx_hsfw_boot(LX_FLCN_SEC2_BASE, ahesasc, "AHESASC") != 0) {
        lx_printk("nouveau-lx: ACR AHESASC falló — sigue booter\n");
        return -1;
    }
    return 0;
}

int acr_lx_boot_asb(void)
{
    const struct acr_fw_blob *asb = acr_fw_get(ACR_FW_ASB);

    if (!asb) {
        return -1;
    }
    lx_printk("nouveau-lx: ACR ola2 — ASB (GSP @0x%x)\n", LX_FLCN_GSP_BASE);
    if (falcon_lx_hsfw_boot(LX_FLCN_GSP_BASE, asb, "ASB") != 0) {
        lx_printk("nouveau-lx: ACR ASB falló\n");
        return -1;
    }
    lx_printk("nouveau-lx: ACR ola2 completado\n");
    return 0;
}

int acr_lx_boot(void)
{
    if (acr_lx_load() != 0) {
        return -1;
    }
    if (acr_lx_boot_ahesasc() != 0) {
        return -1;
    }
    return acr_lx_boot_asb();
}
