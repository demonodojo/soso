/* net_device mínimo para WiFi STA. */
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern char *strncpy(char *dst, const char *src, unsigned long n);

struct wifi_sk_buff;
extern unsigned char *wifi_skb_data(struct wifi_sk_buff *skb);
extern unsigned int wifi_skb_len(const struct wifi_sk_buff *skb);
extern void wifi_free_skb(struct wifi_sk_buff *skb);

struct net_device {
    char name[16];
    unsigned char dev_addr[6];
    unsigned flags;
    void *priv;
    int carrier;
};

static struct net_device *g_wlan;

struct net_device *lx_wlan_dev(void)
{
    return g_wlan;
}

struct net_device *lx_alloc_netdev(int priv_size, const char *name)
{
    struct net_device *dev = lx_kzalloc(sizeof(*dev) + (size_t)priv_size, GFP_KERNEL);
    if (!dev)
        return 0;
    if (name)
        strncpy(dev->name, name, sizeof(dev->name) - 1);
    dev->priv = (char *)dev + sizeof(*dev);
    g_wlan = dev;
    return dev;
}

int lx_register_netdevice(struct net_device *dev)
{
    if (!dev)
        return -1;
    dev->carrier = 1;
    g_wlan = dev;
    return 0;
}

void lx_unregister_netdevice(struct net_device *dev)
{
    if (g_wlan == dev)
        g_wlan = 0;
}

void *lx_wifi_netdev_priv(struct net_device *dev)
{
    return dev ? dev->priv : 0;
}

void lx_wifi_netif_rx(struct wifi_sk_buff *skb)
{
    if (!skb)
        return;
    extern void iwl_ax211_deliver_rx(const unsigned char *data, int len);
    iwl_ax211_deliver_rx(wifi_skb_data(skb), (int)wifi_skb_len(skb));
    wifi_free_skb(skb);
}

int lx_wifi_dev_queue_xmit(struct wifi_sk_buff *skb)
{
    extern int iwl_ax211_tx(const unsigned char *buf, int len);
    int rc = iwl_ax211_tx(wifi_skb_data(skb), (int)wifi_skb_len(skb));
    wifi_free_skb(skb);
    return rc >= 0 ? 0 : -1;
}
