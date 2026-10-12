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

#include "gsp_ce.h"
#include "gsp_vram.h"
#include "gsp_dma.h"
#include "gsp_rm_obj.h"
#include "gsp_vmm.h"

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

/* Ventana propia dentro de la apertura. Va al FINAL, lo más lejos posible del
 * offset 0 donde RM mapea lo suyo: la granularidad de no-interferencia es una
 * entrada de PD1, o sea 512 MiB, así que la última ventana no puede compartir
 * PD1 con RM salvo que la apertura entera midiera menos de 512 MiB. */
#define GSP_BAR1_WINDOW_BYTES  0x200000ull   /* 2 MiB = una entrada de PD0 */

struct gsp_bar1 {
    uint64_t aperture_phys;  /* BAR1 del espacio de configuración */
    uint64_t aperture_size;  /* del BAR (RM da 0 en esta tarjeta) */
    uint64_t pd3;            /* `bar1PdeBase`: la raíz que construyó RM */
    uint64_t window_va;      /* VA (= offset en la apertura) de nuestra ventana */
    struct gsp_dma_buf pd0;  /* nivel 1 propio, si RM no tenía uno */
    struct gsp_dma_buf spt;  /* hoja propia: 512 PTEs de 4 KiB */
    uint64_t mapped_bytes;   /* cuántos bytes hay mapeados ahora en la ventana */
    /* Vaspace propio de BAR1, cuando lo atamos nosotros porque RM no lo hizo
     * (medido el 2026-08-02: `0xb80f40` a cero en la GB205). */
    struct gsp_vmm own;
    uint64_t inst_vram;      /* bloque de instancia de la apertura, en VRAM */
    int owned;               /* 1 = la apertura es nuestra */
    int ready;
};

/* 1 si `v` es un error PRI (`0xbadfxxxx` / `0xbad0xxxx` en cualquier dword),
 * no una dirección de tabla. GA107 run13: PD3 leído `0xbad0fb2fbad0fb2e`. */
int gsp_bar1_pri_poison(uint64_t v);

/* Recoge lo que ya sabemos de otras fases. Devuelve 0 si hay raíz y apertura
 * creíbles; no toca la tarjeta. Rechaza un `pd3` PRI-veneno. */
int gsp_bar1_init(struct gsp_bar1 *b, uint64_t aperture_phys,
                  uint64_t aperture_size, uint64_t pd3_vram);

/* Recorre las tablas de RM hasta donde se pueda leer y rellena `out` (hasta
 * `max` pasos). Devuelve cuántos pasos escribió, o -1. Se para en cuanto una
 * entrada es inválida o vive en una apertura que PRAMIN no alcanza: eso también
 * es un resultado, y decirlo es mejor que seguir leyendo basura. */
int gsp_bar1_walk(const struct gsp_bar1 *b, uint64_t bar1_va,
                  struct gsp_bar1_step *out, unsigned max);

/* Lee dónde ató RM el bloque de instancia de BAR1 (0xb80f40, el registro de
 * Turing+, no el 0x001704 de gf100) y de ahí el PDB que la MMU recorre de verdad
 * y el límite de VA del vaspace. Devuelve 0 si pudo leerlos; `pdb_out` y
 * `limit_out` son opcionales. Sólo lee: no toca nada. */
int gsp_bar1_inst_probe(const struct gsp_bar1 *b, uint64_t *pdb_out, uint64_t *limit_out);

/* Ata BAR1 nosotros mismos cuando RM no lo ha hecho: reserva un bloque de
 * instancia en VRAM, construye un vaspace propio, le escribe el PDB y el límite
 * como `gf100_vmm_join_` y lo enlaza con `0xb80f40` como `tu102_bar_bar1_init`.
 * A partir de ahí la apertura es nuestra y `gsp_bar1_map` usa el vaspace propio.
 * Devuelve 0 si quedó atado. */
int gsp_bar1_bind(struct gsp_bar1 *b, struct gsp_vram *vram);

/* El recorrido, por el log, en una línea por nivel. */
void gsp_bar1_dump(const struct gsp_bar1 *b, uint64_t bar1_va);

/* Mapea `bytes` de VRAM física en la ventana y devuelve el OFFSET dentro de la
 * apertura al que hay que sumar `aperture_phys` para que la CPU escriba ahí; 0
 * si no se pudo.
 *
 * Construye lo que falte de la cadena (PD0 y/o SPT) en sysmem —igual que el
 * resto de nuestras tablas, y RM acepta la apertura SYSMEM_COH en un PDE— y
 * enlaza el primer nivel que ya existiera de RM escribiéndolo por PRAMIN, que
 * es lo que hace nouveau con el PD3 que RM le regala (`r535_bar_bar1_init`).
 *
 * SE NIEGA si la entrada que tendría que escribir ya es válida: eso significaría
 * que RM tiene algo mapeado ahí, y pisarlo no se arregla — cuelga la tarjeta. En
 * ese caso hay que buscar otra ventana, no forzar ésta. */
uint64_t gsp_bar1_map(struct gsp_bar1 *b, uint64_t phys, uint64_t bytes);

/* Suelta la ventana: invalida las PTEs y desenlaza. Las tablas se quedan
 * reservadas para el siguiente mapeo (crearlas cuesta dos páginas y un
 * invalidate). */
void gsp_bar1_unmap(struct gsp_bar1 *b);

/* Prueba falsable de BAR1: mapea `vram_phys` en la ventana, escribe un patrón
 * POR LA APERTURA y lo relee con el CE desde `vram_va` (que es la misma memoria
 * vista por el otro camino). Devuelve 0 sólo si el patrón cuadra — o sea, sólo si
 * la CPU ha escrito VRAM de verdad. El scratch se borra antes de releer: sin eso,
 * un readback que no hiciera nada daría verde. Misma filosofía que
 * `gsp_ce_selftest`. */
int gsp_bar1_selftest(struct gsp_bar1 *b, struct gsp_ce *ce, uint64_t vram_phys,
                      uint64_t vram_va, uint64_t scratch_va, void *scratch_cpu);

#endif
