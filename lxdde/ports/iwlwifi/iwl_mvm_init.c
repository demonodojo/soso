/* Init unificado MVM tras ALIVE (NVM/PHY/INIT_COMPLETE). */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"
#include "iwl_poll.h"

extern void *memset(void *dst, int c, unsigned long n);

/* Linux `iwl_pnvm_load` (fw/pnvm.c): si el ALIVE v5 trajo SKU, pisa el doorbell
 * UMAC BIT20 y espera PNVM_INIT_COMPLETE_NTFY (0xFE) *antes* de INIT_EXTENDED_CFG. */
static int iwl_pnvm_load(struct iwl_ax211_priv *iwl)
{
    int t;

    if (iwl->family == IWL_DEVICE_FAMILY_8000)
        return 0;
    if (!iwl->sku_id[0] && !iwl->sku_id[1] && !iwl->sku_id[2]) {
        return 0;
    }

    iwl->pnvm_complete = 0;
    if (iwl_trans_pnvm_publish(iwl) != 0)
        return -1;
    iwl_trans_pnvm_doorbell(iwl);
    for (t = 0; t < IWL_PNVM_TIMEOUT_MS; t++) {
        iwl_trans_poll(iwl);
        if (iwl->pnvm_complete) {
            lx_printk("iwl_mvm: PNVM listo (sku=0x%x 0x%x 0x%x)\n",
                      (unsigned)iwl->sku_id[0], (unsigned)iwl->sku_id[1],
                      (unsigned)iwl->sku_id[2]);
            return 0;
        }
        iwl_mdelay_locked(1);
    }
    lx_printk("iwl_mvm: timeout PNVM_INIT_COMPLETE_NTFY\n");
    return -1;
}

/* Linux `iwl_mvm_get_phy_config` (mvm/mvm.h): el `phy_config` del firmware
 * (aquí `phy_sku`, TLV PHY_SKU) con los campos de cadenas reescritos según las
 * antenas válidas. `trans_cfg->extra_phy_cfg_flags` de cfg/8000.c es 0. */
static uint32_t iwl_mvm_phy_cfg_8000(struct iwl_ax211_priv *iwl)
{
    uint32_t phy_config = ~(FW_PHY_CFG_TX_CHAIN | FW_PHY_CFG_RX_CHAIN);
    uint32_t valid_tx = iwl_mvm_valid_tx_ant(iwl);
    uint32_t valid_rx = iwl_mvm_valid_rx_ant(iwl);

    phy_config |= (valid_tx << FW_PHY_CFG_TX_CHAIN_POS) |
                  (valid_rx << FW_PHY_CFG_RX_CHAIN_POS);
    return iwl->phy_sku & phy_config;
}

static int iwl_mvm_phy_cfg_8000_send(struct iwl_ax211_priv *iwl, unsigned image)
{
    struct iwl_phy_cfg_cmd_v3 phy;
    uint16_t len;
    int ver = iwl_fw_cmd_ver(iwl, LEGACY_GROUP, PHY_CONFIGURATION_CMD);
    memset(&phy, 0, sizeof(phy));
    phy.phy_cfg = iwl_cpu_to_le32(iwl_mvm_phy_cfg_8000(iwl));
    phy.calib_control.flow_trigger = iwl_cpu_to_le32(iwl->default_calib[image].flow_trigger);
    phy.calib_control.event_trigger = iwl_cpu_to_le32(iwl->default_calib[image].event_trigger);
    len = ver == 3 ? sizeof(struct iwl_phy_cfg_cmd_v3) : sizeof(struct iwl_phy_cfg_cmd_v1);
    return iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, PHY_CONFIGURATION_CMD, &phy, len, 2000);
}

/* INIT ALIVE: collect NVM and PHY_DB before stopping the INIT image. */
int iwl_mvm_calibrate_8000(struct iwl_ax211_priv *iwl)
{
    struct iwl_tx_ant_cfg_cmd ant;
    int t;
    if (!iwl->alive)
        return -1;
    iwl_phy_db_clear(iwl);
    iwl->init_complete = 0;
    iwl->radio_ready = 0;
    iwl->phy_db_collecting = 1;
    if (iwl_mvm_nvm_read_8000(iwl) != 0)
        goto fail;
    memset(&ant, 0, sizeof(ant));
    ant.valid = iwl_mvm_valid_tx_ant(iwl);
    if (iwl_trans_send_cmd_wait(iwl, LEGACY_GROUP, TX_ANT_CONFIGURATION_CMD,
                                &ant, sizeof(ant), 2000) != 0 ||
        iwl_mvm_phy_cfg_8000_send(iwl, IWL_UCODE_INIT) != 0)
        goto fail;
    for (t = 0; t < 600 && !iwl->init_complete && !iwl->phy_db_error; t++) {
        iwl_trans_poll(iwl);
        iwl_mdelay_locked(10);
    }
    iwl->phy_db_collecting = 0;
    if (!iwl->init_complete || !iwl_phy_db_valid(iwl))
        goto fail;
    lx_printk("iwl_mvm: INIT 8000 calibración completa (%u secciones)\n", iwl->phy_db_count);
    return 0;
fail:
    iwl->phy_db_collecting = 0;
    iwl->nvm_ready = 0;
    lx_printk("iwl_mvm: INIT 8000 calibración falló\n");
    return -1;
}

/* RT: Linux envía la PHY DB recogida en INIT antes de PHY_CFG de RT. */
int iwl_mvm_phy_db_apply_8000(struct iwl_ax211_priv *iwl)
{
    if (iwl_phy_db_send(iwl) != 0 ||
        iwl_mvm_phy_cfg_8000_send(iwl, IWL_UCODE_REGULAR) != 0)
        return -1;
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

    if (iwl->family == IWL_DEVICE_FAMILY_8000)
        return iwl->nvm_ready && iwl->init_complete ? (iwl->radio_ready = 1, 0) : -1;

    if (iwl_pnvm_load(iwl) != 0) {
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
            iwl_mdelay_locked(10);
        }
    }
    if (!iwl->init_complete) {
        lx_printk("iwl_mvm: timeout INIT_COMPLETE_NOTIF\n");
        return -1;
    }

    iwl->radio_ready = 1;

    /* Tras INIT_COMPLETE Linux llama `iwl_get_nvm()` (NVM_GET_INFO + CSR MAC). */
    if (iwl_mvm_nvm_get_info_mac(iwl) != 0) {
        iwl->radio_ready = 0;
        iwl->nvm_ready = 0;
        lx_printk("iwl_mvm: NVM/MAC falló — no listo\n");
        return -1;
    }
    iwl->nvm_ready = 1;

    /* TX_ANT / MCC / SCAN_CFG van en `iwl_mvm_up_minimal` (post shared mem + SF). */
    lx_printk("iwl_mvm: init NVM listo (phy_sku=0x%08x n_scan=%u tx=0x%x rx=0x%x)\n",
              (unsigned)iwl->phy_sku, (unsigned)iwl->n_scan_channels,
              (unsigned)iwl_mvm_valid_tx_ant(iwl),
              (unsigned)iwl_mvm_valid_rx_ant(iwl));
    return 0;
}
