/* Tablas de páginas VER3 y espacio de direcciones de RM. Ver gsp_vmm.h. */
#include "gsp_vmm.h"
#include "gsp_mmio.h"
#include "nvrm_r570.h"

void *memset(void *dst, int c, unsigned long n);

/* Geometría de los seis niveles, del más profundo (0, las hojas) al más alto.
 * `shift` es el bit de la VA donde empieza el índice de ese nivel; `bits`, su
 * anchura; `esize`, lo que ocupa una entrada. Sale tal cual de
 * `gh100_vmm_desc_12[]`, leído al revés (allí el primero es la hoja también). */
static const struct {
    uint8_t shift;
    uint8_t bits;
    uint8_t esize;
} g_level[GSP_VMM_LEVELS] = {
    { 12, 9,  8 },   /* SPT */
    { 21, 8, 16 },   /* PD0, PDE doble */
    { 29, 9,  8 },   /* PD1 */
    { 38, 9,  8 },   /* PD2 */
    { 47, 9,  8 },   /* PD3 */
    { 56, 1,  8 },   /* PD4, la raíz */
};

#define VMM_ROOT       (GSP_VMM_LEVELS - 1u)
#define VMM_PT_BYTES   4096u
#define VMM_PAGE       4096ull
#define VMM_VA_BITS    57u
#define VMM_ADDR_MASK  0x000ffffffffff000ull   /* ADDRESS 51:12 */

/* PRI de invalidación de la MMU del cliente (tu102_vmm_flush → dev_vm.h gb100,
 * alias físico 0xb83000). Nuestro directorio vive en sysmem coherente. */
#define VMM_INVAL_PDB         0xb830a0u
#define VMM_INVAL_UPPER_PDB   0xb830a4u
#define VMM_INVAL             0xb830b0u
#define VMM_INVAL_TRIGGER     0x80000000u
#define VMM_INVAL_ALL_VA      0x00000001u
#define VMM_INVAL_APERTURE_SYS 0x2u
#define VMM_INVAL_POLL_MS     2000u

/* Índice de `va` dentro de la tabla de nivel `lvl`. */
static uint32_t lvl_index(unsigned lvl, uint64_t va)
{
    return (uint32_t)((va >> g_level[lvl].shift) & ((1u << g_level[lvl].bits) - 1u));
}

/* VA que cubre la entrada 0 de la tabla de nivel `lvl` que contiene a `va`. Es
 * la clave con la que se reconoce una tabla ya creada. */
static uint64_t lvl_cover(unsigned lvl, uint64_t va)
{
    unsigned top = (unsigned)g_level[lvl].shift + g_level[lvl].bits;

    if (top >= 64u) {
        return 0;
    }
    return va & ~((1ull << top) - 1ull);
}

/* PTE de 4 KiB. Bit 0 VALID, APERTURE 2:1, PCF 7:3, KIND 11:8, ADDRESS 51:12.
 *
 * El PCF no es un número suelto: son cinco bits con estructura, y la tabla entera
 * está en `NV_MMU_VER3_PTE_PCF_*` de `nvhw/ref/gh100/dev_mmu.h` (transcrita el
 * 2026-07-29). Leídos de arriba abajo: bit 4 = ACD (frente a ACE), bit 3 =
 * NO_ATOMIC, **bit 2 = RO**, bit 1 = PRIVILEGE, bit 0 = UNCACHED. De ahí salen los
 * cuatro que usa este port, con su nombre de upstream y no a ojo:
 *
 *   0x10 REGULAR_RW_ATOMIC_CACHED_ACD     VRAM, lectura y escritura
 *   0x11 REGULAR_RW_ATOMIC_UNCACHED_ACD   sysmem, lectura y escritura
 *   0x14 REGULAR_RO_ATOMIC_CACHED_ACD     VRAM, sólo lectura
 *   0x15 REGULAR_RO_ATOMIC_UNCACHED_ACD   sysmem, sólo lectura
 *
 * Sysmem va sin cachear porque la CPU escribe ahí y no hay quien invalide la L2 de
 * la GPU; VRAM sí se cachea. Lo que NO se usa es la variante PRIVILEGE: upstream
 * mapea el contexto de GR con `priv = 1`, y la nuestra queda REGULAR, que es
 * **más** permisiva (un acceso privilegiado a una página regular pasa; al revés
 * no), así que no rompe nada y queda dicho aquí.
 *
 * KIND 0 = `PITCH`: memoria lineal sin tiling ni compresión, que es lo único
 * que se mapea aquí. */
#define VMM_PCF_REGULAR_RW_ATOMIC_CACHED_ACD   0x10u
#define VMM_PCF_REGULAR_RW_ATOMIC_UNCACHED_ACD 0x11u
#define VMM_PCF_REGULAR_RO_ATOMIC_CACHED_ACD   0x14u
#define VMM_PCF_REGULAR_RO_ATOMIC_UNCACHED_ACD 0x15u

uint64_t gsp_vmm_pte_encode(uint64_t phys, enum gsp_vmm_target target, unsigned flags)
{
    uint64_t d = 1ull;                                  /* VALID */
    unsigned pcf;

    if (target == GSP_VMM_VRAM) {
        pcf = (flags & GSP_VMM_RO) ? VMM_PCF_REGULAR_RO_ATOMIC_CACHED_ACD
                                   : VMM_PCF_REGULAR_RW_ATOMIC_CACHED_ACD;
    } else {
        pcf = (flags & GSP_VMM_RO) ? VMM_PCF_REGULAR_RO_ATOMIC_UNCACHED_ACD
                                   : VMM_PCF_REGULAR_RW_ATOMIC_UNCACHED_ACD;
    }
    d |= (uint64_t)(target == GSP_VMM_VRAM ? 0u : 2u) << 1;      /* APERTURE */
    d |= (uint64_t)pcf << 3;                                     /* PCF */
    d |= phys & VMM_ADDR_MASK;                          /* KIND = 0 */
    return d;
}

/* PDE. **El bit 0 se queda a cero**: ahí no hay un "válido" sino `IS_PTE`, y
 * ponerlo convierte el puntero a la tabla de abajo en una traducción final.
 * Válido = APERTURE distinto de 0, que además se codifica distinto que en el
 * PTE (VRAM es 1 aquí y 0 allí). PCF como en `gh100_vmm_pde`:
 * `VALID_CACHED_ATS_NOT_ALLOWED` (2) para VRAM, `VALID_UNCACHED_ATS_ALLOWED`
 * (1) para sysmem coherente.
 *
 * Vale también para la mitad pequeña de la PDE doble del nivel 1: sus campos
 * están en las mismas posiciones 64 bits más arriba (ver gsp_vmm.h). */
uint64_t gsp_vmm_pde_encode(uint64_t phys, enum gsp_vmm_target target)
{
    uint64_t d = 0ull;

    d |= (uint64_t)(target == GSP_VMM_VRAM ? 1u : 2u) << 1;   /* APERTURE */
    d |= (uint64_t)(target == GSP_VMM_VRAM ? 2u : 1u) << 3;   /* PCF */
    d |= phys & VMM_ADDR_MASK;
    return d;
}

static uint64_t pde_address(uint64_t raw)
{
    return raw & VMM_ADDR_MASK;
}

static unsigned pde_aperture(uint64_t raw)
{
    return (unsigned)((raw >> 1) & 3u);
}

/* Escribe una entrada. El nivel 1 son 16 B: la mitad baja (páginas grandes) se
 * deja a cero —solo se usan páginas de 4 KiB— y la alta lleva el PDE. */
static void pt_write(struct gsp_vmm_pt *pt, uint32_t index, uint64_t value)
{
    uint64_t *slot = (uint64_t *)pt->mem.va;

    if (g_level[pt->level].esize == 16) {
        slot[(unsigned long)index * 2u] = 0;
        slot[(unsigned long)index * 2u + 1u] = value;
    } else {
        slot[index] = value;
    }
}

static uint64_t pt_read(const struct gsp_vmm_pt *pt, uint32_t index)
{
    const uint64_t *slot = (const uint64_t *)pt->mem.va;

    if (g_level[pt->level].esize == 16) {
        return slot[(unsigned long)index * 2u + 1u];
    }
    return slot[index];
}

static struct gsp_vmm_pt *pt_find(struct gsp_vmm *v, unsigned lvl, uint64_t va)
{
    uint64_t cover = lvl_cover(lvl, va);
    unsigned i;

    for (i = 0; i < v->pt_nr; i++) {
        if (v->pt[i].used && v->pt[i].level == lvl && v->pt[i].cover == cover) {
            return &v->pt[i];
        }
    }
    return NULL;
}

/* Devuelve la tabla, creándola si hace falta. `*created` dice si es nueva, que
 * es cuando hay que enlazarla desde el nivel de arriba. */
static struct gsp_vmm_pt *pt_get(struct gsp_vmm *v, unsigned lvl, uint64_t va,
                                 int *created)
{
    struct gsp_vmm_pt *pt = pt_find(v, lvl, va);

    *created = 0;
    if (pt) {
        return pt;
    }
    if (v->pt_nr >= GSP_VMM_MAX_PT) {
        lx_printk("nouveau-lx: sin sitio para más tablas de páginas (%u)\n",
                  GSP_VMM_MAX_PT);
        return NULL;
    }
    pt = &v->pt[v->pt_nr];
    if (gsp_dma_alloc(&pt->mem, VMM_PT_BYTES, "tabla de páginas") != 0) {
        return NULL;
    }
    pt->level = lvl;
    pt->cover = lvl_cover(lvl, va);
    pt->used = 1;
    v->pt_nr++;
    *created = 1;
    return pt;
}

/* Paridad con tu102_vmm_flush: tras cada mapeo hay que barrer la TLB y las
 * cachés de PDE, o la GPU puede seguir viendo entradas inválidas (GR_FAULT_DURING
 * _CTXSW con mmuFaultType=PDE en una VA ya mapeada, ciclo 2026-07-29). */
static void gsp_vmm_invalidate(struct gsp_vmm *v)
{
    uint64_t root_phys;
    uint32_t type = VMM_INVAL_ALL_VA;
    unsigned waited = 0;

    if (!v || !v->bound || !v->pt_nr) {
        return;
    }
    root_phys = v->pt[0].mem.phys;
    gsp_mmio_wr32(VMM_INVAL_PDB,
                  (uint32_t)((root_phys >> 8) | VMM_INVAL_APERTURE_SYS));
    gsp_mmio_wr32(VMM_INVAL_UPPER_PDB, (uint32_t)(root_phys >> 40));
    gsp_mmio_wr32(VMM_INVAL, VMM_INVAL_TRIGGER | type);

    for (;;) {
        uint32_t busy = gsp_mmio_rd32(VMM_INVAL);

        if (gsp_mmio_pri_error(busy)) {
            lx_printk("nouveau-lx: MMU invalidate PRI error 0x%08x\n", busy);
            return;
        }
        if (!(busy & VMM_INVAL_TRIGGER)) {
            return;
        }
        if (waited >= VMM_INVAL_POLL_MS) {
            lx_printk("nouveau-lx: MMU invalidate timeout (0x%08x)\n", busy);
            return;
        }
        lx_mdelay(1);
        waited++;
    }
}

int gsp_vmm_map(struct gsp_vmm *v, uint64_t va, uint64_t phys, uint64_t size,
                enum gsp_vmm_target target)
{
    return gsp_vmm_map_flags(v, va, phys, size, target, 0u);
}

/* Una página, sin invalidar: baja el árbol creando lo que falte y escribe el PTE.
 * Separada para que un lote pueda pagar UNA invalidación en vez de N (ver
 * `gsp_vmm_map_pages`). */
static int map_one(struct gsp_vmm *v, uint64_t at, uint64_t phys,
                   enum gsp_vmm_target target, unsigned flags)
{
    struct gsp_vmm_pt *parent = pt_find(v, VMM_ROOT, at);
    unsigned lvl;

    if (!parent) {
        lx_printk("nouveau-lx: sin directorio raíz\n");
        return -1;
    }
    /* Bajar creando lo que falte y enlazando cada tabla nueva en su padre. */
    for (lvl = VMM_ROOT; lvl > 0; lvl--) {
        struct gsp_vmm_pt *child;
        int created = 0;

        child = pt_get(v, lvl - 1u, at, &created);
        if (!child) {
            return -1;
        }
        if (created) {
            pt_write(parent, lvl_index(lvl, at),
                     gsp_vmm_pde_encode(child->mem.phys, GSP_VMM_SYSMEM));
        }
        parent = child;
    }
    pt_write(parent, lvl_index(0, at), gsp_vmm_pte_encode(phys, target, flags));
    v->pages_mapped++;
    return 0;
}

int gsp_vmm_map_flags(struct gsp_vmm *v, uint64_t va, uint64_t phys, uint64_t size,
                      enum gsp_vmm_target target, unsigned flags)
{
    uint64_t off;

    if (!v || !v->ready || size == 0) {
        return -1;
    }
    if ((va | phys | size) & (VMM_PAGE - 1)) {
        lx_printk("nouveau-lx: mapeo sin alinear va=0x%llx phys=0x%llx size=0x%llx\n",
                  (unsigned long long)va, (unsigned long long)phys,
                  (unsigned long long)size);
        return -1;
    }
    if (va >= (1ull << VMM_VA_BITS) || size > (1ull << VMM_VA_BITS) - va) {
        lx_printk("nouveau-lx: VA 0x%llx fuera de los %u bits del espacio\n",
                  (unsigned long long)va, VMM_VA_BITS);
        return -1;
    }

    for (off = 0; off < size; off += VMM_PAGE) {
        if (map_one(v, va + off, phys + off, target, flags) != 0) {
            return -1;
        }
    }

    gsp_vmm_invalidate(v);
    return 0;
}

/* AVERÍA DE RENDIMIENTO (2026-08-17): la subida por DMA mapeaba el origen con una
 * llamada a `gsp_vmm_map` POR PÁGINA, y cada una acaba en `gsp_vmm_invalidate`:
 * tres escrituras MMIO más un sondeo que, si no acierta a la primera, espera con
 * `lx_mdelay(1)` — un milisegundo de granularidad. Un tensor de 44 MiB son 11 264
 * invalidaciones de MMU, y el «camino sin copias» salía más caro que el rebote que
 * venía a sustituir. Nadie lo había medido porque no había contadores.
 *
 * Escribir todos los PTE y barrer la TLB UNA vez al final es correcto por la misma
 * razón que lo era antes: la GPU no lee de estas VAs hasta el `LAUNCH_DMA`, que se
 * encola después. */
int gsp_vmm_map_pages(struct gsp_vmm *v, uint64_t va, const uint64_t *phys,
                      unsigned npages, enum gsp_vmm_target target)
{
    unsigned i;

    if (!v || !v->ready || !phys || npages == 0) {
        return -1;
    }
    if (va & (VMM_PAGE - 1)) {
        lx_printk("nouveau-lx: lote de mapeo sin alinear va=0x%llx\n",
                  (unsigned long long)va);
        return -1;
    }
    if (va >= (1ull << VMM_VA_BITS) ||
        (uint64_t)npages * VMM_PAGE > (1ull << VMM_VA_BITS) - va) {
        lx_printk("nouveau-lx: VA 0x%llx fuera de los %u bits del espacio\n",
                  (unsigned long long)va, VMM_VA_BITS);
        return -1;
    }
    for (i = 0; i < npages; i++) {
        if (phys[i] & (VMM_PAGE - 1)) {
            lx_printk("nouveau-lx: página %u del lote sin alinear (0x%llx)\n", i,
                      (unsigned long long)phys[i]);
            return -1;
        }
    }

    for (i = 0; i < npages; i++) {
        if (map_one(v, va + (uint64_t)i * VMM_PAGE, phys[i], target, 0u) != 0) {
            return -1;
        }
    }

    gsp_vmm_invalidate(v);
    return 0;
}

int gsp_vmm_translate(const struct gsp_vmm *v, uint64_t va, uint64_t *phys,
                      uint64_t *pte)
{
    const struct gsp_vmm_pt *pt;
    uint64_t entry;
    unsigned lvl;

    if (!v || !v->ready) {
        return -1;
    }
    /* El recorrido va por las direcciones que hay ESCRITAS en las tablas, no por
     * el índice de `pt[]`: si un PDE apuntase a otro sitio, esto se enteraría. */
    pt = pt_find((struct gsp_vmm *)v, VMM_ROOT, va);
    if (!pt) {
        return -1;
    }
    for (lvl = VMM_ROOT; ; lvl--) {
        entry = pt_read(pt, lvl_index(lvl, va));
        if (lvl == 0) {
            break;
        }
        if (pde_aperture(entry) == 0) {
            return -1;      /* APERTURE_INVALID: no hay tabla debajo */
        }
        pt = NULL;
        {
            uint64_t want = pde_address(entry);
            unsigned i;

            for (i = 0; i < v->pt_nr; i++) {
                if (v->pt[i].used && v->pt[i].level == lvl - 1u &&
                    v->pt[i].mem.phys == want) {
                    pt = &v->pt[i];
                    break;
                }
            }
        }
        if (!pt) {
            return -1;      /* el PDE apunta a una tabla que no es nuestra */
        }
    }

    if (!(entry & 1ull)) {
        return -1;          /* PTE inválido */
    }
    if (phys) {
        *phys = entry & VMM_ADDR_MASK;
    }
    if (pte) {
        *pte = entry;
    }
    return 0;
}

/* --- El lado de RM ----------------------------------------------------------- */

static int vaspace_alloc(struct gsp_vmm *v)
{
    NV_VASPACE_ALLOCATION_PARAMETERS args;
    uint32_t status = 0;

    memset(&args, 0, sizeof(args));
    args.index = NV_VASPACE_ALLOCATION_INDEX_GPU_NEW;
    args.flags = NV_VASPACE_ALLOCATION_FLAGS_IS_EXTERNALLY_OWNED;

    if (gsp_rm_alloc(&v->rm, v->rm.device, NVKM_RM_VASPACE, FERMI_VASPACE_A,
                     &args, (uint32_t)sizeof(args), &status) != 0) {
        lx_printk("nouveau-lx: FERMI_VASPACE_A rechazado (status=0x%x)\n", status);
        return -1;
    }
    v->vaspace = NVKM_RM_VASPACE;
    return 0;
}

static int page_directory_set(struct gsp_vmm *v, uint64_t root_phys)
{
    NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS ctrl;
    uint32_t status = 0;

    memset(&ctrl, 0, sizeof(ctrl));
    ctrl.physAddress = root_phys;
    /* 2, no 512: la raíz del formato VER3 indexa con **un solo bit** de la VA
     * (`gh100_vmm_desc_12[5]` tiene bits=1). Upstream lo escribe como
     * `1 << vmm->func->page[0].desc->bits`, que es fácil de leer como si fuera
     * el número de entradas de una tabla normal. */
    ctrl.numEntries = 2;
    ctrl.flags = NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_FLAGS_APERTURE_SYSMEM_COH;
    ctrl.hVASpace = v->vaspace;

    if (gsp_rm_control(&v->rm, v->rm.device, NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
                       &ctrl, (uint32_t)sizeof(ctrl), &status) != 0) {
        lx_printk("nouveau-lx: SET_PAGE_DIRECTORY rechazado (status=0x%x)\n", status);
        return -1;
    }
    v->bound = 1;
    return 0;
}

/* Vaspace SIN RM: sólo el directorio raíz en sysmem. Lo usa BAR1, que no le pide
 * nada a RM — su directorio se le entrega al hardware escribiéndolo en el bloque
 * de instancia de la apertura, no con `SET_PAGE_DIRECTORY`.
 *
 * La raíz es el MISMO nivel que la de un vaspace normal (PD4, 2 entradas) aunque
 * el espacio sea de 16 GiB: `nvkm_vmm_ctor` (`nvkm/subdev/mmu/vmm.c:1114-1131`)
 * recorre el descriptor hasta el final y coge el nivel más alto SIEMPRE; lo que
 * acota el espacio es el `limit` del bloque de instancia, no la profundidad. */
int gsp_vmm_init_bare(struct gsp_vmm *v)
{
    struct gsp_vmm_pt *root;
    int created = 0;

    if (!v) {
        return -1;
    }
    memset(v, 0, sizeof(*v));
    v->ready = 1;                 /* para que pt_get pueda trabajar */
    root = pt_get(v, VMM_ROOT, 0, &created);
    if (!root) {
        v->ready = 0;
        return -1;
    }
    /* `bound` es lo que habilita el invalidate en `gsp_vmm_map`. Aquí no hay RM
     * que acepte nada, pero el directorio existe y es nuestro. */
    v->bound = 1;
    return 0;
}

int gsp_vmm_init(struct gsp_cmdq *q, struct gsp_rpc *rpc, struct gsp_vmm *v)
{
    struct gsp_vmm_pt *root;
    int created = 0;

    if (!q || !rpc || !v) {
        return -1;
    }
    memset(v, 0, sizeof(*v));

    if (gsp_rm_client_new(q, rpc, &v->rm, 1) != 0) {
        lx_printk("nouveau-lx: sin cliente para el espacio de direcciones\n");
        return -1;
    }
    if (vaspace_alloc(v) != 0) {
        goto fail;
    }

    /* El directorio raíz tiene que existir antes del control: lo que se le pasa
     * a RM es su física, y RM la lee en cuanto la recibe. */
    v->ready = 1;    /* para que pt_get pueda trabajar */
    root = pt_get(v, VMM_ROOT, 0, &created);
    if (!root) {
        v->ready = 0;
        goto fail;
    }
    if (page_directory_set(v, root->mem.phys) != 0) {
        v->ready = 0;
        goto fail;
    }

    lx_printk("nouveau-lx: vaspace 0x%08x listo (externo, raíz=0x%llx en sysmem, "
              "2 entradas)\n", v->vaspace, (unsigned long long)root->mem.phys);
    return 0;

fail:
    gsp_vmm_fini(v);
    return -1;
}

void gsp_vmm_fini(struct gsp_vmm *v)
{
    unsigned i;

    if (!v) {
        return;
    }
    if (v->bound) {
        NV0080_CTRL_DMA_UNSET_PAGE_DIRECTORY_PARAMS ctrl;

        memset(&ctrl, 0, sizeof(ctrl));
        ctrl.hVASpace = v->vaspace;
        if (gsp_rm_control(&v->rm, v->rm.device,
                           NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY,
                           &ctrl, (uint32_t)sizeof(ctrl), NULL) != 0) {
            /* Se sigue de todas formas: el corte de DMA de `gsp_fini` es lo que
             * de verdad protege al host, y llega después. */
            lx_printk("nouveau-lx: UNSET_PAGE_DIRECTORY falló\n");
        }
        v->bound = 0;
    }
    if (v->vaspace) {
        gsp_rm_free(&v->rm, v->vaspace);
        v->vaspace = 0;
    }
    if (v->rm.ready) {
        gsp_rm_free(&v->rm, v->rm.subdevice);
        gsp_rm_free(&v->rm, v->rm.device);
        gsp_rm_free(&v->rm, v->rm.client);
        v->rm.ready = 0;
    }
    for (i = 0; i < v->pt_nr; i++) {
        if (v->pt[i].used) {
            gsp_dma_free(&v->pt[i].mem);
            v->pt[i].used = 0;
        }
    }
    v->pt_nr = 0;
    v->pages_mapped = 0;
    v->ready = 0;
}
