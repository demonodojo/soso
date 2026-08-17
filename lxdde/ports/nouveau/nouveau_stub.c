/* G3: probe NVIDIA + GSP bring-up gb205. */
#include "lx_emul.h"

extern void lx_nouveau_set_boot0(unsigned boot0, unsigned device_id);
extern int lx_nouveau_gsp_init(struct lx_pci_dev *pdev);
extern int lx_nouveau_gsp_ready(void);
extern const char *lx_nouveau_gsp_status(void);
extern int lx_nouveau_submit_saxpy(float a, const float *x, float *y, unsigned n);
extern int lx_nouveau_submit_matvec_f32(const float *w, unsigned rows, unsigned cols,
                                        const float *x, float *y);
extern int lx_nouveau_submit_matvec_resident(uint64_t w_va, unsigned rows,
                                             unsigned cols, const float *x,
                                             float *y);
extern int lx_nouveau_submit_matvec_q_resident(uint64_t w_va, unsigned dtype,
                                               unsigned rows, unsigned cols,
                                               const float *x, float *y);
extern uint64_t lx_nouveau_vram_bytes(void);
extern uint64_t lx_nouveau_buf_alloc(uint64_t size);
extern int lx_nouveau_buf_ready(void);
extern int lx_nouveau_buf_upload(uint64_t va, const void *src, uint64_t size);
extern int lx_nouveau_buf_upload_at(uint64_t va, uint64_t offset, const void *src,
                                    uint64_t size);
extern int lx_nouveau_buf_upload_dma(uint64_t va, uint64_t offset,
                                     const uint64_t *phys, unsigned npages,
                                     unsigned src_off, uint64_t size);
extern int lx_nouveau_buf_free(uint64_t va);
extern uint64_t lx_nouveau_buf_vram_free(void);

static int nouveau_probe(struct lx_pci_dev *pdev, const struct lx_pci_device_id *id)
{
    (void)id;
    unsigned dev = (unsigned)lx_pci_device_id(pdev);
    lx_printk("nouveau-lx: probe NVIDIA dev=0x%04x\n", dev);
    if (lx_nouveau_gsp_init(pdev) != 0) {
        lx_printk("nouveau-lx: GSP init falló (status=%s)\n", lx_nouveau_gsp_status());
        return -1;
    }
    return 0;
}

static void nouveau_remove(struct lx_pci_dev *pdev)
{
    (void)pdev;
}

static const struct lx_pci_device_id nouveau_ids[] = {
    /* Solo VGA/3D (clase 0x03). 0x228b es el HD Audio HDMI de la misma
     * tarjeta: su BAR0 son ~16 KiB y leer 0x1183a4 (VRAM) page-faultea. */
    { 0x10de, 0, 0, 0, 0x030000, 0xff0000, 0 },
    { 0, 0, 0, 0, 0, 0, 0 },
};

extern int lx_nvkm_build_gsp(void *bar0);

int lx_nouveau_init_module(void)
{
    int rc = lx_pci_register_driver("nouveau", nouveau_ids, nouveau_probe, nouveau_remove);
    /* Ola 3 self-test: valida que el grafo de objetos nvkm real (device +
     * subdev GSP + falcon) se construye. Sin HW: la construcción no toca MMIO.
     * En placa con GPU, nouveau_probe hará el bring-up completo con BAR0 real. */
    lx_printk("nouveau-lx: self-test grafo nvkm (sin HW)\n");
    lx_nvkm_build_gsp((void *)0);
    return rc;
}

int lx_nouveau_gsp_is_ready(void)
{
    return lx_nouveau_gsp_ready();
}

const char *lx_nouveau_gsp_phase(void)
{
    return lx_nouveau_gsp_status();
}

int lx_nouveau_compute_saxpy(float a, const float *x, float *y, unsigned n)
{
    return lx_nouveau_submit_saxpy(a, x, y, n);
}

int lx_nouveau_compute_matvec_f32(const float *w, unsigned rows, unsigned cols,
                                  const float *x, float *y)
{
    return lx_nouveau_submit_matvec_f32(w, rows, cols, x, y);
}

int lx_nouveau_compute_matvec_resident(uint64_t w_va, unsigned rows, unsigned cols,
                                       const float *x, float *y)
{
    return lx_nouveau_submit_matvec_resident(w_va, rows, cols, x, y);
}

int lx_nouveau_compute_matvec_q_resident(uint64_t w_va, unsigned dtype,
                                         unsigned rows, unsigned cols,
                                         const float *x, float *y)
{
    return lx_nouveau_submit_matvec_q_resident(w_va, dtype, rows, cols, x, y);
}

uint64_t lx_nouveau_vram_total(void)
{
    return lx_nouveau_vram_bytes();
}

uint64_t lx_nouveau_device_buf_alloc(uint64_t size)
{
    return lx_nouveau_buf_alloc(size);
}

int lx_nouveau_device_bufs_ready(void)
{
    return lx_nouveau_buf_ready();
}

int lx_nouveau_device_buf_upload(uint64_t va, const void *src, uint64_t size)
{
    return lx_nouveau_buf_upload(va, src, size);
}

int lx_nouveau_device_buf_upload_at(uint64_t va, uint64_t offset, const void *src,
                                    uint64_t size)
{
    return lx_nouveau_buf_upload_at(va, offset, src, size);
}

int lx_nouveau_device_buf_upload_dma(uint64_t va, uint64_t offset,
                                     const uint64_t *phys, unsigned npages,
                                     unsigned src_off, uint64_t size)
{
    return lx_nouveau_buf_upload_dma(va, offset, phys, npages, src_off, size);
}

int lx_nouveau_device_buf_free(uint64_t va)
{
    return lx_nouveau_buf_free(va);
}

uint64_t lx_nouveau_device_vram_free(void)
{
    return lx_nouveau_buf_vram_free();
}
