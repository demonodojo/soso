/* e1000e portado vía lx_emul — estructura pci_driver + net_device de Linux. */
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);

#define VENDOR_INTEL 0x8086u
#define DEV_82574L   0x10d3u
#define DEV_82540EM  0x100eu

#define REG_CTRL   0x0000u
#define REG_ICR    0x00c0u
#define REG_IMS    0x00d0u
#define REG_IMC    0x00d8u
#define REG_RCTL   0x0100u
#define REG_TCTL   0x0400u
#define REG_TIPG   0x0410u
#define REG_RDBAL  0x2800u
#define REG_RDBAH  0x2804u
#define REG_RDLEN  0x2808u
#define REG_RDH    0x2810u
#define REG_RDT    0x2818u
#define REG_TDBAL  0x3800u
#define REG_TDBAH  0x3804u
#define REG_TDLEN  0x3808u
#define REG_TDH    0x3810u
#define REG_TDT    0x3818u
#define REG_RAL    0x5400u
#define REG_RAH    0x5404u
#define REG_MTA    0x5200u
#define REG_EECD   0x0010u
#define REG_EERD   0x0014u

#define CTRL_RST   (1u << 26)
#define CTRL_SLU   (1u << 6)
#define CTRL_ASDE  (1u << 5)
#define RCTL_EN    (1u << 1)
#define RCTL_SBP   (1u << 2)
#define RCTL_UPE   (1u << 3)
#define RCTL_MPE   (1u << 4)
#define RCTL_LPE   (1u << 5)
#define RCTL_BAM   (1u << 15)
#define RCTL_SECRC (1u << 26)
#define TCTL_EN    (1u << 1)
#define TCTL_PSP   (1u << 3)
#define IMS_RXT0   (1u << 7)
#define RX_DD      (1u << 0)
#define RX_EOP     (1u << 1)
#define TX_DD      (1u << 0)

#define RX_DESC 32
#define TX_DESC 32
#define BUF_LEN 2048

struct rx_desc {
    uint64_t addr;
    uint16_t length;
    uint16_t checksum;
    uint8_t status;
    uint8_t errors;
    uint16_t special;
} __attribute__((packed));

struct tx_desc {
    uint64_t addr;
    uint16_t length;
    uint8_t cso;
    uint8_t cmd;
    uint8_t status;
    uint8_t css;
    uint16_t special;
} __attribute__((packed));

struct e1000_adapter {
    struct lx_net_device *netdev;
    struct lx_pci_dev *pdev;
    volatile uint32_t *mmio;
    uint64_t mmio_base;
    uint8_t mac[6];
    uint64_t rx_ring_dma;
    uint64_t tx_ring_dma;
    uint64_t rx_buf_dma;
    uint64_t tx_buf_dma;
    struct rx_desc *rx;
    struct tx_desc *tx;
    uint8_t *rx_bufs;
    uint8_t *tx_bufs;
    uint16_t rx_tail;
    uint16_t tx_tail;
    int irq;
};

static struct e1000_adapter *g_adapter;

static uint32_t rr(volatile uint32_t *base, uint32_t off)
{
    return base[off / 4];
}

static void rw(volatile uint32_t *base, uint32_t off, uint32_t v)
{
    base[off / 4] = v;
}

static int mac_ok(const uint8_t *m)
{
    if (m[0] == 0 && m[1] == 0 && m[2] == 0 && m[3] == 0 && m[4] == 0 && m[5] == 0)
        return 0;
    if (m[0] == 0xff && m[1] == 0xff && m[2] == 0xff)
        return 0;
    return 1;
}

static void read_mac_hw(volatile uint32_t *mmio, uint16_t devid, uint8_t *mac)
{
    uint32_t ral = rr(mmio, REG_RAL);
    uint32_t rah = rr(mmio, REG_RAH);
    mac[0] = (uint8_t)(ral);
    mac[1] = (uint8_t)(ral >> 8);
    mac[2] = (uint8_t)(ral >> 16);
    mac[3] = (uint8_t)(ral >> 24);
    mac[4] = (uint8_t)(rah);
    mac[5] = (uint8_t)(rah >> 8);
    if (devid == DEV_82574L) {
        uint32_t eecd = rr(mmio, REG_EECD);
        if (eecd & (1u << 3)) {
            rw(mmio, REG_EERD, (1u << 31) | (1u << 8));
            for (int i = 0; i < 10000; i++) {
                uint32_t v = rr(mmio, REG_EERD);
                if (v & (1u << 4)) {
                    uint16_t w = (uint16_t)((v >> 16) & 0xffff);
                    mac[0] = (uint8_t)(w);
                    mac[1] = (uint8_t)(w >> 8);
                    break;
                }
            }
        }
    }
    (void)devid;
}

static irqreturn_t e1000_irq(int irq, void *dev_id)
{
    (void)irq;
    struct e1000_adapter *ad = dev_id;
    if (ad && ad->mmio)
        (void)rr(ad->mmio, REG_ICR);
    lx_irq_wake((unsigned)ad->irq);
    return IRQ_HANDLED;
}

static int e1000_open(struct lx_net_device *dev)
{
    struct e1000_adapter *ad = lx_netdev_priv(dev);
    volatile uint32_t *mmio = ad->mmio;

    rw(mmio, REG_RCTL, RCTL_EN | RCTL_SBP | RCTL_UPE | RCTL_MPE | RCTL_LPE | RCTL_BAM | RCTL_SECRC);
    rw(mmio, REG_TCTL, TCTL_EN | TCTL_PSP | (0x10u << 4) | (0x40u << 12));
    rw(mmio, REG_TIPG, 0x0060200Au);
    rw(mmio, REG_IMS, IMS_RXT0 | (1u << 0) | (1u << 1) | (1u << 2) | (1u << 6) | (1u << 7));
    lx_netif_start_queue(dev);
    lx_netif_carrier_on(dev);
    return 0;
}

static int tx_slot_free(struct tx_desc *d)
{
    return (d->status & TX_DD) != 0 || d->cmd == 0;
}

static int e1000_xmit(struct lx_sk_buff *skb, struct lx_net_device *dev)
{
    struct e1000_adapter *ad = lx_netdev_priv(dev);
    uint16_t i = ad->tx_tail;
    struct tx_desc *d = &ad->tx[i];
    uint8_t *buf = ad->tx_bufs + i * BUF_LEN;
    unsigned len = lx_skb_len(skb);
    unsigned t;

    if (len > BUF_LEN)
        len = BUF_LEN;
    for (t = 0; t < 500000; t++) {
        if (tx_slot_free(d))
            break;
    }
    if (!tx_slot_free(d))
        return -1;

    memcpy(buf, lx_skb_data(skb), len);
    d->addr = ad->tx_buf_dma + (uint64_t)i * BUF_LEN;
    d->length = (uint16_t)len;
    d->cmd = (1u << 0) | (1u << 1) | (1u << 3);
    d->status = 0;

    ad->tx_tail = (uint16_t)((i + 1) % TX_DESC);
    rw(ad->mmio, REG_TDT, ad->tx_tail);

    for (t = 0; t < 500000; t++) {
        if (d->status & TX_DD)
            break;
    }
    lx_kfree_skb(skb);
    return (d->status & TX_DD) ? 0 : -1;
}

static int e1000_stop(struct lx_net_device *dev)
{
    struct e1000_adapter *ad = lx_netdev_priv(dev);
    rw(ad->mmio, REG_IMC, 0xffffffffu);
    rw(ad->mmio, REG_RCTL, 0);
    rw(ad->mmio, REG_TCTL, 0);
    lx_netif_stop_queue(dev);
    return 0;
}

/* Declaración externa del bridge Rust. */
unsigned lx_skb_len(struct lx_sk_buff *skb);
const uint8_t *lx_skb_data(struct lx_sk_buff *skb);

void lx_e1000_poll(struct e1000_adapter *ad)
{
    if (!ad || !ad->mmio)
        return;
    for (int burst = 0; burst < 16; burst++) {
        uint16_t i = ad->rx_tail;
        struct rx_desc *d = &ad->rx[i];
        if (!(d->status & RX_DD))
            break;
        if (!(d->status & RX_EOP) || d->errors != 0) {
            d->status = 0;
            ad->rx_tail = (uint16_t)((i + 1) % RX_DESC);
            rw(ad->mmio, REG_RDT, (uint32_t)((ad->rx_tail + RX_DESC - 1) % RX_DESC));
            continue;
        }
        unsigned len = d->length;
        if (len > BUF_LEN)
            len = BUF_LEN;
        struct lx_sk_buff *skb = lx_alloc_skb(len + 64, GFP_ATOMIC);
        if (skb) {
            lx_skb_reserve(skb, 0);
            unsigned char *p = lx_skb_put(skb, len);
            memcpy(p, ad->rx_bufs + i * BUF_LEN, len);
            lx_netif_rx(skb);
        }
        d->status = 0;
        ad->rx_tail = (uint16_t)((i + 1) % RX_DESC);
        rw(ad->mmio, REG_RDT, (uint32_t)((ad->rx_tail + RX_DESC - 1) % RX_DESC));
    }
}

static int e1000_probe(struct lx_pci_dev *pdev, const struct lx_pci_device_id *id)
{
    (void)id;
    struct lx_net_device *netdev = lx_alloc_etherdev((int)sizeof(struct e1000_adapter));
    if (!netdev)
        return -1;

    struct e1000_adapter *ad = lx_netdev_priv(netdev);
    memset(ad, 0, sizeof(*ad));
    ad->netdev = netdev;
    ad->pdev = pdev;
    lx_pci_set_drvdata(pdev, ad);

    if (lx_pci_enable_device(pdev) != 0)
        return -1;
    lx_pci_set_master(pdev);

    ad->mmio_base = (uint64_t)(uintptr_t)lx_pci_iomap(pdev, 0, 0);
    ad->mmio = (volatile uint32_t *)(uintptr_t)ad->mmio_base;

    uint8_t mac_before[6];
    read_mac_hw(ad->mmio, (uint16_t)lx_pci_device_id(pdev), mac_before);

    rw(ad->mmio, REG_CTRL, rr(ad->mmio, REG_CTRL) | CTRL_RST);
    for (int i = 0; i < 100000; i++) {
        if (!(rr(ad->mmio, REG_CTRL) & CTRL_RST))
            break;
    }
    rw(ad->mmio, REG_IMC, 0xffffffffu);
    (void)rr(ad->mmio, REG_ICR);

    read_mac_hw(ad->mmio, (uint16_t)lx_pci_device_id(pdev), ad->mac);
    if (!mac_ok(ad->mac))
        memcpy(ad->mac, mac_before, 6);
    if (!mac_ok(ad->mac))
        memcpy(ad->mac, (uint8_t[]){0x52, 0x54, 0x00, 0x12, 0x34, 0x56}, 6);

    rw(ad->mmio, REG_CTRL, rr(ad->mmio, REG_CTRL) | CTRL_SLU | CTRL_ASDE);
    for (int i = 0; i < 128; i++)
        rw(ad->mmio, REG_MTA + (uint32_t)i * 4, 0);

    uint32_t ral = (uint32_t)ad->mac[0] | ((uint32_t)ad->mac[1] << 8) |
                   ((uint32_t)ad->mac[2] << 16) | ((uint32_t)ad->mac[3] << 24);
    uint32_t rah = (uint32_t)ad->mac[4] | ((uint32_t)ad->mac[5] << 8) | (1u << 31);
    rw(ad->mmio, REG_RAL, ral);
    rw(ad->mmio, REG_RAH, rah);

    ad->rx = lx_dma_alloc_coherent(pdev, RX_DESC * sizeof(struct rx_desc), &ad->rx_ring_dma, GFP_KERNEL);
    ad->tx = lx_dma_alloc_coherent(pdev, TX_DESC * sizeof(struct tx_desc), &ad->tx_ring_dma, GFP_KERNEL);
    ad->rx_bufs = lx_dma_alloc_coherent(pdev, RX_DESC * BUF_LEN, &ad->rx_buf_dma, GFP_KERNEL);
    ad->tx_bufs = lx_dma_alloc_coherent(pdev, TX_DESC * BUF_LEN, &ad->tx_buf_dma, GFP_KERNEL);
    if (!ad->rx || !ad->tx || !ad->rx_bufs || !ad->tx_bufs)
        return -1;

    for (int i = 0; i < RX_DESC; i++) {
        ad->rx[i].addr = ad->rx_buf_dma + (uint64_t)i * BUF_LEN;
        ad->rx[i].status = 0;
    }
    for (int i = 0; i < TX_DESC; i++) {
        ad->tx[i].status = TX_DD;
    }
    ad->rx_tail = 0;
    ad->tx_tail = 0;

    rw(ad->mmio, REG_RDBAL, (uint32_t)ad->rx_ring_dma);
    rw(ad->mmio, REG_RDBAH, (uint32_t)(ad->rx_ring_dma >> 32));
    rw(ad->mmio, REG_RDLEN, RX_DESC * (uint32_t)sizeof(struct rx_desc));
    rw(ad->mmio, REG_RDH, 0);
    rw(ad->mmio, REG_RDT, (uint32_t)(RX_DESC - 1));

    rw(ad->mmio, REG_TDBAL, (uint32_t)ad->tx_ring_dma);
    rw(ad->mmio, REG_TDBAH, (uint32_t)(ad->tx_ring_dma >> 32));
    rw(ad->mmio, REG_TDLEN, TX_DESC * (uint32_t)sizeof(struct tx_desc));
    rw(ad->mmio, REG_TDH, 0);
    rw(ad->mmio, REG_TDT, 0);

    int nvec = lx_pci_alloc_irq_vectors(pdev, 1, 1, PCI_IRQ_MSIX);
    if (nvec < 1)
        return -1;
    ad->irq = lx_pci_irq_vector(pdev, 0);
    lx_request_irq((unsigned)ad->irq, e1000_irq, 0, "e1000e-lx", ad);

    lx_set_netdev_ops(netdev, e1000_open, e1000_stop, e1000_xmit);
    lx_set_netdev_mac(netdev, ad->mac);
    if (lx_register_netdev(netdev) != 0)
        return -1;

    g_adapter = ad;
    lx_printk("lx-e1000e: probe ok mac=%x:%x:%x:%x:%x:%x\n",
              ad->mac[0], ad->mac[1], ad->mac[2], ad->mac[3], ad->mac[4], ad->mac[5]);
    return 0;
}

static void e1000_remove(struct lx_pci_dev *pdev)
{
    struct e1000_adapter *ad = (struct e1000_adapter *)lx_pci_get_drvdata(pdev);
    if (ad && ad->netdev)
        lx_unregister_netdev(ad->netdev);
    g_adapter = 0;
}

static const struct lx_pci_device_id e1000_ids[] = {
    { VENDOR_INTEL, DEV_82574L, 0, 0, 0, 0, 0 },
    { VENDOR_INTEL, DEV_82540EM, 0, 0, 0, 0, 0 },
    { 0, 0, 0, 0, 0, 0, 0 },
};

static int e1000_driver_registered;

int lx_e1000e_init_module(void)
{
    if (e1000_driver_registered)
        return 0;
    int r = lx_pci_register_driver("e1000e", e1000_ids, e1000_probe, e1000_remove);
    if (r == 0)
        e1000_driver_registered = 1;
    return r;
}

void lx_e1000e_exit_module(void)
{
    g_adapter = 0;
}

struct e1000_adapter *lx_e1000e_adapter(void)
{
    return g_adapter;
}