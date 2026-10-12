/* sk_buff mínimo para mac80211/iwlwifi (distinto de lx_sk_buff de e1000e). */
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);

struct wifi_sk_buff {
    unsigned char *head;
    unsigned char *data;
    unsigned int len;
    unsigned int tailroom;
    unsigned char cb[48];
};

struct wifi_sk_buff *wifi_alloc_skb(unsigned int size)
{
    struct wifi_sk_buff *skb = lx_kzalloc(sizeof(*skb) + size + 256, GFP_KERNEL);
    if (!skb)
        return 0;
    skb->head = (unsigned char *)(skb + 1);
    skb->data = skb->head + 128;
    skb->tailroom = size;
    skb->len = 0;
    return skb;
}

void wifi_free_skb(struct wifi_sk_buff *skb)
{
    lx_kfree(skb);
}

unsigned char *wifi_skb_put(struct wifi_sk_buff *skb, unsigned int len)
{
    unsigned char *p = skb->data + skb->len;
    skb->len += len;
    return p;
}

void wifi_skb_reserve(struct wifi_sk_buff *skb, int len)
{
    skb->data += len;
}

unsigned int wifi_skb_len(const struct wifi_sk_buff *skb)
{
    return skb ? skb->len : 0;
}

unsigned char *wifi_skb_data(struct wifi_sk_buff *skb)
{
    return skb ? skb->data : 0;
}
