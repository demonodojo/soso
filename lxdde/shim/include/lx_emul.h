/* Capa lx_emul — emulación mínima del entorno Linux para drivers portados. */
#ifndef LX_EMUL_H
#define LX_EMUL_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
#include <stdarg.h>

/* --- Trazas y parada (stub generator) --- */
void lx_emul_trace_and_stop(const char *func) __attribute__((noreturn));
void lx_emul_trace(const char *func);

/* --- Memoria --- */
void *lx_kmalloc(size_t size, unsigned flags);
void *lx_kzalloc(size_t size, unsigned flags);
void *lx_krealloc(void *ptr, size_t size, unsigned flags);
void lx_kfree(void *ptr);
void *lx_vmalloc(unsigned long size);
void lx_vfree(void *ptr);
/* Buffer alineado a página (para tablas de páginas del GSP); no garantiza
 * contigüidad física — traducir página a página con lx_virt_to_phys. */
void *lx_alloc_pages_exact(size_t size);
void lx_free_pages_exact(void *ptr, size_t size);
uint64_t lx_virt_to_phys(const void *ptr);

#define GFP_KERNEL 0x40u
#define GFP_ATOMIC 0x20u
#define __GFP_ZERO 0x8000u

/* --- Printk --- */
int lx_printk(const char *fmt, ...);
int lx_vprintk(const char *fmt, va_list ap);

/* Volcado inmediato de SOSOLOG.TXT (checkpoints de arranque). */
void lx_fatlog_flush(void);

/* --- Tiempo --- */
unsigned long lx_jiffies(void);
unsigned long lx_msecs_to_jiffies(unsigned int m);
unsigned int lx_jiffies_to_msecs(unsigned long j);
void lx_udelay(unsigned long us);
void lx_mdelay(unsigned int ms);
uint64_t lx_ktime_get_ns(void);

/* --- Fibras / scheduling --- */
void lx_schedule(void);
void lx_yield(void);
void lx_msleep(unsigned int ms);

/* --- Completions --- */
struct lx_completion {
    unsigned int done;
};
typedef struct lx_completion lx_completion_t;
void lx_init_completion(lx_completion_t *c);
void lx_complete(lx_completion_t *c);
void lx_wait_for_completion(lx_completion_t *c);
unsigned long lx_wait_for_completion_timeout(lx_completion_t *c, unsigned long timeout_jiffies);

/* --- Workqueues --- */
struct lx_work {
    void (*fn_ptr)(struct lx_work *work);
    int pending;
};
typedef struct lx_work lx_work_t;
typedef void (*lx_work_fn_t)(lx_work_t *work);
void lx_init_work(lx_work_t *work, lx_work_fn_t fn);
int lx_schedule_work(lx_work_t *work);
void lx_flush_work(lx_work_t *work);

struct lx_delayed_work {
    struct lx_work work;
    unsigned long deadline_ms;
};
typedef struct lx_delayed_work lx_delayed_work_t;
void lx_init_delayed_work(lx_delayed_work_t *dwork, lx_work_fn_t fn);
int lx_schedule_delayed_work(lx_delayed_work_t *dwork, unsigned long delay_jiffies);
bool lx_cancel_delayed_work(lx_delayed_work_t *dwork);

/* --- IRQ --- */
typedef unsigned irqreturn_t;
#define IRQ_NONE 0u
#define IRQ_HANDLED 1u
#define IRQ_WAKE_THREAD 2u
typedef irqreturn_t (*lx_irq_handler_t)(int irq, void *dev_id);
int lx_request_irq(unsigned int irq, lx_irq_handler_t handler, unsigned long flags,
                     const char *name, void *dev);
void lx_free_irq(unsigned int irq, void *dev);
void lx_irq_wake(unsigned int irq);

/* --- PCI --- */
struct lx_pci_dev;
struct lx_pci_device_id {
    uint32_t vendor, device, subvendor, subdevice, class, class_mask;
    unsigned long driver_data;
};
int lx_pci_register_driver(const char *name,
                           const struct lx_pci_device_id *ids,
                           int (*probe)(struct lx_pci_dev *, const struct lx_pci_device_id *),
                           void (*remove)(struct lx_pci_dev *));
void *lx_pci_iomap(struct lx_pci_dev *dev, int bar, unsigned long max_len);
/* Base de la apertura de FB (BAR1) y su tamaño por `size_out`. Los dos vienen de
 * la enumeración: dimensionar un BAR en caliente exige escribirle unos, y el
 * bring-up del GSP lo pediría con la tarjeta ya en marcha. 0 = sin BAR1. */
uint64_t lx_pci_bar1(struct lx_pci_dev *dev, uint64_t *size_out);
void lx_pci_iounmap(struct lx_pci_dev *dev, void *addr);
int lx_pci_enable_device(struct lx_pci_dev *dev);
void lx_pci_disable_device(struct lx_pci_dev *dev);
void lx_pci_set_master(struct lx_pci_dev *dev);
/* Quita el bit de bus master: la GPU deja de poder iniciar DMA. Lo usa el
 * apagado de GSP (gsp_fini.c) para que un reset de vfio-pci no pille
 * transacciones en vuelo. */
void lx_pci_clear_master(struct lx_pci_dev *dev);
uint32_t lx_pci_read_config(struct lx_pci_dev *dev, int offset, int size);
void lx_pci_write_config(struct lx_pci_dev *dev, int offset, uint32_t val, int size);
int lx_pci_alloc_irq_vectors(struct lx_pci_dev *dev, unsigned int min, unsigned int max, unsigned int flags);
void lx_pci_free_irq_vectors(struct lx_pci_dev *dev);
int lx_pci_irq_vector(struct lx_pci_dev *dev, unsigned int nr);
void *lx_pci_get_drvdata(struct lx_pci_dev *dev);
void lx_pci_set_drvdata(struct lx_pci_dev *dev, void *data);

#define PCI_IRQ_MSIX 0x8u

/* --- DMA --- */
void *lx_dma_alloc_coherent(struct lx_pci_dev *dev, size_t size, uint64_t *dma_handle, unsigned gfp);
/* Igual, pero con el mapeo de CPU **cacheado** (write-back), que es lo que
 * devuelve `dma_alloc_coherent` de Linux en x86 porque el DMA de PCIe fisga la
 * caché. Sólo para buffers que la CPU ESCRIBE en volumen y el dispositivo LEE:
 * un `memcpy` de 1 MiB a memoria UC tarda dos órdenes de magnitud más. No usarlo
 * para anillos donde el dispositivo escribe (ahí sí hace falta UC). Se libera con
 * `lx_dma_free_coherent`. */
void *lx_dma_alloc_wb(struct lx_pci_dev *dev, size_t size, uint64_t *dma_handle, unsigned gfp);
/* Baja a RAM las líneas de caché del rango y ordena la escritura. Obligatorio
 * tras escribir en memoria de `lx_dma_alloc_wb` y antes de que el dispositivo la
 * lea: una GPU puede pedir la lectura en modo no-snoop y saltarse la caché. */
void lx_dma_flush_range(const void *ptr, size_t len);
void lx_dma_free_coherent(struct lx_pci_dev *dev, size_t size, void *cpu_addr, uint64_t dma_handle);
uint64_t lx_dma_map_single(struct lx_pci_dev *dev, void *ptr, size_t size, int dir);
void lx_dma_unmap_single(struct lx_pci_dev *dev, uint64_t dma_addr, size_t size, int dir);

#define DMA_BIDIRECTIONAL 0
#define DMA_TO_DEVICE 1
#define DMA_FROM_DEVICE 2

/* --- Initcalls --- */
typedef int (*lx_initcall_fn_t)(void);
void lx_register_initcall(lx_initcall_fn_t fn, int level);
#define lx_module_init(fn) \
    static void lx_modinit_##fn(void) { lx_register_initcall(fn, 6); } \
    static void __attribute__((constructor)) lx_ctor_##fn(void) { lx_modinit_##fn(); }

/* --- Net bridge (mini net-core) --- */
struct lx_net_device;
struct lx_sk_buff;
struct lx_net_device *lx_alloc_etherdev(int priv_size);
int lx_register_netdev(struct lx_net_device *dev);
void lx_unregister_netdev(struct lx_net_device *dev);
void *lx_netdev_priv(struct lx_net_device *dev);
void lx_netif_rx(struct lx_sk_buff *skb);
struct lx_sk_buff *lx_alloc_skb(unsigned int size, unsigned gfp);
void lx_kfree_skb(struct lx_sk_buff *skb);
unsigned char *lx_skb_put(struct lx_sk_buff *skb, unsigned int len);
void lx_skb_reserve(struct lx_sk_buff *skb, int len);
int lx_dev_queue_xmit(struct lx_sk_buff *skb);
void lx_netif_start_queue(struct lx_net_device *dev);
void lx_netif_stop_queue(struct lx_net_device *dev);
bool lx_netif_queue_stopped(struct lx_net_device *dev);
void lx_netif_carrier_on(struct lx_net_device *dev);
void lx_eth_hw_addr_random(struct lx_net_device *dev);
int lx_eth_mac_addr(struct lx_net_device *dev, void *addr);
unsigned lx_skb_len(struct lx_sk_buff *skb);
const unsigned char *lx_skb_data(struct lx_sk_buff *skb);
void lx_set_netdev_ops(struct lx_net_device *dev,
                       int (*open)(struct lx_net_device *),
                       int (*stop)(struct lx_net_device *),
                       int (*xmit)(struct lx_sk_buff *, struct lx_net_device *));
void lx_set_netdev_mac(struct lx_net_device *dev, const unsigned char *mac);
uint16_t lx_pci_device_id(struct lx_pci_dev *pdev);
uint32_t lx_pci_bdf(struct lx_pci_dev *pdev);
unsigned lx_tx_head(void *adapter);

int lx_request_firmware(const char *name, const unsigned char **data, unsigned long *len);
void lx_release_firmware(const unsigned char *data);
void *lx_map_wc(unsigned long phys, unsigned long size);
unsigned long lx_drm_gem_create(unsigned long size);
void *lx_drm_gem_vmap(unsigned handle);

/* --- Spike / test / port entrypoints --- */
void lx_spike_run(void);
void lx_testdrv_run(void);
int lx_e1000e_init_module(void);
void lx_e1000e_exit_module(void);

#endif /* LX_EMUL_H */
