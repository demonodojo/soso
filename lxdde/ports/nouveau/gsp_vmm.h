/* G4d (2/2): espacio de direcciones de la GPU y sus tablas de páginas.
 *
 * QUIÉN CONSTRUYE LAS TABLAS. El espacio se pide a RM (`FERMI_VASPACE_A`), pero
 * con `IS_EXTERNALLY_OWNED`: las tablas las construye el driver y a RM solo se
 * le dice dónde está la raíz, con `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`. Es
 * lo que hace nouveau para el VMM que promociona (`r535_mmu_promote_vmm` pasa
 * `external=true`) y no hay atajo por el otro lado: el camino "que las lleve RM"
 * también exige un directorio propio, y encima copiarle los PDE de su región
 * reservada (`NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES`).
 *
 * DÓNDE VIVEN. En sysmem coherente, no en VRAM. Upstream las pone en VRAM porque
 * tiene instmem y BAR1; aquí la CPU no tiene hoy ninguna ventana con la que
 * escribir VRAM, así que el directorio va a sysmem y el `FLAGS_APERTURE` del
 * control se pone a `SYSMEM_COH` en vez del `VIDMEM` de upstream. La GPU lee
 * sysmem por DMA igual que lee las colas del RPC, que es lo que lleva
 * funcionando desde G4a.
 *
 * FORMATO: VER3, el de Hopper/Blackwell (`nvhw/ref/gh100/dev_mmu.h`,
 * `vmmgh100.c`). Seis niveles para 4 KiB de página, del más profundo al más alto:
 *
 *   nivel 0  SPT   bits 20:12   512 entradas de  8 B   PTE
 *   nivel 1  PD0   bits 28:21   256 entradas de 16 B   PDE doble (grande+pequeña)
 *   nivel 2  PD1   bits 37:29   512 entradas de  8 B
 *   nivel 3  PD2   bits 46:38   512 entradas de  8 B
 *   nivel 4  PD3   bits 55:47   512 entradas de  8 B
 *   nivel 5  PD4   bit  56      2 entradas de  8 B     ← la raíz
 *
 * 57 bits de VA. La raíz tiene DOS entradas, y ese 2 es el `numEntries` que pide
 * el control (upstream: `1 << vmm->func->page[0].desc->bits`, y ese `bits` es 1).
 *
 * TRES TRAMPAS DEL FORMATO, y las tres son silenciosas:
 *
 *  1. **El bit 0 de un PDE NO es "válido", es `IS_PTE`.** Ponerlo a 1 "por
 *     analogía con el PTE" le dice a la MMU que la entrada es una traducción
 *     final, no un puntero a la tabla de abajo. Lo que hace válido a un PDE es
 *     que su APERTURE no sea 0 (`INVALID`). Upstream nunca escribe ese bit
 *     (`gh100_vmm_pde`), y por eso es fácil no darse cuenta de que existe.
 *  2. **APERTURE se codifica distinto en PTE y en PDE.** En el PTE, VRAM = 0;
 *     en el PDE, VRAM = 1 y el 0 significa INVALID. Sysmem coherente es 2 en los
 *     dos, que es justo la coincidencia que hace que el error no se vea si solo
 *     se prueba con tablas en sysmem.
 *  3. La mitad "pequeña" de la PDE doble del nivel 1 tiene **exactamente el mismo
 *     reparto de bits** que un PDE normal, 64 bits más arriba: APERTURE en 66:65
 *     (= 2:1 de la palabra alta), PCF en 69:67 (= 5:3) y ADDRESS en 115:76
 *     (= 51:12). Parece demasiada suerte, pero está así en dev_mmu.h y es lo que
 *     permite que aquí haya una sola `pde_encode`.
 *
 * Referencias: `r535_mmu_vaspace_new` (`rm/r535/vmm.c`), `gh100_vmm_*`
 * (`subdev/mmu/vmmgh100.c`), `rm/r535/nvrm/vmm.h`, `nvhw/ref/gh100/dev_mmu.h`.
 */
#ifndef GSP_VMM_H
#define GSP_VMM_H

#include "gsp_dma.h"
#include "gsp_rm_obj.h"

#define GSP_VMM_LEVELS   6u
/* Tablas vivas a la vez. Con PTEs de 4 KiB cada hoja cubre 2 MiB. G4d/G4e
 * bastaban con ~13 tablas; la promoción del grctx de gb205 (~56 MiB, solo
 * ATTRIBUTE_CB ~51552 KiB ≈ 26 hojas) suma ~29 más → ~42 en total. El pool de
 * 24 del bring-up CE se agotaba justo al mapear ATTRIBUTE_CB (2026-07-29). 96
 * deja margen 2x; cada tabla sigue reservándose bajo demanda en sysmem. */
#define GSP_VMM_MAX_PT  96u

/* Base de TODO el mapa de VAs del bring-up (G4d, canal, CE, compute). Estaba
 * repetida a mano en cinco constantes de tres ficheros, todas empezando por
 * 0x0000010000000000; ahora se deriva de aquí.
 *
 * **512 GiB y no 1 TiB.** El 1 TiB de antes es exactamente 2^40, o sea con el bit
 * 40 puesto, y una entrada de GPFIFO no puede direccionar tan alto: `clc56f.h`
 * define `GP_ENTRY0_GET` en 31:2 y `GP_ENTRY1_GET_HI` en **7:0**, ocho bits, así
 * que el pushbuffer tiene que vivir por debajo de 2^40. Con la base en 1 TiB el
 * `& 0xff` del encoder tiraba el bit 40 sin decir nada y el host iba a buscar el
 * pushbuffer a otro sitio: canal en la runlist, doorbell escrito y el semáforo
 * del CE a 0 para siempre (2026-07-28).
 *
 * El límite es SOLO del GPFIFO: los operandos de los métodos del CE
 * (`OFFSET_IN/OUT`) y el `gpFifoOffset` del alloc son de 64 bits de verdad. Pero
 * como el mapa es uno, se baja entero en vez de dejar una isla que cabe rodeada
 * de otras que no.
 *
 * 512 GiB (bit 39) sigue estando lejísimos de cualquier cosa real y cabe: sus
 * bits 39:32 son 0x80, que entra justo en GET_HI. */
#define GSP_VA_BASE     0x0000008000000000ull
/* La VA más alta que una entrada de GPFIFO puede expresar, para el guardia del
 * encoder: GET (31:2) + GET_HI (7:0) llegan al bit 39. */
#define GSP_GPFIFO_VA_MAX  0x000000ffffffffffull

enum gsp_vmm_target {
    GSP_VMM_VRAM = 0,
    GSP_VMM_SYSMEM = 1,     /* coherente */
};

struct gsp_vmm_pt {
    struct gsp_dma_buf mem;
    uint64_t cover;         /* VA que cubre la entrada 0 de esta tabla */
    unsigned level;
    int used;
};

struct gsp_vmm {
    struct gsp_rm rm;       /* cliente/device/subdevice propios del vaspace */
    uint32_t vaspace;
    struct gsp_vmm_pt pt[GSP_VMM_MAX_PT];
    unsigned pt_nr;
    unsigned pages_mapped;
    int bound;              /* RM aceptó el directorio */
    int ready;
};

/* Cliente + device + subdevice + `FERMI_VASPACE_A` + directorio raíz, y el
 * control que se lo entrega a RM. Como en upstream, el vaspace cuelga de un
 * cliente **propio** (`r535_mmu_vaspace_new` llama a `nvkm_gsp_client_device_ctor`
 * en vez de reutilizar el del GSP); los handles van por cliente, así que el
 * device vuelve a ser 0xde1d0000 sin chocar con el de G4c. */
int gsp_vmm_init(struct gsp_cmdq *q, struct gsp_rpc *rpc, struct gsp_vmm *v);

/* Vaspace sin RM: sólo el directorio raíz. Para BAR1, que entrega su directorio
 * por el bloque de instancia de la apertura y no por `SET_PAGE_DIRECTORY`. */
int gsp_vmm_init_bare(struct gsp_vmm *v);

/* Codificadores del formato VER3, expuestos porque BAR1 construye sus propias
 * tablas y NO debe reimplementarlos: las tres trampas del formato (el bit 0 de un
 * PDE no es «válido» sino IS_PTE, la APERTURE se codifica distinto en PTE que en
 * PDE, y la mitad que cuenta de la PDE doble del nivel 1 es la ALTA) están
 * explicadas arriba y valen para los dos usuarios. Dos copias de esto serían dos
 * sitios donde equivocarse en silencio. */
uint64_t gsp_vmm_pte_encode(uint64_t phys, enum gsp_vmm_target target, unsigned flags);
uint64_t gsp_vmm_pde_encode(uint64_t phys, enum gsp_vmm_target target);

/* Mapea `size` bytes desde `phys` en `va`. Los tres han de estar alineados a
 * 4 KiB. Crea las tablas que falten por el camino. */
int gsp_vmm_map(struct gsp_vmm *v, uint64_t va, uint64_t phys, uint64_t size,
                enum gsp_vmm_target target);

/* Sólo lectura para la GPU: el PCF del PTE pasa de `REGULAR_RW_*` a `REGULAR_RO_*`
 * (`NV_MMU_VER3_PTE_PCF_*`, bit 2 del campo). Lo pide el contexto de GR para el
 * mapa de acceso privilegiado, que upstream mapea con `.ro = 1`. */
#define GSP_VMM_RO 0x1u

/* Igual que `gsp_vmm_map` pero con banderas. `gsp_vmm_map` es esto con 0, para no
 * tocar los ocho sitios que no necesitan ninguna. */
int gsp_vmm_map_flags(struct gsp_vmm *v, uint64_t va, uint64_t phys, uint64_t size,
                      enum gsp_vmm_target target, unsigned flags);

/* Mapea `npages` físicas (dispersas) en VAs consecutivas desde `va`, con UNA sola
 * invalidación de MMU al final en vez de una por página.
 *
 * Existe por rendimiento y no por comodidad: `gsp_vmm_invalidate` son tres
 * escrituras MMIO más un sondeo cuyo reintento espera un milisegundo, así que la
 * subida por DMA de un tensor de 44 MiB pagaba 11 264 de ellas y salía más cara
 * que el rebote. Escribir todos los PTE antes de barrer es correcto porque la GPU
 * no lee estas VAs hasta el `LAUNCH_DMA`, que se encola después. */
int gsp_vmm_map_pages(struct gsp_vmm *v, uint64_t va, const uint64_t *phys,
                      unsigned npages, enum gsp_vmm_target target);

/* Recorre las tablas ya construidas como lo haría la MMU y devuelve a qué
 * física traduce `va`. -1 si algún nivel falta o está inválido. Existe para
 * poder comprobar el mapeo sin la GPU: se construye por un camino y se lee por
 * otro. `pte`, si se pasa, recibe la entrada en crudo. */
int gsp_vmm_translate(const struct gsp_vmm *v, uint64_t va, uint64_t *phys,
                      uint64_t *pte);

/* Quita el directorio, libera el vaspace y su cliente, y suelta las tablas.
 * Se llama ANTES de `gsp_fini`: mientras RM tenga apuntado nuestro directorio,
 * esas páginas de sysmem no se pueden devolver al heap. */
void gsp_vmm_fini(struct gsp_vmm *v);

#endif
