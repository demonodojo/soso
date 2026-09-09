/* Init unificado MVM tras ALIVE (NVM/PHY/INIT_COMPLETE). */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memset(void *dst, int c, unsigned long n);

int iwl_fw_cmd_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    unsigned i;

    for (i = 0; i < iwl->cmd_ver_count; i++) {
        if (iwl->cmd_ver[i].group == group && iwl->cmd_ver[i].cmd == cmd) {
            return iwl->cmd_ver[i].version;
        }
    }
    return 0;
}

int iwl_mvm_run_init(struct iwl_ax211_priv *iwl)
{
    struct iwl_init_extended_cfg_cmd init_cfg;
    struct iwl_nvm_access_complete_cmd nvm_done;
    int t;

    if (!iwl->alive) {
        return -1;
    }

    /* Como `iwl_init_notification_wait` en Linux: armar antes de mandar comandos.
     * `send_cmd_wait` drena RX y puede marcar init_complete durante NVM_ACCESS. */
    iwl->init_complete = 0;

    memset(&init_cfg, 0, sizeof(init_cfg));
    init_cfg.init_flags = (uint32_t)(1u << IWL_INIT_NVM);
    if (iwl_trans_send_cmd_wait(iwl, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, &init_cfg,
                                (uint16_t)sizeof(init_cfg), 2000) != 0) {
        lx_printk("iwl_mvm: INIT_EXTENDED_CFG falló\n");
        return -1;
    }

    /* Linux `iwl_run_unified_mvm_ucode`: ucode unificado no manda NVM_ACCESS_CMD
     * (0x88 es LEGACY, no grp=12); solo NVM_ACCESS_COMPLETE. MAC vía CSR/NVM_GET_INFO
     * tras INIT_COMPLETE. */

    memset(&nvm_done, 0, sizeof(nvm_done));
    if (iwl_trans_send_cmd_wait(iwl, REGULATORY_AND_NVM_GROUP, NVM_ACCESS_COMPLETE,
                                &nvm_done, (uint16_t)sizeof(nvm_done), 2000) != 0) {
        lx_printk("iwl_mvm: NVM_ACCESS_COMPLETE falló\n");
        return -1;
    }

    /* Linux `iwl_send_phy_cfg_cmd` (fw.c:539): ucode unificado sin SISO no manda
     * PHY_CFG — AX200/AX211 son familia 22000. Tras NVM_ACCESS_COMPLETE el FW
     * puede haber enviado INIT_COMPLETE ya (RX drenado en send_cmd_wait). */

    if (!iwl->init_complete) {
        for (t = 0; t < 600; t++) {
            iwl_trans_poll(iwl);
            if (iwl->init_complete) {
                break;
            }
            lx_mdelay(10);
        }
    }
    if (!iwl->init_complete) {
        lx_printk("iwl_mvm: timeout INIT_COMPLETE_NOTIF\n");
        return -1;
    }

    iwl->radio_ready = 1;

    /* Tras INIT_COMPLETE Linux llama `iwl_get_nvm()` (NVM_GET_INFO + CSR MAC). */
    if (iwl_mvm_nvm_get_info_mac(iwl) != 0) {
        lx_printk("iwl_mvm: MAC NVM no disponible (se mantiene BDF)\n");
    }

    /* TX_ANT / MCC / SCAN_CFG van en `iwl_mvm_up_minimal` (post shared mem + SF). */
    lx_printk("iwl_mvm: init NVM listo (phy_sku=0x%08x n_scan=%u tx=0x%x rx=0x%x)\n",
              (unsigned)iwl->phy_sku, (unsigned)iwl->n_scan_channels,
              (unsigned)iwl_mvm_valid_tx_ant(iwl),
              (unsigned)iwl_mvm_valid_rx_ant(iwl));
    return 0;
}
