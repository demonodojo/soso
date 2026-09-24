/* Puente lx_emul → driver Intel AX211/AX200. */
#include "lx_emul.h"
#include "iwl_ax211.h"

extern int lx_mac80211_register_hw(void);

static int driver_registered;

int lx_iwlwifi_init_module(void)
{
    if (driver_registered)
        return 0;
    int err = iwl_ax211_register();
    if (err == 0)
        driver_registered = 1;
    return err;
}

int lx_iwlwifi_start_module(void)
{
    if (!iwl_ax211_probed()) {
        iwl_ax211_log_missing_devices();
        return -1;
    }
    int err = iwl_ax211_start_firmware();
    if (err == 0)
        err = lx_mac80211_register_hw();
    return err;
}

void lx_iwlwifi_exit_module(void)
{
    lx_printk("iwlwifi: exit\n");
}

int lx_iwlwifi_fw_alive(void)
{
    return iwl_ax211_alive();
}

int lx_iwlwifi_probed(void)
{
    return iwl_ax211_probed();
}

const char *lx_iwlwifi_fw_phase(void)
{
    return iwl_ax211_phase();
}

int lx_iwlwifi_scan(struct iwl_ax211_bss *out, int max, int *count)
{
    return iwl_ax211_scan(out, max, count);
}

int lx_iwlwifi_get_scan_results(struct iwl_ax211_bss *out, int max, int *count)
{
    return iwl_ax211_get_scan_results(out, max, count);
}

int lx_iwlwifi_connect_open(const char *ssid)
{
    return iwl_ax211_connect_open(ssid);
}

int lx_iwlwifi_connect_wpa2(const char *ssid, const unsigned char psk[32])
{
    return iwl_ax211_connect_wpa2(ssid, psk);
}

int lx_iwlwifi_install_key(const unsigned char key[16], int key_idx)
{
    return iwl_ax211_install_key(key, key_idx);
}

int lx_iwlwifi_install_gtk(const unsigned char key[16], int key_idx,
                           const unsigned char rsc[8])
{
    return iwl_ax211_install_gtk(key, key_idx, rsc);
}

int lx_iwlwifi_connected(void)
{
    return iwl_ax211_connected();
}

int lx_iwlwifi_authorized(void)
{
    return iwl_ax211_authorized();
}

void lx_iwlwifi_set_authorized(int authorized)
{
    iwl_ax211_set_authorized(authorized);
}

int lx_iwlwifi_rsn_ie(unsigned char *out, int max)
{
    return iwl_ax211_rsn_ie(out, max);
}

int lx_iwlwifi_rx(unsigned char *buf, int buflen)
{
    return iwl_ax211_rx(buf, buflen);
}

int lx_iwlwifi_rx_eapol(unsigned char *buf, int buflen)
{
    return iwl_ax211_rx_eapol(buf, buflen);
}

int lx_iwlwifi_tx(const unsigned char *buf, int len)
{
    return iwl_ax211_tx(buf, len);
}

int lx_iwlwifi_can_send(void)
{
    return iwl_ax211_can_tx();
}

int lx_iwlwifi_mac(unsigned char mac[6])
{
    return iwl_ax211_mac(mac);
}

int lx_iwlwifi_bssid(unsigned char bssid[6])
{
    return iwl_ax211_bssid(bssid);
}

void lx_iwlwifi_poll(void)
{
    iwl_ax211_poll();
}

struct lx_iwl_dma_range {
    uint64_t pa;
    uint64_t len;
};

static void push_dma(struct lx_iwl_dma_range *out, int max, int *n,
                     uint64_t pa, uint64_t len)
{
    if (!pa || !len || !out || *n >= max)
        return;
    out[*n].pa = pa;
    out[*n].len = len;
    (*n)++;
}

int lx_iwlwifi_dma_ranges(struct lx_iwl_dma_range *out, int max)
{
    struct iwl_ax211_priv *iwl = &g_iwl;
    int n = 0;
    unsigned bd_sz = IWL_GEN2_RX_N * 16u;
    unsigned used_sz = IWL_GEN2_RX_N * 8u;
    unsigned rx_pages = (unsigned)IWL_GEN2_RX_N * (unsigned)IWL_GEN2_RX_SZ;

    push_dma(out, max, &n, iwl->rx_bd_dma, bd_sz);
    push_dma(out, max, &n, iwl->used_bd_dma, used_sz);
    push_dma(out, max, &n, iwl->rb_stts_dma, 16);
    push_dma(out, max, &n, iwl->rx_page_dma, rx_pages);
    push_dma(out, max, &n, iwl->mtr_dma,
             (uint64_t)IWL_CMD_QUEUE_SIZE * (uint64_t)IWL_TFH_TFD_SIZE);
    push_dma(out, max, &n, iwl->mcr_dma,
             (uint64_t)IWL_CMD_QUEUE_SIZE * (uint64_t)IWL_CMD_SLOT_SIZE);
    push_dma(out, max, &n, iwl->data_body_dma,
             (uint64_t)IWL_MGMT_QUEUE_SIZE * (uint64_t)IWL_MGMT_TX_SLOT_SIZE);
    push_dma(out, max, &n, iwl->mgmt_body_dma,
             (uint64_t)IWL_MGMT_QUEUE_SIZE * (uint64_t)IWL_MGMT_TX_SLOT_SIZE);
    return n;
}
