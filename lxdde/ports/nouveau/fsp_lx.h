/* G3b paso 6: envío del COT al FSP por EMEM.
 *
 * **Este es el primer módulo del port que ESCRIBE registros de la GPU.** Hasta
 * aquí todo era construir estructuras en RAM y leer estado. El COT le dice al FSP
 * dónde está la imagen GSP-FMC firmada y sus boot params; el FSP la verifica, la
 * arranca y el FMC levanta el GSP.
 *
 * Referencias upstream: `gh100_fsp_boot_gsp_fmc`, `gh100_fsp_send_sync`,
 * `gh100_fsp_{send,recv,poll,wait}` (`nvkm/subdev/fsp/gh100.c`),
 * `gb202_fsp` (variante de tamaños del COT), `gp102_flcn_emem_pio`
 * (`nvkm/falcon/gp102.c`) y `gh100_gsp_lockdown_released` (`subdev/gsp/gh100.c`).
 */
#ifndef FSP_LX_H
#define FSP_LX_H

#include "fmc_lx.h"
#include "gsp_libos.h"

/* Manda el COT y espera a que el FMC libere el lockdown del GSP.
 *
 * Devuelve 0 solo si el FSP respondió sin error **y** el GSP salió del lockdown.
 * Cualquier otra cosa es -1 con el motivo en el log; nunca se queda girando
 * (todas las esperas están acotadas).
 *
 * Precondiciones que comprueba antes de escribir nada: FSP con secure boot
 * completo, colas EMEM ociosas, y payload (FMC stageado + boot params) listo. */
int fsp_lx_boot_gsp_fmc(const struct fmc_staged *fmc, const struct gsp_libos *libos,
                        const struct gsp_wpr *wpr);

#endif
