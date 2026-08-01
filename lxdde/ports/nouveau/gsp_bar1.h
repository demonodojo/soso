/* BAR1: la ventana con la que la CPU escribe VRAM sin canal.
 *
 * POR QUÉ. Hoy todo lo que va a VRAM pasa por el CE: rebote en sysmem, copia y
 * espera de semáforo. Upstream no hace eso para los accesos de CPU — tiene una
 * apertura de PCI (BAR1) cuyas tablas de páginas construye el driver, y escribir
 * en VRAM es un `memcpy` a esa apertura. Con GSP-RM las tablas ya existen: RM
 * las construye y devuelve la raíz en `bar1PdeBase` (`GspStaticConfigInfo`), que
 * es exactamente lo que hace `r535_bar_bar1_init` en nouveau — envolver el PD3
 * de RM y mapear encima.
 *
 * QUÉ HAY AQUÍ Y QUÉ NO. Esto es el paso de MIRAR, no el de mapear: recorre las
 * tablas de RM leyéndolas por PRAMIN y cuenta lo que encuentra. El mapeo de
 * verdad depende de cosas que no se pueden adivinar desde el sofá — en qué nivel
 * está la raíz de RM, si los niveles de abajo existen ya o hay que crearlos, y
 * en qué apertura viven. Inventarse esas tres respuestas y escribir PDEs encima
 * de las estructuras de RM es la clase de cambio que se depura a un ciclo de
 * VFIO por intento; con el volcado de un solo arranque se escriben con datos.
 *
 * FORMATO: el mismo VER3 de `gsp_vmm.h` (Hopper/Blackwell), y las mismas dos
 * trampas: en un PDE el bit 0 NO es "válido" sino `IS_PTE`, y la APERTURE se
 * codifica distinto en PDE (VRAM=1, 0=INVALID) que en PTE (VRAM=0, bit 0
 * VALID). El nivel PD0 tiene entradas de 16 B y la mitad que importa —la de
 * páginas de 4 KiB— es la ALTA.
 */
#ifndef GSP_BAR1_H
#define GSP_BAR1_H

#include "gsp_rm_obj.h"

/* PD3 (entradas de 2^47) → PD2 → PD1 → PD0 → SPT. La raíz de un vaspace de BAR
 * es PD3 y no PD4: `r535_bar_bar2_update_pde` manda `entryLevelShift = 47`, que
 * es justo el nivel cuyas entradas cubren 2^47. */
#define GSP_BAR1_LEVELS  5u

struct gsp_bar1_step {
    unsigned level;      /* 4 = PD3 … 0 = SPT (hoja) */
    uint64_t table;      /* dirección de VRAM de la tabla leída */
    uint32_t index;      /* entrada dentro de ella */
    uint64_t entry;      /* el dato crudo, tal cual */
    unsigned aperture;   /* campo APERTURE del PDE/PTE */
    uint64_t next;       /* a dónde apunta (tabla de abajo o página) */
};

struct gsp_bar1 {
    uint64_t aperture_phys;  /* BAR1 del espacio de configuración */
    uint64_t aperture_size;  /* lo que dice RM; 0 = no lo sabemos */
    uint64_t pd3;            /* `bar1PdeBase`: la raíz que construyó RM */
    int ready;
};

/* Recoge lo que ya sabemos de otras fases. Devuelve 0 si hay raíz y apertura
 * creíbles; no toca la tarjeta. */
int gsp_bar1_init(struct gsp_bar1 *b, uint64_t aperture_phys,
                  uint64_t aperture_size, uint64_t pd3_vram);

/* Recorre las tablas de RM hasta donde se pueda leer y rellena `out` (hasta
 * `max` pasos). Devuelve cuántos pasos escribió, o -1. Se para en cuanto una
 * entrada es inválida o vive en una apertura que PRAMIN no alcanza: eso también
 * es un resultado, y decirlo es mejor que seguir leyendo basura. */
int gsp_bar1_walk(const struct gsp_bar1 *b, uint64_t bar1_va,
                  struct gsp_bar1_step *out, unsigned max);

/* El recorrido, por el log, en una línea por nivel. */
void gsp_bar1_dump(const struct gsp_bar1 *b, uint64_t bar1_va);

#endif
