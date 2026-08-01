/* Recorrido de las tablas de BAR1 que construyó RM. Ver gsp_bar1.h. */
#include "gsp_bar1.h"
#include "gsp_mmio.h"
#include "gsp_pramin.h"
#include "gsp_vmm.h"

void *memset(void *dst, int c, unsigned long n);

/* Geometría de los cinco niveles que puede tener un vaspace de BAR, del más
 * profundo al más alto. Es el mismo `gh100_vmm_desc_12[]` que usa `gsp_vmm.c`,
 * recortado por arriba: la raíz de BAR es PD3. */
static const struct {
    uint8_t shift;
    uint8_t bits;
    uint8_t esize;
    const char *name;
} g_lvl[GSP_BAR1_LEVELS] = {
    { 12, 9,  8, "SPT" },
    { 21, 8, 16, "PD0" },
    { 29, 9,  8, "PD1" },
    { 38, 9,  8, "PD2" },
    { 47, 9,  8, "PD3" },
};

#define BAR1_ADDR_MASK  0x000ffffffffff000ull   /* ADDRESS 51:12 */

/* 0 = INVALID, 1 = VRAM, 2 = SYSMEM_COH, 3 = SYSMEM_NONCOH (PDE). */
static const char *aperture_name(unsigned ap)
{
    switch (ap) {
    case 0u: return "INVALID";
    case 1u: return "VRAM";
    case 2u: return "SYS_COH";
    default: return "SYS_NONCOH";
    }
}

static uint64_t vram_rd64(uint64_t addr)
{
    uint64_t lo = gsp_pramin_rd32(addr);
    uint64_t hi = gsp_pramin_rd32(addr + 4u);

    return lo | (hi << 32);
}

int gsp_bar1_init(struct gsp_bar1 *b, uint64_t aperture_phys,
                  uint64_t aperture_size, uint64_t pd3_vram)
{
    if (!b) {
        return -1;
    }
    b->aperture_phys = aperture_phys;
    b->aperture_size = aperture_size;
    b->pd3 = pd3_vram;
    b->ready = 0;

    /* Una raíz a cero no es "todavía no": es que RM no la ha dado o que el
     * struct de la config estática está desplazado, y en los dos casos lo que
     * salga del recorrido sería inventado. */
    if (!pd3_vram || (pd3_vram & 0xfffull)) {
        lx_printk("nouveau-lx: BAR1 — bar1PdeBase=0x%llx no es una tabla "
                  "alineada a página\n", (unsigned long long)pd3_vram);
        return -1;
    }
    if (!aperture_phys) {
        lx_printk("nouveau-lx: BAR1 — sin apertura en el espacio de "
                  "configuración\n");
        return -1;
    }
    b->ready = 1;
    return 0;
}

int gsp_bar1_walk(const struct gsp_bar1 *b, uint64_t bar1_va,
                  struct gsp_bar1_step *out, unsigned max)
{
    uint64_t table;
    unsigned lvl;
    unsigned n = 0;

    if (!b || !b->ready || !out || !max) {
        return -1;
    }
    if (!gsp_pramin_alive()) {
        return -1;
    }

    table = b->pd3;
    for (lvl = GSP_BAR1_LEVELS; lvl-- > 0 && n < max;) {
        struct gsp_bar1_step *s = &out[n];
        uint32_t idx = (uint32_t)((bar1_va >> g_lvl[lvl].shift) &
                                  ((1u << g_lvl[lvl].bits) - 1u));
        uint64_t off = (uint64_t)idx * g_lvl[lvl].esize;
        uint64_t raw;

        /* PD0 son 16 B por entrada y la mitad que traduce páginas de 4 KiB es
         * la ALTA; la baja es la de páginas grandes, que aquí no se usan. */
        if (g_lvl[lvl].esize == 16u) {
            off += 8u;
        }
        raw = vram_rd64(table + off);

        s->level = lvl;
        s->table = table;
        s->index = idx;
        s->entry = raw;
        s->next = raw & BAR1_ADDR_MASK;
        if (lvl == 0u) {
            /* Hoja: el bit 0 es VALID de verdad y la APERTURE se codifica al
             * revés que en un PDE (VRAM = 0). Se deja el campo crudo y quien
             * lea el volcado ya sabe que en este nivel se lee distinto. */
            s->aperture = (unsigned)((raw >> 1) & 3u);
            n++;
            break;
        }
        s->aperture = (unsigned)((raw >> 1) & 3u);
        n++;
        if (s->aperture == 0u) {
            break;      /* rama sin construir: no hay nada más abajo */
        }
        if (s->aperture != 1u) {
            /* Vive en sysmem: PRAMIN no llega. Pararse aquí y decirlo es el
             * resultado — leer la tabla de abajo por una ventana de VRAM daría
             * números con buena pinta y sin ninguna relación con la realidad. */
            break;
        }
        table = s->next;
    }
    return (int)n;
}

/* Registros de invalidación de la MMU del cliente, los mismos que usa
 * `gsp_vmm.c` (tu102_vmm_flush → dev_vm.h gb100). Aquí el PDB es el de RM y vive
 * en VRAM, así que la apertura del campo va a 0 y no a SYS. */
#define BAR1_INVAL_PDB        0xb830a0u
#define BAR1_INVAL_UPPER_PDB  0xb830a4u
#define BAR1_INVAL            0xb830b0u
#define BAR1_INVAL_TRIGGER    0x80000000u
#define BAR1_INVAL_ALL_VA     0x00000001u
/* HUB_ONLY | ALL_PDB. **Obligatorio para un vaspace de BAR** y no es opcional:
 * `tu102_vmm_flush` (`nvkm/subdev/mmu/vmmtu102.c:32-34`) añade justo estos dos
 * bits cuando el VMM tiene referencia del subdev BAR —con el comentario
 * «HUB_ONLY | ALL PDB (hack)»—. La MMU de BAR vive en el HUB, así que sin
 * HUB_ONLY el invalidate no le llega.
 *
 * AVERÍA (2026-08-01): con sólo PAGE_ALL, la MMU se quedaba con la traducción
 * INVÁLIDA cacheada de la ventana y descartaba en silencio todo lo que la CPU
 * escribía por la apertura. Se medía como «el CE relee la VRAM y encuentra el
 * patrón anterior», o sea indistinguible de «la escritura no salió del host». */
#define BAR1_INVAL_HUB_ALL_PDB 0x00000006u
#define BAR1_INVAL_POLL_MS    2000u

/* Escribe una entrada de 64 bits en una tabla que vive en VRAM.
 *
 * El dword ALTO primero y el bajo después, a propósito: los bits que hacen
 * válida a la entrada (APERTURE, 2:1) están en el bajo, así que con este orden la
 * MMU nunca puede ver una entrada medio escrita que ya parezca válida y apunte a
 * una dirección a medias. Al revés sí. */
static void vram_wr64(uint64_t addr, uint64_t val)
{
    gsp_pramin_wr32(addr + 4u, (uint32_t)(val >> 32));
    gsp_pramin_wr32(addr, (uint32_t)val);
}

/* Igual, pero RELEYENDO. Devuelve 0 si el dato se quedó puesto.
 *
 * Escribir una tabla de RM y no comprobarlo era el agujero de este fichero: los
 * LECTORES por PRAMIN están validados contra silicio (el recorrido cuadra con la
 * cadena real), pero de una ESCRITURA por PRAMIN a VRAM no había ninguna prueba.
 * Si RM protege esa región, o la ventana está mal, el PDE no se queda y todo lo
 * de abajo —invalidate incluido— trabaja sobre una tabla que no cambió, con un
 * síntoma idéntico al de una traducción mal codificada. */
static int vram_wr64_verify(uint64_t addr, uint64_t val, const char *que)
{
    uint64_t leido;

    vram_wr64(addr, val);
    leido = vram_rd64(addr);
    if (leido != val) {
        lx_printk("nouveau-lx: BAR1 — %s @0x%llx no se quedó: escribí "
                  "0x%016llx y lee 0x%016llx (¿PRAMIN no escribe esa VRAM?)\n",
                  que, (unsigned long long)addr, (unsigned long long)val,
                  (unsigned long long)leido);
        return -1;
    }
    return 0;
}

/* Dónde ata Turing+ el bloque de instancia de BAR1. **No es el 0x001704 de
 * gf100**: `tu102_bar_bar1_init` (`nvkm/subdev/bar/tu102.c`) escribe
 * `0xb80f40 = 0x80000000 | (inst >> 12)`, con BAR2 en 0xb80f48 y el estado en
 * 0xb80f50 (bits 1:0 para BAR1). Leer el registro viejo devuelve el centinela de
 * error de PRI, que es lo que pasó el 2026-08-01. */
#define BAR1_INST_REG      0xb80f40u
#define BAR2_INST_REG      0xb80f48u
#define BAR_INST_STATUS    0xb80f50u
#define BAR_INST_ENABLE    0x80000000u

int gsp_bar1_inst_probe(struct gsp_bar1 *b, uint64_t *pdb_out, uint64_t *limit_out)
{
    uint32_t reg, st;
    uint64_t inst, pdb, limit;
    unsigned target;

    if (!b || !gsp_pramin_alive()) {
        return -1;
    }
    reg = gsp_mmio_rd32(BAR1_INST_REG);
    st = gsp_mmio_rd32(BAR_INST_STATUS);
    /* Comprobar el error de PRI ANTES de interpretar nada: `0xbadf5040` tiene el
     * bit 31 puesto, así que leído como «atado» miente con toda la cara. */
    if (gsp_mmio_pri_error(reg)) {
        lx_printk("nouveau-lx: BAR1 — 0x%06x lee 0x%08x (error de PRI): ese "
                  "registro no existe en este chip\n", BAR1_INST_REG, reg);
        return -1;
    }
    if (!(reg & BAR_INST_ENABLE)) {
        lx_printk("nouveau-lx: BAR1 — bloque de instancia SIN ATAR (0x%06x="
                  "0x%08x): la apertura no puede traducir nada\n",
                  BAR1_INST_REG, reg);
        return -1;
    }
    inst = (uint64_t)(reg & 0x0fffffffu) << 12;

    /* Y dentro del bloque, lo que de verdad manda: `gf100_vmm_join_`
     * (`nvkm/subdev/mmu/vmmgf100.c:342-362`) escribe en +0x200 el PDB con el
     * destino en los bits bajos (0=VRAM, 2=sysmem coherente, 3=no coherente) y
     * en +0x208 el LÍMITE de VA (`vmm->limit - 1`). Esas dos son justo las dos
     * incógnitas que llevamos tres ciclos sin poder separar: qué raíz recorre
     * BAR1 —que no tiene por qué ser el `bar1PdeBase` que RM nos contó— y hasta
     * dónde llega su espacio. */
    pdb = vram_rd64(inst + 0x200u);
    limit = vram_rd64(inst + 0x208u);
    target = (unsigned)(pdb & 3u);

    lx_printk("nouveau-lx: BAR1 inst=0x%llx (0x%06x=0x%08x, estado 0x%08x) "
              "PDB=0x%llx target=%u limite=0x%llx (%llu MiB)\n",
              (unsigned long long)inst, BAR1_INST_REG, reg, st,
              (unsigned long long)(pdb & ~0xfffull), target,
              (unsigned long long)limit,
              (unsigned long long)((limit + 1ull) >> 20));
    if ((pdb & ~0xfffull) != b->pd3) {
        lx_printk("nouveau-lx: BAR1 — ¡la raíz que recorre BAR1 (0x%llx) NO es la "
                  "que dio RM en bar1PdeBase (0x%llx)! Parchear la de RM no puede "
                  "servir de nada\n",
                  (unsigned long long)(pdb & ~0xfffull),
                  (unsigned long long)b->pd3);
    }
    if (pdb_out) {
        *pdb_out = pdb & ~0xfffull;
    }
    if (limit_out) {
        *limit_out = limit;
    }
    return 0;
}

static void bar1_invalidate(const struct gsp_bar1 *b)
{
    unsigned waited = 0;

    /* PDB de BAR1: `addr >> 8`, como `tu102_vmm_flush`. Con vaspace propio la
     * raíz vive en SYSMEM, y ahí nuestro `gsp_vmm_invalidate` —que sí funciona—
     * marca la apertura en los bits bajos; se hace igual por coherencia. */
    gsp_mmio_wr32(BAR1_INVAL_PDB, (uint32_t)((b->pd3 >> 8) | (b->owned ? 2u : 0u)));
    gsp_mmio_wr32(BAR1_INVAL_UPPER_PDB, (uint32_t)(b->pd3 >> 40));
    gsp_mmio_wr32(BAR1_INVAL, BAR1_INVAL_TRIGGER | BAR1_INVAL_ALL_VA |
                              BAR1_INVAL_HUB_ALL_PDB);

    for (;;) {
        uint32_t busy = gsp_mmio_rd32(BAR1_INVAL);

        if (gsp_mmio_pri_error(busy)) {
            lx_printk("nouveau-lx: BAR1 invalidate PRI error 0x%08x\n", busy);
            return;
        }
        if (!(busy & BAR1_INVAL_TRIGGER)) {
            return;
        }
        if (waited >= BAR1_INVAL_POLL_MS) {
            lx_printk("nouveau-lx: BAR1 invalidate timeout (0x%08x)\n", busy);
            return;
        }
        lx_mdelay(1);
        waited++;
    }
}

/* Escribe una palabra de 32 bits en VRAM por PRAMIN (las tablas y el bloque de
 * instancia viven ahí y no hay otra ventana). */
static void vram_wr32(uint64_t addr, uint32_t val)
{
    gsp_pramin_wr32(addr, val);
}

/* Le pide a RM que instale NUESTRA subtabla en el vaspace de BAR1.
 *
 * Es lo que hace `r535_bar_bar2_update_pde` para BAR2, y bajo GSP es el único
 * camino: con lockdown el host escribe `0xb80f40` y se relee cero, o sea que la
 * escritura se descarta (medido el 2026-08-02). RM escribe la entrada del nivel
 * cuyas entradas cubren 2^47 —PD3, `entryLevelShift = 47`— apuntando a la tabla
 * PD2 que le damos, así que a partir de ahí TODO el vaspace de la apertura pasa
 * a colgar de nuestras tablas. Eso incluye lo que RM tuviera mapeado; en esta
 * tarjeta no tiene nada (su PD0 está a inválido en las entradas que miramos), y
 * upstream hace exactamente esto mismo con BAR2.
 *
 * `entryValue` va en el formato que usa upstream tal cual: `(addr >> 4)` con la
 * apertura en los bits bajos. No se reinterpreta ni se "arregla": es el formato
 * que RM acepta en hardware real y no hay documentación para deducirlo mejor. */
int gsp_bar1_update_pde(struct gsp_bar1 *b, struct gsp_rm *rm, uint64_t pd2_phys,
                        enum gsp_vmm_target target)
{
    rpc_update_bar_pde_v15_00 req;
    uint32_t got = 0, transporte = 0;

    if (!b || !rm || !rm->ready || !rm->q) {
        return -1;
    }
    memset(&req, 0, sizeof(req));
    req.info.barType = NV_RPC_UPDATE_PDE_BAR_1;
    req.info.entryLevelShift = 47u;
    req.info.entryValue = pd2_phys
        ? ((pd2_phys >> 4) | ((uint64_t)(target == GSP_VMM_VRAM ? 1u : 2u) << 1))
        : 0ull;

    if (gsp_cmdq_call(rm->q, rm->rpc, NV_VGPU_MSG_FUNCTION_UPDATE_BAR_PDE,
                      &req, (uint32_t)sizeof(req), NULL, 0, &got, &transporte,
                      2000u) != 0) {
        lx_printk("nouveau-lx: BAR1 — UPDATE_BAR_PDE sin respuesta "
                  "(transporte=0x%x)\n", transporte);
        return -1;
    }
    lx_printk("nouveau-lx: BAR1 — RM instaló nuestra PD2 0x%llx en el vaspace de "
              "la apertura (entryValue=0x%llx shift=47)\n",
              (unsigned long long)pd2_phys,
              (unsigned long long)req.info.entryValue);
    return 0;
}

int gsp_bar1_bind(struct gsp_bar1 *b, struct gsp_vram *vram)
{
    uint64_t base, root, limite, off;
    uint32_t reg, leido;

    if (!b || !b->ready || !vram) {
        return -1;
    }
    if (b->owned) {
        return 0;
    }
    if (!b->aperture_size) {
        lx_printk("nouveau-lx: BAR1 — sin tamaño de apertura, no se puede atar\n");
        return -1;
    }
    /* El bloque de instancia va en VRAM: el registro que lo ata sólo lleva
     * `inst >> 12` y NO tiene campo de apertura, así que no cabe ponerlo en
     * sysmem (`tu102_bar_bar1_init` + `gf100_bar_oneinit_bar`, que lo reserva con
     * NVKM_MEM_TARGET_INST). Se escribe por PRAMIN, que ya está demostrado que
     * escribe VRAM (el readback de los PDE pasó en silicio). */
    b->inst_vram = gsp_vram_alloc(vram, 4096, 4096);
    if (!b->inst_vram) {
        lx_printk("nouveau-lx: BAR1 — sin VRAM para el bloque de instancia\n");
        return -1;
    }
    /* Un bloque de instancia con basura es un vaspace indefinido: a cero entero
     * antes de tocar nada. */
    for (off = 0; off < 4096u; off += 4u) {
        vram_wr32(b->inst_vram + off, 0);
    }

    if (gsp_vmm_init_bare(&b->own) != 0) {
        lx_printk("nouveau-lx: BAR1 — sin directorio propio\n");
        return -1;
    }
    root = b->own.pt[0].mem.phys;

    /* Palabra del PDB, exactamente como la arma upstream para Volta+ y Blackwell
     * (`gp100_vmm_join` → `gf100_vmm_join_`, con `gh100_vmm.join = gv100_vmm_join`):
     *
     *   base = BIT(10) VER2 | BIT(11) página grande de 64 KiB
     *        | destino en 1:0 (0 VRAM, 2 sysmem coherente, 3 no coherente)
     *        | BIT(2) VOL si el destino es sysmem
     *        | dirección del directorio TAL CUAL (está alineado a 4 KiB, así que
     *          no pisa los bits de abajo)
     *
     * Nuestro directorio vive en sysmem coherente, igual que el del canal. */
    base = (1ull << 10) | (1ull << 11) | 2ull | (1ull << 2) | root;
    limite = b->aperture_size - 1ull;

    vram_wr32(b->inst_vram + 0x200u, (uint32_t)base);
    vram_wr32(b->inst_vram + 0x204u, (uint32_t)(base >> 32));
    vram_wr32(b->inst_vram + 0x208u, (uint32_t)limite);
    vram_wr32(b->inst_vram + 0x20cu, (uint32_t)(limite >> 32));
    /* Y lo que añade `gv100_vmm_join` por encima: limpiar 0x21c y replicar el PDB
     * en la entrada 0 del array de subcontextos. Para una apertura seguramente no
     * se consulta, pero cuesta tres escrituras y es lo que hace upstream. */
    vram_wr32(b->inst_vram + 0x21cu, 0);
    vram_wr32(b->inst_vram + 0x2a0u, (uint32_t)base);
    vram_wr32(b->inst_vram + 0x2a4u, (uint32_t)(base >> 32));

    if (vram_rd64(b->inst_vram + 0x200u) != base) {
        lx_printk("nouveau-lx: BAR1 — el bloque de instancia no acepta escrituras "
                  "(0x%llx lee 0x%llx)\n", (unsigned long long)base,
                  (unsigned long long)vram_rd64(b->inst_vram + 0x200u));
        return -1;
    }

    /* Y el atado: `0xb80f40 = 0x80000000 | (inst >> 12)`, el registro de Turing+
     * (`tu102_bar_bar1_init`). Se relee para saber si además es legible: si lo es,
     * el cero que leímos antes significaba de verdad «RM no lo ató». */
    reg = BAR_INST_ENABLE | (uint32_t)(b->inst_vram >> 12);
    gsp_mmio_wr32(BAR1_INST_REG, reg);
    leido = gsp_mmio_rd32(BAR1_INST_REG);

    b->owned = 1;
    b->pd3 = root;   /* el recorrido y el invalidate pasan a mirar NUESTRA raíz */
    lx_printk("nouveau-lx: BAR1 ATADO por nosotros — inst=0x%llx (VRAM) raíz=0x%llx "
              "(sysmem) límite=0x%llx; 0x%06x escrito 0x%08x, relee 0x%08x%s\n",
              (unsigned long long)b->inst_vram, (unsigned long long)root,
              (unsigned long long)limite, BAR1_INST_REG, reg, leido,
              leido == reg ? "" : " (registro de sólo escritura)");
    return 0;
}

uint64_t gsp_bar1_map(struct gsp_bar1 *b, uint64_t phys, uint64_t bytes)
{
    struct gsp_bar1_step steps[GSP_BAR1_LEVELS];
    const struct gsp_bar1_step *punto;
    uint64_t va, off;
    uint64_t *spt;
    int n;

    if (!b || !b->ready || !b->window_va || !bytes) {
        return 0;
    }
    if (bytes > GSP_BAR1_WINDOW_BYTES || (phys & 0xfffull)) {
        lx_printk("nouveau-lx: BAR1 — %llu B en phys 0x%llx no cabe alineado en la "
                  "ventana de %llu KiB\n", (unsigned long long)bytes,
                  (unsigned long long)phys,
                  (unsigned long long)(GSP_BAR1_WINDOW_BYTES >> 10));
        return 0;
    }
    va = b->window_va;

    /* Con BAR1 atado por nosotros el vaspace es NUESTRO: se mapea con la
     * maquinaria de siempre, que ya crea los niveles que falten, y no hay que
     * escribir dentro de las estructuras de RM ni por PRAMIN. Es el camino
     * limpio; el de abajo sólo existe para el caso en que RM sí lo hubiera
     * atado. */
    if (b->owned) {
        if (gsp_vmm_map(&b->own, va, phys, bytes, GSP_VMM_VRAM) != 0) {
            lx_printk("nouveau-lx: BAR1 — no se pudo mapear 0x%llx en el vaspace "
                      "propio\n", (unsigned long long)va);
            return 0;
        }
        bar1_invalidate(b);
        b->mapped_bytes = bytes;
        lx_printk("nouveau-lx: BAR1 — ventana 0x%llx ← VRAM 0x%llx (%llu KiB) en "
                  "vaspace propio; la CPU escribe en 0x%llx\n",
                  (unsigned long long)va, (unsigned long long)phys,
                  (unsigned long long)(bytes >> 10),
                  (unsigned long long)(b->aperture_phys + va));
        return va;
    }

    n = gsp_bar1_walk(b, va, steps, GSP_BAR1_LEVELS);
    if (n <= 0) {
        return 0;
    }
    punto = &steps[n - 1];

    /* El punto de enganche es el ÚLTIMO nivel que leímos, y tiene que estar
     * inválido. Si está válido hay dos casos y ninguno es nuestro: o RM tiene
     * algo mapeado en esta ventana (y escribir ahí cuelga la tarjeta), o ya
     * mapeamos nosotros y el llamante debería haber hecho `unmap`. */
    if (punto->aperture != 0u) {
        lx_printk("nouveau-lx: BAR1 — %s[%u] ya es válido (0x%016llx): la ventana "
                  "0x%llx está ocupada, no se pisa\n",
                  punto->level == 0u ? "SPT" : (punto->level == 1u ? "PD0" : "PD1"),
                  punto->index, (unsigned long long)punto->entry,
                  (unsigned long long)va);
        return 0;
    }
    /* Y tiene que ser un nivel que sepamos rellenar: PD1 (nos toca crear PD0 y
     * SPT) o PD0 (sólo la hoja). Más arriba significaría que RM no ha construido
     * ni PD2, y entonces esto no es la cadena que creemos. */
    if (punto->level != 1u && punto->level != 2u) {
        lx_printk("nouveau-lx: BAR1 — la cadena de RM se corta en el nivel %u: no "
                  "es la forma esperada, no se toca\n", punto->level);
        return 0;
    }

    /* Hoja propia, en sysmem: es lo único que la CPU puede escribir sin ventana a
     * VRAM, y RM acepta SYSMEM_COH en un PDE (lo mismo que ya hacemos con el
     * directorio del vaspace del canal). */
    if (!b->spt.va && gsp_dma_alloc(&b->spt, 4096, "SPT de BAR1") != 0) {
        return 0;
    }
    memset(b->spt.va, 0, 4096);
    spt = (uint64_t *)b->spt.va;
    for (off = 0; off < bytes; off += 4096ull) {
        /* Índice dentro de la hoja: bits 20:12 de la VA. */
        uint32_t i = (uint32_t)(((va + off) >> 12) & 0x1ffu);

        spt[i] = gsp_vmm_pte_encode(phys + off, GSP_VMM_VRAM, 0u);
    }
    __asm__ __volatile__("mfence" ::: "memory");

    if (punto->level == 2u) {
        /* RM no tenía PD0 para estos 512 MiB: lo creamos y lo enlazamos. */
        uint64_t *pd0;
        uint32_t i0 = (uint32_t)((va >> 21) & 0xffu);

        if (!b->pd0.va && gsp_dma_alloc(&b->pd0, 4096, "PD0 de BAR1") != 0) {
            return 0;
        }
        memset(b->pd0.va, 0, 4096);
        pd0 = (uint64_t *)b->pd0.va;
        /* Entradas de 16 B: la mitad BAJA es la de páginas grandes (a cero, no
         * usamos 2 MiB) y la ALTA la de 4 KiB. */
        pd0[(unsigned long)i0 * 2u] = 0;
        pd0[(unsigned long)i0 * 2u + 1u] =
            gsp_vmm_pde_encode(b->spt.phys, GSP_VMM_SYSMEM);
        __asm__ __volatile__("mfence" ::: "memory");
        /* Y el enlace en la tabla de RM, que está en VRAM. */
        if (vram_wr64_verify(punto->table + (uint64_t)punto->index * 8u,
                             gsp_vmm_pde_encode(b->pd0.phys, GSP_VMM_SYSMEM),
                             "PDE de PD1") != 0) {
            return 0;
        }
    } else {
        /* RM ya tenía PD0 (es su tabla, en VRAM): sólo colgamos la hoja, en la
         * mitad ALTA de la entrada doble. */
        if (vram_wr64_verify(punto->table + (uint64_t)punto->index * 16u + 8u,
                             gsp_vmm_pde_encode(b->spt.phys, GSP_VMM_SYSMEM),
                             "PDE de PD0") != 0) {
            return 0;
        }
    }

    bar1_invalidate(b);
    b->mapped_bytes = bytes;
    lx_printk("nouveau-lx: BAR1 — ventana 0x%llx ← VRAM 0x%llx (%llu KiB) por "
              "%s[%u]; la CPU escribe en 0x%llx\n",
              (unsigned long long)va, (unsigned long long)phys,
              (unsigned long long)(bytes >> 10),
              punto->level == 2u ? "PD1" : "PD0", punto->index,
              (unsigned long long)(b->aperture_phys + va));
    return va;
}

void gsp_bar1_unmap(struct gsp_bar1 *b)
{
    struct gsp_bar1_step steps[GSP_BAR1_LEVELS];
    int n;

    if (!b || !b->ready || !b->mapped_bytes) {
        return;
    }
    if (b->owned) {
        /* Vaspace propio: basta invalidar las PTEs. Las tablas se quedan para el
         * siguiente mapeo — el mapeo reescribe las entradas que use. */
        b->mapped_bytes = 0;
        bar1_invalidate(b);
        return;
    }
    /* Desenlazar por donde se enlazó. El recorrido se para en la entrada que
     * apunta a NUESTRA tabla, no una más abajo: nuestras tablas viven en sysmem y
     * PRAMIN no las alcanza, así que el walk corta ahí con aperture=SYS_COH. Ese
     * corte ES el punto de enganche. (Buscarlo un nivel más allá dejaba el PDE
     * puesto y el siguiente mapeo se negaba «porque está ocupado» — por sí mismo.) */
    n = gsp_bar1_walk(b, b->window_va, steps, GSP_BAR1_LEVELS);
    if (n >= 1) {
        const struct gsp_bar1_step *p = &steps[n - 1];

        if (p->aperture == 2u || p->aperture == 3u) {
            if (p->level == 2u) {
                vram_wr64(p->table + (uint64_t)p->index * 8u, 0);
            } else if (p->level == 1u) {
                vram_wr64(p->table + (uint64_t)p->index * 16u + 8u, 0);
            }
        }
    }
    if (b->spt.va) {
        memset(b->spt.va, 0, 4096);
    }
    __asm__ __volatile__("mfence" ::: "memory");
    bar1_invalidate(b);
    b->mapped_bytes = 0;
}

int gsp_bar1_selftest(struct gsp_bar1 *b, struct gsp_ce *ce, uint64_t vram_phys,
                      uint64_t vram_va, uint64_t scratch_va, void *scratch_cpu)
{
    const uint32_t bytes = 4096u;
    volatile uint32_t *win;
    uint32_t *leido;
    uint64_t off;
    unsigned i;
    int ok = 0;

    if (!b || !b->ready || !ce || !vram_phys || !vram_va || !scratch_cpu) {
        return -1;
    }
    off = gsp_bar1_map(b, vram_phys, bytes);
    if (!off) {
        return -1;
    }
    win = (volatile uint32_t *)lx_map_wc(
        (unsigned long)(b->aperture_phys + off), (unsigned long)bytes);
    if (!win) {
        lx_printk("nouveau-lx: BAR1 selftest — la apertura no se pudo mapear en la "
                  "CPU (0x%llx)\n", (unsigned long long)(b->aperture_phys + off));
        gsp_bar1_unmap(b);
        return -1;
    }

    /* ANTES de escribir: qué se lee por la ventana. Esto es lo que separa las dos
     * averías posibles cuando el patrón no aparece en VRAM (medido el
     * 2026-08-01: el CE releía el patrón del selftest del CE, o sea que nuestra
     * escritura no llegó):
     *
     *   - 0xffffffff → la CPU no está hablando con el dispositivo: el mapeo de la
     *     apertura en el kernel, o la dirección, están mal.
     *   - 0x00000000 u otra cosa → la apertura contesta, así que el problema es
     *     la traducción de la MMU de BAR1 (PTE/PDE o el invalidate).
     *   - el patrón del CE (0xa5c3e1b4…) → ya estaríamos leyendo esa VRAM y el
     *     problema sería sólo de escritura (posted write perdido, WC sin vaciar). */
    lx_printk("nouveau-lx: BAR1 selftest — la ventana lee 0x%08x 0x%08x antes de "
              "escribir\n", win[0], win[1]);

    /* Patrón por la apertura. */
    for (i = 0; i < bytes / 4u; i++) {
        win[i] = 0x5ea1b0a5u ^ (uint32_t)i;
    }
    __asm__ __volatile__("sfence" ::: "memory");
    /* En PCIe una escritura es posted: sin una LECTURA de vuelta puede no haber
     * salido del root complex. Y con memoria WC hace falta además vaciar el
     * buffer de combinación, que es lo que hace la propia lectura. Upstream hace
     * dos `nvkm_bar_flush` seguidos por esto mismo (`gf100_bar_bar1_wait`, con su
     * «NFI why it's twice»). */
    (void)win[0];
    (void)win[0];
    lx_printk("nouveau-lx: BAR1 selftest — la ventana lee 0x%08x 0x%08x tras "
              "escribir (esperado 0x%08x 0x%08x)\n", win[0], win[1],
              0x5ea1b0a5u, 0x5ea1b0a5u ^ 1u);

    /* Y la prueba de verdad: que el CE, que lee VRAM de otra manera, vea el
     * patrón. El scratch se borra antes para que un readback que no hiciera nada
     * no pueda dar un falso verde. */
    memset(scratch_cpu, 0, bytes);
    __asm__ __volatile__("mfence" ::: "memory");
    if (gsp_ce_copy_sync(ce, scratch_va, vram_va, bytes, GSP_CE_WAIT_MS) != 0) {
        lx_printk("nouveau-lx: BAR1 selftest — el CE no pudo releer la VRAM\n");
        gsp_bar1_unmap(b);
        return -1;
    }
    leido = (uint32_t *)scratch_cpu;
    for (i = 0; i < bytes / 4u; i++) {
        if (leido[i] != (0x5ea1b0a5u ^ (uint32_t)i)) {
            lx_printk("nouveau-lx: BAR1 selftest — dword %u vale 0x%08x, esperaba "
                      "0x%08x: la apertura NO escribe en esa VRAM\n",
                      i, leido[i], 0x5ea1b0a5u ^ (uint32_t)i);
            break;
        }
    }
    if (i == bytes / 4u) {
        lx_printk("nouveau-lx: BAR1 selftest OK — %u B escritos por la apertura y "
                  "releídos por el CE desde VRAM (la CPU ya escribe VRAM sin "
                  "canal)\n", bytes);
        ok = 1;
    }
    gsp_bar1_unmap(b);
    return ok ? 0 : -1;
}

void gsp_bar1_dump(const struct gsp_bar1 *b, uint64_t bar1_va)
{
    struct gsp_bar1_step steps[GSP_BAR1_LEVELS];
    int n, i;

    if (!b || !b->ready) {
        return;
    }
    lx_printk("nouveau-lx: BAR1 apertura=0x%llx (%llu MiB según RM) raíz PD3 en "
              "VRAM 0x%llx\n",
              (unsigned long long)b->aperture_phys,
              (unsigned long long)(b->aperture_size >> 20),
              (unsigned long long)b->pd3);
    (void)gsp_bar1_inst_probe(b, NULL, NULL);

    n = gsp_bar1_walk(b, bar1_va, steps, GSP_BAR1_LEVELS);
    if (n <= 0) {
        lx_printk("nouveau-lx: BAR1 — no se pudo recorrer la tabla (PRAMIN=%d)\n",
                  gsp_pramin_alive());
        return;
    }
    for (i = 0; i < n; i++) {
        const struct gsp_bar1_step *s = &steps[i];

        lx_printk("nouveau-lx: BAR1 va=0x%llx %s[%u] @0x%llx = 0x%016llx "
                  "ap=%u (%s) → 0x%llx\n",
                  (unsigned long long)bar1_va, g_lvl[s->level].name, s->index,
                  (unsigned long long)s->table, (unsigned long long)s->entry,
                  s->aperture,
                  s->level == 0u ? "PTE, VRAM=0" : aperture_name(s->aperture),
                  (unsigned long long)s->next);
    }
    if ((unsigned)n < GSP_BAR1_LEVELS) {
        lx_printk("nouveau-lx: BAR1 — el recorrido se paró en %s: por debajo no "
                  "hay tablas que la CPU pueda leer hoy\n",
                  g_lvl[steps[n - 1].level].name);
    }
}
