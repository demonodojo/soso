/* G4c: objetos de RM sobre la llamada síncrona.
 *
 * GSP-RM no se maneja con registros sino con un modelo de objetos: se reserva un
 * cliente, y colgando de él un device y un subdevice. De ahí para arriba salen el
 * espacio de direcciones, la memoria y el canal — es decir, todo G4d/G4e.
 *
 * Los handles los elegimos nosotros (`rm/handles.h`), no los devuelve RM: en la
 * reserva se manda el que queremos usar.
 *
 * Referencias: `r535_gsp_client_ctor` (`rm/r535/client.c`),
 * `r535_gsp_device_ctor`/`r535_gsp_subdevice_ctor` (`rm/r535/device.c`),
 * `r535_gsp_rpc_rm_alloc_{get,push}` (`rm/r535/alloc.c`) y
 * `r535_gsp_rpc_rm_ctrl_{get,push}` (`rm/r535/ctrl.c`).
 */
#ifndef GSP_RM_OBJ_H
#define GSP_RM_OBJ_H

#include "gsp_cmdq.h"

struct gsp_rm {
    struct gsp_cmdq *q;
    struct gsp_rpc *rpc;
    uint32_t client;       /* handle del NV01_ROOT */
    uint32_t device;       /* NV01_DEVICE_0 */
    uint32_t subdevice;    /* NV20_SUBDEVICE_0 */
    int ready;
};

/* Reserva un objeto de clase `cls` colgado de `parent` con el handle `handle`.
 * `params`/`params_size` es el bloque de parámetros de esa clase; se copia detrás
 * de la cabecera. Devuelve 0 solo si el RPC llegó **y** el `status` que RM pone
 * dentro del wrapper es 0. `rm_status`, si se pasa, recibe ese NV_STATUS. */
int gsp_rm_alloc(struct gsp_rm *rm, uint32_t parent, uint32_t handle, uint32_t cls,
                 const void *params, uint32_t params_size, uint32_t *rm_status);

/* Invoca un control sobre `object`. `params` es de entrada y salida: se manda tal
 * cual y se sobreescribe con lo que devuelva RM (hasta `params_size`). */
int gsp_rm_control(struct gsp_rm *rm, uint32_t object, uint32_t cmd,
                   void *params, uint32_t params_size, uint32_t *rm_status);

/* Igual con timeout explícito (ms). PROMOTE_CTX y otros controles lentos lo usan. */
int gsp_rm_control_timeout(struct gsp_rm *rm, uint32_t object, uint32_t cmd,
                           void *params, uint32_t params_size, uint32_t *rm_status,
                           unsigned timeout_ms);

/* Libera un objeto (`NV_VGPU_MSG_FUNCTION_FREE`). */
int gsp_rm_free(struct gsp_rm *rm, uint32_t handle);

/* La cadena cliente → device → subdevice para el cliente número `client_id`.
 * Los handles se cuentan **por cliente**, así que dos clientes distintos usan el
 * mismo 0xde1d0000 de device sin pisarse — es lo que hace upstream, donde cada
 * VMM promocionado se trae su propio cliente (`r535_mmu_vaspace_new`). */
int gsp_rm_client_new(struct gsp_cmdq *q, struct gsp_rpc *rpc, struct gsp_rm *rm,
                      unsigned client_id);

/* El cliente 0, el del bring-up. */
int gsp_rm_init(struct gsp_cmdq *q, struct gsp_rpc *rpc, struct gsp_rm *rm);

/* Catálogo de clases del chip (`GET_CLASSLIST_V2`), una vez tras los objetos de
 * RM. Sin él las clases se piden a ciegas; con él, `gsp_rm_class_pick` elige. */
int gsp_rm_classes_probe(struct gsp_rm *rm);

/* 1 = sí, 0 = no, **-1 = no se sabe** (nadie ha pedido el catálogo o falló). Los
 * tres casos son distintos y colapsar el -1 con el 0 haría que un catálogo
 * ausente pareciera un chip sin clases. */
int gsp_rm_class_supported(uint32_t cls);

/* La primera de `cand` que el chip reconozca, de más nueva a más vieja. Sin
 * catálogo devuelve `cand[0]` diciéndolo. Nunca devuelve 0 salvo lista vacía. */
uint32_t gsp_rm_class_pick(const char *what, const uint32_t *cand, unsigned n);

/* Sólo para pruebas: tira el catálogo para que el siguiente escenario empiece
 * sin saber nada. En el kernel no hay nada que lo llame. */
void gsp_rm_classes_forget(void);

/* Motores del chip (`GPU_GET_ENGINES_V2`) y topología del FIFO
 * (`FIFO_GET_DEVICE_INFO_TABLE`), los dos sobre el subdevice. Por ahora sólo
 * vuelca: contesta si el `engineType` que pide el canal existe de verdad en esta
 * tarjeta, en vez de darlo por bueno por aritmética sobre una tabla. Devuelve 0
 * si los dos controles pasaron; que fallen no impide arrancar. */
int gsp_rm_engines_probe(struct gsp_rm *rm);

/* Dónde están los registros del FIFO de `engine` (`NV2080_ENGINE_TYPE_*`): el pri
 * base de su runlist y el de su channel RAM, tal como los dio la tabla de arriba.
 * Devuelve 0 si el motor estaba en la tabla y su base cae dentro de BAR0. Es para
 * diagnóstico —mirar en el silicio si el canal está encendido y si el PBDMA se ha
 * quejado—, no para programar nada: en el camino GSP el FIFO lo lleva RM. */
int gsp_rm_engine_fifo_regs(uint32_t engine, uint32_t *runl_pri,
                            uint32_t *chram_pri);

/* Lo que sacamos de `GET_GSP_STATIC_INFO`: el mapa de VRAM utilizable y los
 * regalos de RM (su juego interno de objetos y las bases de las tablas de
 * páginas de BAR1/BAR2, que RM ya ha construido). */
struct gsp_fb_region {
    uint64_t base;
    uint64_t size;
};

#define GSP_FB_REGION_MAX 16u

struct gsp_static_info {
    uint64_t fb_length;             /* VRAM total según RM */
    struct gsp_fb_region region[GSP_FB_REGION_MAX];
    unsigned region_nr;             /* solo las utilizables */
    uint64_t usable_bytes;          /* suma de las anteriores */
    uint64_t bar1_pde_base;
    uint64_t bar2_pde_base;
    uint32_t internal_client;
    uint32_t internal_device;
    uint32_t internal_subdevice;
    uint32_t l2_cache_size;
    char name[65];                  /* gpuNameString, con NUL */
    int ready;
};

/* Pide la configuración estática. `vram_expected` es la VRAM que ya conocemos
 * por otra vía (el registro que lee `gsp_wpr`): si no cuadra con `fb_length`, la
 * transcripción del struct está desplazada y se avisa. Pasar 0 para no contrastar. */
int gsp_static_info_get(struct gsp_rm *rm, uint64_t vram_expected,
                        struct gsp_static_info *out);

#endif
