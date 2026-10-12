/* Accesor mínimo del grafo de dispositivo nvkm.
 *
 * `nvkm_device_subdev` vive en `engine/device/base.c` (3268 LOC con la tabla de
 * TODOS los chipsets, que arrastraría cientos de constructores como dummies).
 * Aquí replicamos solo el accesor real (idéntico a upstream) sobre la lista
 * `device->subdev`, sin la tabla de chips. El resto de `engine/device` se porta
 * cuando se necesite el bring-up completo del dispositivo.
 */
#include <core/device.h>
#include <core/subdev.h>
#include <core/engine.h>

struct nvkm_subdev *
nvkm_device_subdev(struct nvkm_device *device, int type, int inst)
{
	struct nvkm_subdev *subdev;

	list_for_each_entry(subdev, &device->subdev, head) {
		if (subdev->type == type && subdev->inst == inst)
			return subdev;
	}

	return NULL;
}

struct nvkm_engine *
nvkm_device_engine(struct nvkm_device *device, int type, int inst)
{
	struct nvkm_subdev *subdev = nvkm_device_subdev(device, type, inst);

	if (subdev && subdev->func == &nvkm_engine)
		return container_of(subdev, struct nvkm_engine, subdev);

	return NULL;
}
