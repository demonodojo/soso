/* G4: apagado ordenado de GSP-RM. Ver gsp_fini.h. */
#include "gsp_fini.h"
#include "gsp_mmio.h"

/* Definidos en el shim (lxdde/shim/src/shims.c). */
void *memset(void *dst, int c, unsigned long n);

/* Falcon del GSP, los mismos offsets que usa fsp_lx.c (base 0x110000 de
 * `nvkm_falcon_ctor`). Se repiten aquí a propósito: son dos módulos
 * independientes y ninguno de los dos exporta su mapa de registros. */
#define NV_PGSP_FALCON      0x00110000u
#define NV_PFALCON_MAILBOX0 (NV_PGSP_FALCON + 0x040u)

/* Lo que GSP-RM deja en MAILBOX0 cuando ha terminado de descargarse. Sale del
 * `nvkm_msec(..., 2000, if (nvkm_falcon_rd32(&gsp->falcon, 0x040) == 0x80000000)`
 * de `r535_gsp_fini`: 0x040 es MAILBOX0 relativo a la base del falcon. */
#define GSP_UNLOAD_DONE 0x80000000u

/* Los dos plazos de upstream: 2 s para la respuesta del RPC y 2 s para que el
 * falcon publique el handshake. Acotados de verdad — el sentido de este módulo
 * es no colgarse dentro del apagado. */
#define UNLOAD_RPC_TIMEOUT_MS 2000u
#define UNLOAD_HALT_TIMEOUT_MS 2000u

/* --- Paso 1: soltar los objetos de RM ---------------------------------------
 *
 * En orden inverso al de la reserva. Si falla el del medio se sigue con el
 * resto: son handles que elegimos nosotros y RM los va a tirar igual cuando el
 * cliente muera, esto es cortesía para que no queden colgados si el GSP
 * sobrevive al ciclo (que es lo que pasa entre `cargo xtask run` y el
 * siguiente, con la tarjeta sin resetear del todo).
 */
static int free_rm_objects(struct gsp_rm *rm)
{
    int bad = 0;

    if (!rm || !rm->ready) {
        return 0;
    }
    if (gsp_rm_free(rm, rm->subdevice) != 0) {
        bad++;
    }
    if (gsp_rm_free(rm, rm->device) != 0) {
        bad++;
    }
    if (gsp_rm_free(rm, rm->client) != 0) {
        bad++;
    }
    rm->ready = 0;
    if (bad) {
        lx_printk("nouveau-lx: fini — %d de 3 objetos de RM no se soltaron\n", bad);
        return -1;
    }
    lx_printk("nouveau-lx: fini — objetos de RM soltados (sub, dev, cli)\n");
    return 0;
}

/* --- Paso 2: el aviso de descarga -------------------------------------------
 *
 * `r535_gsp_rpc_unloading_guest_driver(gsp, false)`. Los tres campos van a cero
 * pero se ponen explícitos: la rama `suspend` de upstream los llena con
 * `bInPMTransition=1` y `newLevel=GPU_LEVEL_3`, y cuando aquí haya suspensión
 * se verá exactamente qué toca cambiar.
 */
static int send_unload(struct gsp_cmdq *q, struct gsp_rpc *rpc)
{
    rpc_unloading_guest_driver_v1F_07 args;
    uint32_t status = 0;

    if (!q || !q->ready || !rpc || !rpc->ready) {
        lx_printk("nouveau-lx: fini — sin RPC vivo, no se avisa a GSP-RM\n");
        return -1;
    }
    memset(&args, 0, sizeof(args));
    args.bInPMTransition = 0;
    args.bGc6Entering = 0;
    args.newLevel = NV2080_CTRL_GPU_SET_POWER_STATE_GPU_LEVEL_0;

    if (gsp_cmdq_call(q, rpc, NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER,
                      &args, (uint32_t)sizeof(args), NULL, 0, NULL, &status,
                      UNLOAD_RPC_TIMEOUT_MS) != 0) {
        lx_printk("nouveau-lx: fini — UNLOADING_GUEST_DRIVER sin respuesta "
                  "(status=0x%x)\n", status);
        return -1;
    }
    lx_printk("nouveau-lx: fini — GSP-RM avisado (UNLOADING_GUEST_DRIVER ok)\n");
    return 0;
}

/* --- Paso 3: esperar el handshake del falcon --------------------------------
 *
 * Ojo con el all-ones (gotcha del 2026-07-25): si la GPU se ha ido del bus,
 * MAILBOX0 lee 0xffffffff. No colisiona con 0x80000000, así que no daría un
 * falso positivo, pero sí gastaría los 2 s enteros sondeando un cadáver.
 * `gsp_mmio_alive()` corta eso en la primera vuelta.
 */
static int wait_unload_done(uint32_t *last)
{
    unsigned waited = 0;

    for (;;) {
        uint32_t mbox0;

        if (!gsp_mmio_alive()) {
            lx_printk("nouveau-lx: fini — GPU fuera del bus esperando el unload\n");
            return -1;
        }
        mbox0 = gsp_mmio_rd32(NV_PFALCON_MAILBOX0);
        if (last) {
            *last = mbox0;
        }
        if (mbox0 == GSP_UNLOAD_DONE) {
            lx_printk("nouveau-lx: fini — GSP descargado (mbox0=0x%08x)\n", mbox0);
            return 0;
        }
        if (waited >= UNLOAD_HALT_TIMEOUT_MS) {
            lx_printk("nouveau-lx: fini — el GSP no confirmó la descarga en %u ms "
                      "(mbox0=0x%08x, esperaba 0x%08x)\n",
                      UNLOAD_HALT_TIMEOUT_MS, mbox0, GSP_UNLOAD_DONE);
            return -1;
        }
        lx_mdelay(1);
        waited++;
    }
}

/* --- Paso 4: cortar el DMA ---------------------------------------------------
 *
 * El único paso que no viene de upstream y el que de verdad protege al host.
 * Sin bus master la GPU no puede iniciar transacciones: da igual en qué estado
 * quedara el RISC-V, cuando vfio-pci resetee la función no habrá DMA en vuelo
 * contra un dominio IOMMU que ya no existe.
 *
 * Va el último porque los pasos 1 y 2 necesitan DMA para hablar por las colas.
 * Y va SIEMPRE, hayan salido bien o no: es precisamente cuando el apagado
 * ordenado falla cuando hace falta la red.
 */
static int stop_dma(struct lx_pci_dev *pdev)
{
    if (!pdev) {
        lx_printk("nouveau-lx: fini — sin pci_dev, el bus master se queda puesto\n");
        return -1;
    }
    lx_pci_clear_master(pdev);
    lx_printk("nouveau-lx: fini — bus master quitado (sin DMA)\n");
    return 0;
}

int gsp_fini(struct gsp_rm *rm, struct gsp_cmdq *q, struct gsp_rpc *rpc,
             struct lx_pci_dev *pdev)
{
    uint32_t mbox0 = 0;
    int objs, unload, halt, dma;

    lx_printk("nouveau-lx: apagando GSP-RM…\n");

    objs = free_rm_objects(rm);
    unload = send_unload(q, rpc);
    /* Sin aviso no hay handshake que esperar: el GSP no sabe que se va. */
    halt = unload == 0 ? wait_unload_done(&mbox0) : -1;
    dma = stop_dma(pdev);

    lx_printk("nouveau-lx: GSP-RM apagado (objetos=%s unload=%s halt=%s dma=%s)\n",
              objs == 0 ? "ok" : "fallo",
              unload == 0 ? "ok" : "fallo",
              halt == 0 ? "ok" : "fallo",
              dma == 0 ? "off" : "fallo");

    return (objs == 0 && unload == 0 && halt == 0 && dma == 0) ? 0 : -1;
}
