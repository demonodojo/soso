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
    if (!iwl_ax211_probed())
        return -1;
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

int lx_iwlwifi_connected(void)
{
    return iwl_ax211_connected();
}

int lx_iwlwifi_rx(unsigned char *buf, int buflen)
{
    return iwl_ax211_rx(buf, buflen);
}

int lx_iwlwifi_tx(const unsigned char *buf, int len)
{
    return iwl_ax211_tx(buf, len);
}

int lx_iwlwifi_mac(unsigned char mac[6])
{
    return iwl_ax211_mac(mac);
}

void lx_iwlwifi_poll(void)
{
    iwl_ax211_poll();
}
