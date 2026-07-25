/* G4d: la VRAM la reparte el driver, no RM.
 *
 * Contra lo que uno esperaría del modelo de objetos, GSP-RM **no** es quien da
 * memoria de vídeo: nouveau se la reparte él mismo (`r535_fb_ram_new` monta un
 * `nvkm_mm` sobre las regiones que vinieron en `GET_GSP_STATIC_INFO` y ahí se
 * acaba la intervención de RM). Por eso G4d 1/2 se molestó en quedarse con la
 * lista de regiones utilizables: son el mapa del que sale todo lo demás.
 *
 * Lo de aquí es deliberadamente un asignador de puntero que avanza y **no sabe
 * liberar**. G4d y G4e reservan un puñado de bloques al arrancar y los tienen
 * hasta el apagado; un `nvkm_mm` de verdad —bloques, huecos, coalescencia— es
 * trabajo de cuando haya que meter y sacar pesos de un modelo, no ahora. Está
 * escrito para que se note: `gsp_vram_free` no existe.
 */
#ifndef GSP_VRAM_H
#define GSP_VRAM_H

#include "gsp_rm_obj.h"

struct gsp_vram_region {
    uint64_t base;
    uint64_t end;      /* exclusivo */
    uint64_t next;     /* primer byte libre */
};

struct gsp_vram {
    struct gsp_vram_region region[GSP_FB_REGION_MAX];
    unsigned region_nr;
    uint64_t total;    /* suma de las regiones */
    uint64_t used;
    int ready;
};

/* Toma las regiones utilizables de la static info. Falla si no hay ninguna:
 * seguir con cero VRAM sería reservar direcciones inventadas. */
int gsp_vram_init(struct gsp_vram *v, const struct gsp_static_info *info);

/* Devuelve la dirección física en VRAM, o 0 si no cabe. `align` se redondea a
 * página como mínimo. El 0 como "no hay" es seguro aquí: la VRAM utilizable
 * nunca empieza en 0 en esta tarjeta, y aun así `gsp_vram_init` lo comprueba. */
uint64_t gsp_vram_alloc(struct gsp_vram *v, uint64_t size, uint64_t align);

#endif
