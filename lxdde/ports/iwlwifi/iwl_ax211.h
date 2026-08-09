#ifndef IWL_AX211_H
#define IWL_AX211_H

#include <stdint.h>
#include <stddef.h>

#define IWL_AX211_MAX_SCAN 32
#define IWL_AX211_SSID_MAX 32

struct iwl_ax211_bss {
    char ssid[IWL_AX211_SSID_MAX + 1];
    uint8_t bssid[6];
    int8_t rssi;
    uint8_t channel;
    uint8_t open; /* 1 = sin clave */
};

int iwl_ax211_init(void);
void iwl_ax211_poll(void);
int iwl_ax211_alive(void);
const char *iwl_ax211_phase(void);

int iwl_ax211_scan(struct iwl_ax211_bss *out, int max, int *count);
int iwl_ax211_connect_open(const char *ssid);
int iwl_ax211_connect_wpa2(const char *ssid, const uint8_t psk[32]);
int iwl_ax211_connected(void);

int iwl_ax211_rx(uint8_t *buf, int buflen);
int iwl_ax211_tx(const uint8_t *buf, int len);
int iwl_ax211_mac(uint8_t mac[6]);

#endif
