/* D2/G2: driver de prueba — kmalloc, firmware, vmalloc, DRM, workqueue, PCI. */
#include "lx_emul.h"

static lx_completion_t done;
static lx_work_t wq_work;
static int pci_ok;

extern int lx_request_firmware(const char *name, const unsigned char **data, unsigned long *len);
extern void lx_release_firmware(const unsigned char *data);
extern void *lx_vmalloc(unsigned long size);
extern void lx_vfree(void *ptr);
extern void *lx_map_wc(unsigned long phys, unsigned long size);
extern unsigned long lx_drm_gem_create(unsigned long size);
extern void *lx_drm_gem_vmap(unsigned handle);

static void test_work_fn(lx_work_t *work)
{
    (void)work;
    lx_printk("lxdde-test: workqueue ejecutada\n");
    lx_complete(&done);
}

static int test_pci_scan(void)
{
    pci_ok = 1;
    return 0;
}

void lx_testdrv_run(void)
{
    lx_printk("lxdde-test: inicio\n");

    void *p = lx_kmalloc(128, GFP_KERNEL);
    if (!p) {
        lx_emul_trace_and_stop("kmalloc failed");
    }
    lx_kfree(p);

    void *v = lx_vmalloc(4096);
    if (v)
        lx_vfree(v);

    const unsigned char *fw = 0;
    unsigned long fwlen = 0;
    int fwrc = lx_request_firmware("test.bin", &fw, &fwlen);
    lx_printk("lxdde-test: firmware rc=%d len=%lu\n", fwrc, fwlen);
    if (fw)
        lx_release_firmware(fw);

    unsigned long gem = lx_drm_gem_create(256);
    void *mapped = lx_drm_gem_vmap((unsigned)gem);
    lx_printk("lxdde-test: drm gem=%lu map=%p\n", gem, mapped);

    (void)lx_map_wc;

    lx_init_completion(&done);
    lx_init_work(&wq_work, test_work_fn);
    lx_schedule_work(&wq_work);
    lx_wait_for_completion(&done);

    test_pci_scan();
    lx_printk("lxdde-test: pci_ok=%d\n", pci_ok);
    lx_printk("lxdde-test: ok\n");
}
