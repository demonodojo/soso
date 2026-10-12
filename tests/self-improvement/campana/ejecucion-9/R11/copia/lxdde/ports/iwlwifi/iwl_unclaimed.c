/* Intel WiFi en el bus que el port no reclama: log, no probe gen3. */
#include "lx_emul.h"
#include "iwl_ax211.h"

int iwl_ax211_family_from_id(uint16_t device_id)
{
    switch (device_id) {
    case IWL_PCI_AX200:
        return IWL_DEVICE_FAMILY_22000;
    case 0x7f70u:
    case 0x51f0u:
    case 0x54f0u:
        return IWL_DEVICE_FAMILY_AX210;
    case IWL_PCI_8265:
    case 0x24f3u:
        return IWL_DEVICE_FAMILY_8000;
    default:
        return IWL_DEVICE_FAMILY_UNKNOWN;
    }
}

const char *iwl_ax211_family_name(int family)
{
    switch (family) {
    case IWL_DEVICE_FAMILY_8000:
        return "8000";
    case IWL_DEVICE_FAMILY_22000:
        return "22000";
    case IWL_DEVICE_FAMILY_AX210:
        return "AX210";
    default:
        return "?";
    }
}

const char *iwl_ax211_unclaimed_family(uint16_t device_id)
{
    int family = iwl_ax211_family_from_id(device_id);
    if (family == IWL_DEVICE_FAMILY_8000)
        return "familia 8000";
    return "familia no portada";
}

void iwl_ax211_log_unclaimed(uint16_t device_id)
{
    lx_printk("iwlwifi: 8086:%04x %s no portada (IDs: 7f70/51f0/54f0/2723/24fd)\n",
              (unsigned)device_id, iwl_ax211_unclaimed_family(device_id));
}

static void iwl_log_unclaimed_each(uint16_t vendor, uint16_t device,
                                   uint8_t pci_class, uint8_t subclass, void *ctx)
{
    int *found = (int *)ctx;
    (void)subclass;
    if (vendor != 0x8086u || pci_class != 0x02)
        return;
    if (iwl_ax211_id_supported(device))
        return;
    iwl_ax211_log_unclaimed(device);
    if (found)
        *found = 1;
}

void iwl_ax211_log_missing_devices(void)
{
    int found = 0;
    lx_pci_for_each(iwl_log_unclaimed_each, &found);
    if (!found)
        lx_printk("iwlwifi: sin adaptador AX211/AX200\n");
}
