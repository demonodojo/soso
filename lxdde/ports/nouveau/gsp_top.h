/* PTOP: la topología de motores que el propio chip publica en BAR0.
 *
 * Hasta ahora las direcciones de las runlists salían de adivinar qué índice de
 * `engineData` de la tabla del FIFO significaba qué (`data[11]`, `data[14]`), y en
 * GB205 la apuesta falló: leer ahí dio `chcfg=0x00000002` y `0xbadf5040`
 * (2026-07-28). PTOP no es una apuesta — es una tabla enumerable que el chip
 * rellena con el tipo de cada motor, su bloque de registros, su runlist y su id
 * de fallo, y es de donde nouveau saca exactamente esto (`ga100_top_parse`).
 *
 * Referencia: `nvkm/subdev/top/ga100.c`. Tamaño en `0x0224fc >> 20`, entradas de
 * hasta tres palabras en `0x022800 + i*4` encadenadas por el bit 31.
 *
 * Sólo lee. No programa nada: en el camino GSP el FIFO lo lleva RM.
 */
#ifndef GSP_TOP_H
#define GSP_TOP_H

#include <stdint.h>

/* Tipos PTOP que nos interesan (`ga100_top_parse`). El resto se vuelca por número. */
#define GSP_TOP_TYPE_GR     0x00u
#define GSP_TOP_TYPE_SEC2   0x0du
#define GSP_TOP_TYPE_NVENC  0x0eu
#define GSP_TOP_TYPE_NVDEC  0x10u
#define GSP_TOP_TYPE_IOCTRL 0x12u
#define GSP_TOP_TYPE_CE     0x13u
#define GSP_TOP_TYPE_GSP    0x14u
#define GSP_TOP_TYPE_NVJPG  0x15u
#define GSP_TOP_TYPE_OFA    0x16u

#define GSP_TOP_MAX 32u

struct gsp_top_engine {
    uint32_t addr;      /* bloque de registros del motor (0x00fff000) */
    uint32_t runlist;   /* pri base de su runlist (0x00fffc00) */
    uint8_t type;       /* tipo PTOP */
    uint8_t inst;       /* instancia dentro del tipo (CE0, CE1, …) */
    uint8_t engine;     /* índice del motor dentro de su runlist */
    uint8_t reset;      /* bit de reset en PMC */
    uint8_t fault;      /* id de fallo de MMU */
};

/* Lee y vuelca la tabla. Devuelve el número de motores encontrados, o -1 si PTOP
 * no contesta (all-ones, error de PRI o un tamaño imposible). */
int gsp_top_probe(void);

/* La runlist de `type`/`inst` según PTOP. Devuelve 0 si estaba en la tabla. */
int gsp_top_runlist_of(uint8_t type, uint8_t inst, uint32_t *runlist,
                       uint32_t *addr);

/* Traduce un `NV2080_ENGINE_TYPE_*` a tipo+instancia de PTOP. Devuelve 0 si sabe
 * hacerlo — hay motores de RM que no tienen equivalente y decirlo es mejor que
 * devolver el 0, que es justo GR. */
int gsp_top_type_of_engine(uint32_t engine, uint8_t *type, uint8_t *inst);

#endif
