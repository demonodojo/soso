/* Puente Ola 3: construye un nvkm_device real y ejercita el grafo de objetos
 * nvkm (ga102_gsp_new → nvkm_subdev_ctor + nvkm_falcon_ctor) con BAR0 real en
 * device->pri. La construcción NO toca MMIO (solo asigna campos/listas), así
 * que valida el grafo nvkm sin hardware. El boot GSP efectivo (falcon reset,
 * WPR, RPC) sí requiere HW y llega tras G1. Ruta gated: no altera el soft boot.
 */
#include <core/device.h>
#include <subdev/gsp.h>
/* lx_printk/lx_kzalloc/lx_kfree vienen del prelude nvkm (printk.h/slab.h).
 * No incluir lx_emul.h: sus decls legacy chocan con los shims del prelude. */

/* Construye el device y crea el subdev GSP ga102. Devuelve 0 si el grafo se
 * construye; !=0 si falla. bar0 puede ser NULL (construcción pura). */
int lx_nvkm_build_gsp(void *bar0)
{
	struct nvkm_device *device;
	struct nvkm_gsp *gsp = NULL;
	int ret;

	device = lx_kzalloc(sizeof(*device), GFP_KERNEL);
	if (!device) {
		lx_printk("nouveau-lx: nvkm device alloc falló\n");
		return -1;
	}
	INIT_LIST_HEAD(&device->subdev);
	device->pri = bar0;       /* BAR0 real (para MMIO posterior) */
	device->cfgopt = (const char *)0;
	device->dbgopt = (const char *)0;

	ret = ga102_gsp_new(device, NVKM_SUBDEV_GSP, 0, &gsp);
	if (ret || !gsp) {
		lx_printk("nouveau-lx: nvkm ga102_gsp_new ret=%d\n", ret);
		lx_kfree(device);
		return ret ? ret : -1;
	}

	lx_printk("nouveau-lx: nvkm device graph OK — subdev='%s' falcon.func=%p pri=%p\n",
		  gsp->subdev.name, (void *)gsp->falcon.func, bar0);
	/* No liberamos: gsp cuelga de device->subdev; vida = proceso (soft). */
	return 0;
}
