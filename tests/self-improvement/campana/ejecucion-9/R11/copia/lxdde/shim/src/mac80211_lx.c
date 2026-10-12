/* mac80211 mínimo — puente hacia cfg80211_lx e iwl_ax211. */
#include "iwl_ax211.h"
#include "lx_emul.h"

int lx_mac80211_register_hw(void)
{
    lx_printk("mac80211: ieee80211_register_hw ok\n");
    return 0;
}

void lx_mac80211_poll(void)
{
    iwl_ax211_poll();
}

int lx_mac80211_scan(void)
{
    extern int lx_cfg80211_scan(void);
    return lx_cfg80211_scan();
}

int lx_mac80211_connect_open(const char *ssid)
{
    extern int lx_cfg80211_connect(const char *ssid, int open_network);
    return lx_cfg80211_connect(ssid, 1);
}

int lx_mac80211_connect_wpa2(const char *ssid, const unsigned char psk[32])
{
    return iwl_ax211_connect_wpa2(ssid, psk);
}
