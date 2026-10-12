/* G4: apagado ordenado de GSP-RM antes de soltar la tarjeta.
 *
 * Por qué existe: el 2026-07-25 una prueba VFIO **colgó el host** (no fue un
 * panic — `efi_pstore` está registrado en esa máquina y capturó el GPF del día
 * anterior, pero de este cuelgue no quedó ni un registro: la CPU no llegó al
 * handler). soso había arrancado el GSP, hablado por RPC y llegado al shell; el
 * `timeout 90` del script mató QEMU y vfio-pci reseteó una GPU cuyo RISC-V
 * seguía ejecutando GSP-RM y haciendo DMA contra un dominio IOMMU que se estaba
 * desmontando. No había — literalmente, `grep` de `_fini` no daba nada — ningún
 * camino de apagado en el port.
 *
 * Réplica de `r535_gsp_fini` (`rm/r535/gsp.c`) en su rama no-suspend:
 *
 *   1. soltar los objetos de RM (subdevice → device → cliente)
 *   2. `UNLOADING_GUEST_DRIVER` con `bInPMTransition=0`, `bGc6Entering=0`,
 *      `newLevel=GPU_LEVEL_0`
 *   3. esperar a que `MAILBOX0` del falcon GSP valga `0x80000000`
 *   4. quitar el bus master — esto ya no es de upstream, es la red de seguridad:
 *      si algo de lo anterior falló, al menos la GPU no puede seguir haciendo
 *      DMA cuando el host le pase el reset por encima
 *
 * Cada paso es best-effort y acotado en tiempo: un apagado a medias es mejor
 * que quedarse colgado dentro del apagado. El paso 4 se ejecuta pase lo que
 * pase con los tres primeros, que es justo lo que faltaba el día del cuelgue.
 *
 * Deliberadamente NO se resetea el falcon a mano: los bits de reset del RISC-V
 * de GB20x no están verificados en este árbol, y vfio-pci va a resetear la
 * función igualmente. La diferencia que aporta este módulo es que cuando llegue
 * ese reset, RM estará avisado y el DMA parado.
 */
#ifndef GSP_FINI_H
#define GSP_FINI_H

#include "gsp_rm_obj.h"

struct lx_pci_dev;

/* Apaga GSP-RM y deja la tarjeta sin DMA. `rm`, `q` y `rpc` pueden venir a
 * medio inicializar (se comprueba `ready` de cada uno): lo que esté vivo se
 * apaga y lo que no, se salta. `pdev` puede ser NULL, en cuyo caso no se toca
 * el bus master y se avisa por el log.
 *
 * Devuelve 0 solo si los cuatro pasos salieron bien; el valor es para el log y
 * los tests, no para decidir nada — el llamante suelta la tarjeta igual. */
int gsp_fini(struct gsp_rm *rm, struct gsp_cmdq *q, struct gsp_rpc *rpc,
             struct lx_pci_dev *pdev);

#endif
