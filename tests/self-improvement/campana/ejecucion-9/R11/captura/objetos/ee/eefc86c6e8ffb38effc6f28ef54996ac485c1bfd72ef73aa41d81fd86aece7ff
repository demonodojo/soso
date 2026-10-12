/* cfg80211 mínimo — scan y connect sin nl80211. */
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);

int lx_cfg80211_scan(void)
{
    struct iwl_ax211_bss tmp[IWL_AX211_MAX_SCAN];
    int count = 0;
    return iwl_ax211_scan(tmp, IWL_AX211_MAX_SCAN, &count);
}

int lx_cfg80211_get_scan_results(struct iwl_ax211_bss *out, int max, int *count)
{
    return iwl_ax211_scan(out, max, count);
}

int lx_cfg80211_connect(const char *ssid, int open_network)
{
    if (open_network)
        return iwl_ax211_connect_open(ssid);
    return -1; /* WPA via lx_cfg80211_set_key desde Rust */
}

int lx_cfg80211_connected(void)
{
    return iwl_ax211_connected();
}

int lx_cfg80211_add_key(const unsigned char *gtk, unsigned int gtk_len, int key_idx)
{
    if (!gtk || gtk_len < 16)
        return -1;
    unsigned char key16[16];
    memcpy(key16, gtk, 16);
    return iwl_ax211_install_key(key16, key_idx);
}
