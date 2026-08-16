/* Detección de familia de chip y valores chip-aware (doorbell gb20x, etc.). */
#ifndef GSP_CHIP_H
#define GSP_CHIP_H

#include <stdint.h>

enum nv_family {
    NV_FAM_UNKNOWN = 0,
    NV_FAM_AMPERE,
    NV_FAM_ADA,
    NV_FAM_BLACKWELL,
};

#define GB205_DEVICE_ID  0x2f18u  /* RTX 5070 Ti Mobile — también en gsp_chip.c */
#define GA107_DEVICE_ID  0x249cu  /* RTX 3050 Mobile */

int gsp_nv_boot0_valid(uint32_t boot0);
void gsp_nv_family_set(uint32_t boot0, uint16_t device_id);
uint32_t gsp_nv_family_boot0(void);
uint16_t gsp_nv_family_device_id(void);
enum nv_family gsp_nv_family_of(uint32_t boot0, uint16_t device_id);
enum nv_family gsp_nv_family_current(void);
const char *gsp_nv_family_name(enum nv_family f);
/* Directorio linux-firmware del die Ampere (ga107 en 3050 Mobile, ga102 por defecto). */
const char *gsp_nv_ampere_chip_name(uint16_t device_id);
const char *gsp_nv_chip_name(uint16_t device_id, enum nv_family fam);

/* Valor a escribir en NV_VFN_DOORBELL. `rpc_token` es lo que devuelve
 * GET_WORK_SUBMIT_TOKEN; en gb20x hay que OR el bit RUNLIST_DOORBELL_ENABLE
 * (30) que ese RPC no incluye — ver gb202/dev_vm.h y gb202_chan_doorbell_handle. */
uint32_t gsp_chan_doorbell_kick(enum nv_family fam, uint32_t rpc_token);

#endif
