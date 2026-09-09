/* G3: GSP bring-up gb205 (Blackwell) + ga10x (Ampere, RTX 3060) —
 * firmware + ACR ola2 + poll MMIO. La ruta nvkm real usa ga102_gsp_new, que
 * cubre Ampere GA10x de forma madura en nouveau 6.6; GB205 es más reciente. */
#include "acr_lx.h"
#include "fmc_lx.h"
#include "gsp_fini.h"
#include "gsp_fw.h"
#include "gsp_mmio.h"
#include "fsp_lx.h"
#include "gsp_cmdq.h"
#include "gsp_libos.h"
#include "gsp_rm.h"
#include "gsp_rm_obj.h"
#include "gsp_rpc.h"
#include "gsp_vmm.h"
#include "gsp_chan.h"
#include "gsp_ce.h"
#include "gsp_compute.h"
#include "gsp_grctx.h"
#include "gsp_top.h"
#include "gsp_vram.h"
#include "gsp_buf.h"
#include "gsp_bar1.h"
#include "gsp_wpr.h"
#include "gsp_fwsec.h"
#include "gsp_chip.h"
#include "gsp_dma.h"
#include "falcon_lx.h"
#include "gsp_cpu_seq.h"
#include "nvfw_lx.h"
#include "lx_emul.h"

#define NV_PMC_BOOT_0_OFF 0x0000u
#define GSP_POLL_MS         2000u
/* Los 50 + 2000 ms de `tu102_devinit_wait`, que es quien decide cuánto puede tardar
 * el firmware de la GPU en acabar su devinit. */
#define GSP_GFW_WAIT_MS     2050u

/* Qué juego de firmware pedir. Ada aún no tiene blobs empaquetados (ad10x); cae
 * en el juego Blackwell, que es el que trae el rootfs. */
static enum gsp_fw_chip gsp_fw_chip_of(enum nv_family f)
{
    return f == NV_FAM_AMPERE ? GSP_FW_CHIP_AMPERE : GSP_FW_CHIP_BLACKWELL;
}

/* Ola 3: construye el grafo de objetos nvkm real con BAR0 (nvkm_bringup_lx.c).
 * Best-effort: no altera la secuencia soft si falla. */
int lx_nvkm_build_gsp(void *bar0);

/* Definidas más abajo en este mismo fichero; `lx_nouveau_gsp_fini` las usa. */
int lx_nouveau_gsp_ready(void);
const char *lx_nouveau_gsp_status(void);

enum gsp_phase {
    GSP_NONE = 0,
    GSP_BAR0,
    GSP_FW_LOADING,
    GSP_FW_READY,
    GSP_FW_STAGED,
    GSP_RM_RADIX3,
    GSP_FMC_PARSE,
    GSP_WPR_META,
    GSP_LIBOS_ARGS,
    GSP_COT_READY,
    GSP_COT_SENT,
    GSP_FMC_READY,
    GSP_ACR_LOAD,
    GSP_ACR_AHESASC,
    GSP_ACR_ASB,
    GSP_KICK,
    GSP_POLL,
    GSP_BOOTED,
    GSP_FAILED,
    GSP_RM_READY,
    GSP_RM_OBJECTS,
    GSP_RM_VMM,     /* + espacio de direcciones con VRAM y sysmem mapeadas */
    GSP_RM_CHAN,    /* canal GPFIFO + USERD */
    GSP_RM_CE,      /* objeto CE + pushbuffer preparado */
    GSP_RM_COMPUTE, /* objeto compute + QMD (G4f) */
    GSP_BOOTED_SOFT,
    GSP_GONE,
    GSP_FINI,       /* apagado por gsp_fini(): sin DMA, no se puede volver atrás */
};

static enum gsp_phase g_phase = GSP_NONE;
static struct gsp_rm_fw g_rm;   /* imagen GSP-RM + radix3, viva hasta el boot */
static struct gsp_wpr g_wpr;    /* bootloader + GspFwWprMeta (FMC y Ampere) */
static struct gsp_libos g_libos;    /* colas, logs, RMARGS y boot params */
static struct fmc_staged g_fmc;     /* imagen FMC + cadena de firma en sysmem */
static struct gsp_rpc g_rpc;        /* anillo de mensajes de GSP-RM */
static struct gsp_cmdq g_cmdq;      /* cola de comandos hacia GSP-RM */
static struct gsp_rm g_rm_obj;      /* cliente/device/subdevice de RM */
static struct gsp_static_info g_static;  /* VRAM utilizable y regalos de RM */
static struct gsp_vram g_vram_pool;      /* reparto de VRAM sobre esas regiones */
static struct gsp_vmm g_vmm;             /* vaspace de RM + tablas de páginas */
static struct gsp_dma_buf g_scratch;     /* página de sysmem visible por la GPU */
/* Rebote de las subidas a VRAM (G6). Aparte del scratch de G4d y mucho mayor:
 * la subida cuesta un LAUNCH_DMA + una espera de semáforo por búfer, así que su
 * tamaño es el que decide si un modelo tarda segundos o minutos en entrar. Va
 * cacheado porque aquí la CPU escribe megabytes y el dispositivo sólo lee. */
static struct gsp_dma_buf g_bounce;
static struct gsp_chan g_chan;           /* canal GPFIFO del CE, motor COPY0 (G4e) */
/* Segundo canal, atado a GR0. No es duplicación: RM no acepta un objeto de
 * compute sobre un canal de copia (INVALID_CLASS con la clase correcta, HW
 * 2026-07-28), así que G4f/G5 necesitan el suyo. El CE se queda con el de COPY0
 * porque es quien mueve el SASS a VRAM. */
static struct gsp_chan g_chan_gr;        /* canal GPFIFO del compute, motor GR0 */
static struct gsp_ce g_ce;               /* motor de copia CE (G4e) */
static struct gsp_compute g_compute;     /* compute + QMD (G4f) */
static struct gsp_buf g_buf;             /* buffers de usuario en VRAM (G6) */
static struct gsp_bar1 g_bar1;           /* apertura de CPU a VRAM: sólo mirar */
static struct gsp_grctx g_grctx;          /* contexto del canal de GR (G4f) */
static int g_ce_verified;                /* el CE movió bytes de verdad (G4e) */
static uint64_t g_vram_block;            /* bloque de VRAM mapeado en G4d */
static struct lx_pci_dev *g_pdev;   /* para leer BARs y BDF del espacio de config */
static uint64_t g_vram_bytes;

void lx_nouveau_set_boot0(unsigned boot0, unsigned device_id)
{
    gsp_nv_family_set((uint32_t)boot0, (uint16_t)device_id);
    /* NO retroceder la fase. `nvidia_probe::init()` corre en el kernel DESPUÉS
     * del bring-up (main.rs: lxdde::init → gpu::init → nvidia_probe::init) y
     * volvía a poner GSP_BAR0 encima de un `rm_ready` ya conseguido: el arranque
     * del 2026-07-25 llegó a `GSP-RM listo` y aun así el log decía `GSP=bar0`,
     * `gsp_ready()` daba falso y el compute se iba a la CPU sin avisar. Esta
     * función solo aporta boot0/device_id; la fase la manda el bring-up. */
    if (gsp_nv_family_boot0() != 0 && g_phase == GSP_NONE) {
        g_phase = GSP_BAR0;
    }
}

/* VRAM heurística por SKU (en HW real la da nvkm_ram del fb). El RTX 3060 tiene
 * 12 GiB (GA106) o 8 GiB (3060 Ti/GA104); la 3050 Mobile (GA107) = 4 GiB. */
static uint64_t vram_for_device(uint16_t dev_id)
{
    enum nv_family fam = gsp_nv_family_of(gsp_nv_family_boot0(), dev_id);
    if (dev_id == 0x2f18u) {
        return 12ull * 1024ull * 1024ull * 1024ull;   /* 5070 Ti Mobile */
    }
    if (dev_id == GA107_DEVICE_ID) {
        return 4ull * 1024ull * 1024ull * 1024ull;    /* 3050 Mobile */
    }
    if (fam == NV_FAM_AMPERE) {
        if (dev_id == 0x2486u || dev_id == 0x2489u) /* 3060 Ti (GA104) */
            return 8ull * 1024ull * 1024ull * 1024ull;
        return 12ull * 1024ull * 1024ull * 1024ull;   /* 3060 (GA106) 12 GiB */
    }
    return 8ull * 1024ull * 1024ull * 1024ull;
}

/* Mapa de BARs de una GPU NVIDIA en el espacio de configuración:
 *
 *   0x10  BAR0  registros, 16 MiB, **32 bits**
 *   0x14  BAR1  apertura de FB, 64 bits prefetchable  (ocupa 0x14 y 0x18)
 *   0x1c  BAR3  instancia,      64 bits prefetchable  (ocupa 0x1c y 0x20)
 *   0x24  BAR5  E/S
 *
 * Que BAR0 sea de 32 bits es justo lo que se me pasó la primera vez: al leerla
 * como si fuera de 64 me llevaba el dword bajo de BAR1 como parte alta, y las
 * otras dos salían corridas media BAR. Con esas tres direcciones inventadas
 * dentro del SET_SYSTEM_INFO, GSP-RM tumbó la GPU (2026-07-25). */
static uint64_t pci_bar32(int off)
{
    return (uint64_t)(lx_pci_read_config(g_pdev, off, 4) & ~0xfu);
}

static uint64_t pci_bar64(int off)
{
    uint32_t lo = lx_pci_read_config(g_pdev, off, 4);
    uint32_t hi = lx_pci_read_config(g_pdev, off + 4, 4);

    return ((uint64_t)hi << 32) | (uint64_t)(lo & ~0xfu);
}

/* Lo que `r570_gsp_set_system_info` saca del `pci_dev`; aquí sale del espacio
 * de configuración, que es lo que tenemos. BAR0 = registros, BAR1 = FB,
 * BAR3 = instancia (los índices de `NVKM_BAR{0_PRI,1_FB,2_INST}`). */
static void collect_sysinfo(struct gsp_sysinfo *si)
{
    uint32_t ids = lx_pci_read_config(g_pdev, 0x00, 4);
    uint32_t sub = lx_pci_read_config(g_pdev, 0x2c, 4);

    si->bar0_phys = pci_bar32(0x10);
    si->bar1_phys = pci_bar64(0x14);
    si->bar3_phys = pci_bar64(0x1c);
    si->bdf = lx_pci_bdf(g_pdev);
    /* TASK_SIZE de x86-64: el límite de direcciones de usuario que asume RM. */
    si->max_user_va = 0x00007ffffffff000ull;
    /* Espejo del espacio de configuración dentro de BAR0 en Turing+
     * (`tu102_pci`: cfg.addr = 0x88000, cfg.size = 0x1000). */
    si->cfg_mirror_base = 0x88000u;
    si->cfg_mirror_size = 0x1000u;
    si->vendor_id = (uint16_t)ids;
    si->device_id = (uint16_t)(ids >> 16);
    si->subvendor_id = (uint16_t)sub;
    si->subdevice_id = (uint16_t)(sub >> 16);
    si->revision_id = (uint8_t)lx_pci_read_config(g_pdev, 0x08, 1);
}

/* Una apertura tiene que estar asignada y alineada a página; lo contrario
 * significa que la hemos leído del sitio equivocado. */
static int sysinfo_plausible(const struct gsp_sysinfo *si)
{
    const uint64_t bars[3] = { si->bar0_phys, si->bar1_phys, si->bar3_phys };
    unsigned i;

    for (i = 0; i < 3u; i++) {
        if (bars[i] == 0 || (bars[i] & 0xfffu)) {
            lx_printk("nouveau-lx: BAR%u = 0x%llx no es una apertura válida\n",
                      i == 2u ? 3u : i, (unsigned long long)bars[i]);
            return 0;
        }
    }
    if (si->bar1_phys == si->bar3_phys) {
        lx_printk("nouveau-lx: BAR1 y BAR3 coinciden (0x%llx)\n",
                  (unsigned long long)si->bar1_phys);
        return 0;
    }
    return 1;
}

/* SET_SYSTEM_INFO / SET_REGISTRY en la cmdq ANTES de arrancar el GSP
 * (`r535_gsp_oneinit`). Sin ellas GSP-RM se inicializa a ciegas. */
static void enqueue_boot_rpcs(void)
{
    if (gsp_cmdq_init(&g_libos, &g_cmdq) == 0) {
        struct gsp_sysinfo si;
        collect_sysinfo(&si);
        if (!sysinfo_plausible(&si)) {
            lx_printk("nouveau-lx: BARs no creíbles — no se manda SET_SYSTEM_INFO\n");
        } else if (gsp_cmdq_set_system_info(&g_cmdq, &si) != 0 ||
            gsp_cmdq_set_registry(&g_cmdq) != 0) {
            lx_printk("nouveau-lx: no se pudieron encolar SET_SYSTEM_INFO/SET_REGISTRY\n");
        }
    } else {
        lx_printk("nouveau-lx: cmdq no inicializable — GSP-RM arrancará a ciegas\n");
    }
}

/* Ruta Blackwell: el GSP lo arranca el FSP con la imagen GSP-FMC, no el ACR de
 * SEC2. Validamos el ELF firmado, leemos el estado del FSP y construimos el WPR
 * meta (`gsp_wpr.c`) sobre la radix3 de `gsp_rm.c`. Para enviar el COT falta el
 * último escalón: los libos boot args. Ver fmc_lx.c. */
static int run_fmc_blackwell(void)
{
    const struct gsp_fw_blob *fmc = gsp_fw_get(GSP_FW_FMC);
    struct fmc_image img;

    /* Aquí hubo un rato una puerta que exigía el GFW boot de la isla GC6
     * (`0x118234`) antes de mandar el COT, y **estaba mal**: en gb20x ese registro
     * no es el indicador —NVIDIA no publica siquiera `dev_gc6_island.h` para
     * gb202— y leerlo 0 es lo normal, así que bloqueaba una tarjeta sana y el
     * arranque acababa en `booted_soft` (2026-07-29).
     *
     * El indicador de este chip es `NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE`
     * (0x00ad00bc, SUCCESS = 0xff) y **ya se comprueba donde toca**:
     * `fsp_ready_to_send()` no manda el COT sin él. O sea que la puerta correcta
     * llevaba puesta desde el principio y lo que sobraba era la nueva. */

    g_phase = GSP_FMC_PARSE;
    if (!fmc || !fmc->data) {
        lx_printk("nouveau-lx: FMC sin blob cargado — sigue kick/poll\n");
        return -1;
    }
    if (fmc_lx_parse(fmc->data, fmc->len, &img) != 0) {
        return -1;
    }
    (void)fmc_lx_verify_sizes(&img);
    fmc_lx_fsp_probe();
    g_phase = GSP_FMC_READY;

    /* Paso 4: el descriptor que el FMC leerá para montar WPR2. Necesita la
     * radix3 ya construida; sin ella no hay nada que describir. */
    if (!g_rm.ready) {
        lx_printk("nouveau-lx: WPR meta sin imagen GSP-RM — no se construye\n");
        return -1;
    }
    g_phase = GSP_WPR_META;
    if (gsp_wpr_prepare(&g_rm, &g_wpr) != 0) {
        lx_printk("nouveau-lx: WPR meta no preparado — sigue soft\n");
        g_phase = GSP_FMC_READY;
        return -1;
    }

    /* Paso 5: colas compartidas, búferes de log, RMARGS y el GSP_FMC_BOOT_PARAMS
     * que enlaza el WPR meta con los boot args de libos. */
    g_phase = GSP_LIBOS_ARGS;
    if (gsp_libos_prepare(&g_wpr, &g_libos) != 0) {
        lx_printk("nouveau-lx: libos boot args no preparados — sigue soft\n");
        g_phase = GSP_WPR_META;
        return -1;
    }

    /* La imagen del FMC y su cadena de firma, donde el FSP puede leerlas. */
    if (fmc_lx_stage(&img, &g_fmc) != 0) {
        lx_printk("nouveau-lx: FMC sin stagear — sigue soft\n");
        return -1;
    }

    /* Las dos RPCs que GSP-RM consume durante su init tienen que estar en la
     * cmdq ANTES de arrancar el GSP (`r535_gsp_oneinit`). */
    enqueue_boot_rpcs();

    /* Todo lo que el COT referencia está construido y verificado en memoria. */
    g_phase = GSP_COT_READY;
    lx_printk("nouveau-lx: COT listo — enviando al FSP\n");

    /* Paso 6: la primera escritura MMIO del port. `fsp_lx_boot_gsp_fmc` no manda
     * nada si el FSP no está como debe, y todas sus esperas están acotadas. */
    g_phase = GSP_COT_SENT;
    if (fsp_lx_boot_gsp_fmc(&g_fmc, &g_libos, &g_wpr) != 0) {
        return -1;
    }
    return 0;
}

/* ACR ola 2 (Ampere): carga los ucode de SEC2 y arranca AHESASC + ASB. Cada
 * fallo es soft — la secuencia sigue con kick/poll. */
static void run_acr_sec2(void)
{
    g_phase = GSP_ACR_LOAD;
    if (acr_lx_load() != 0) {
        lx_printk("nouveau-lx: ACR firmware no cargado — sigue kick/poll\n");
        return;
    }
    g_phase = GSP_ACR_AHESASC;
    if (acr_lx_boot_ahesasc() != 0) {
        lx_printk("nouveau-lx: ACR AHESASC soft-fail — sigue kick/poll\n");
        return;
    }
    g_phase = GSP_ACR_ASB;
    if (acr_lx_boot_asb() != 0) {
        lx_printk("nouveau-lx: ACR ASB soft-fail — sigue kick/poll\n");
    }
}

#define NV_PGSP_FALCON_MBOX0  0x00110040u
#define NV_PGSP_FALCON_MBOX1  0x00110044u
#define NV_PGSP_FALCON_OS     0x00110080u
#define NV_PRISCV_CPUCTL      0x00111388u
#define CPUCTL_ACTIVE_STAT    (1u << 7)

/* Booter_load en SEC2 (`tu102_gsp_booter_load`): mailbox = física del WPR meta.
 * Antes, la física del array libos en el mailbox del falcon GSP. */
static int run_ampere_booter(void)
{
    const struct gsp_fw_blob *blob = gsp_fw_get(GSP_FW_BOOTER_LOAD);
    struct gsp_dma_buf dma;
    struct acr_fw_blob wrap;
    uint32_t m0, m1;
    uint32_t cpuctl;
    unsigned sec2_base = LX_FLCN_SEC2_BASE;

    if (!blob || !blob->valid || !blob->data || !blob->len) {
        lx_printk("nouveau-lx: Ampere sin blob booter_load\n");
        return -1;
    }
    if (!g_wpr.ready || !g_wpr.meta_phys || !g_libos.ready) {
        return -1;
    }
    {
        const struct nvfw_bin_hdr *bhdr = (const struct nvfw_bin_hdr *)blob->data;

        if (blob->len < sizeof(*bhdr) || bhdr->bin_magic != NVFW_BIN_MAGIC ||
            bhdr->data_offset + bhdr->data_size > blob->len) {
            lx_printk("nouveau-lx: Ampere booter_load cabecera inválida\n");
            return -1;
        }
        if (gsp_dma_alloc_copy(&dma, blob->data + bhdr->data_offset,
                               bhdr->data_size, "booter_load") != 0) {
            return -1;
        }
    }

    if (falcon_lx_gsp_reset_riscv(LX_FLCN_GSP_BASE) != 0) {
        lx_printk("nouveau-lx: Ampere reset GSP RISC-V falló\n");
        return -1;
    }

    gsp_mmio_wr32(NV_PGSP_FALCON_MBOX0, (uint32_t)g_libos.libos.phys);
    gsp_mmio_wr32(NV_PGSP_FALCON_MBOX1, (uint32_t)(g_libos.libos.phys >> 32));

    wrap.path = blob->path;
    wrap.data = blob->data;
    wrap.len = blob->len;
    wrap.payload_len = dma.size;
    wrap.dma_handle = dma.phys;
    wrap.dma_cpu = dma.va;
    wrap.valid = 1;

    m0 = (uint32_t)g_wpr.meta_phys;
    m1 = (uint32_t)(g_wpr.meta_phys >> 32);
    /* ga102_sec2_new fuerza 0x840000: el campo addr de PTOP no refleja la
     * ventana PRI actual de SEC2 en Ampere. No usar gsp_top_falcon_base aquí. */
    sec2_base = LX_FLCN_SEC2_BASE;
    lx_printk("nouveau-lx: Ampere booter_load SEC2 base=0x%x WPR meta @0x%llx "
              "libos @0x%llx fuse@0x824148=0x%08x\n",
              sec2_base,
              (unsigned long long)g_wpr.meta_phys,
              (unsigned long long)g_libos.libos.phys,
              falcon_lx_read_fuse(0x0001u, 3u));

    if (falcon_lx_sec2_prepare(sec2_base) != 0) {
        gsp_dma_free(&dma);
        return -1;
    }

    if (falcon_lx_hsfw_boot_mbox(sec2_base, &wrap, "booter_load", m0, m1, 1) !=
        0) {
        gsp_dma_free(&dma);
        return -1;
    }
    gsp_dma_free(&dma);

    /* Linux `r535_gsp_init`: publicar app_version y pasar a poll RPC (no abortar
     * aquí si cpuctl bit7=0; CORE_RESUME puede llegar por GSP_RUN_CPU_SEQUENCER). */
    gsp_mmio_wr32(NV_PGSP_FALCON_OS, g_wpr.boot.app_version);
    cpuctl = gsp_mmio_rd32(NV_PRISCV_CPUCTL);
    {
        uint32_t sec2_m0 = gsp_mmio_rd32(sec2_base + 0x040u);
        uint32_t sec2_m1 = gsp_mmio_rd32(sec2_base + 0x044u);
        uint32_t gsp_m0 = gsp_mmio_rd32(NV_PGSP_FALCON_MBOX0);
        uint32_t gsp_m1 = gsp_mmio_rd32(NV_PGSP_FALCON_MBOX1);
        uint32_t gsp_os = gsp_mmio_rd32(NV_PGSP_FALCON_OS);
        uint32_t bcr = gsp_mmio_rd32(LX_FLCN_GSP_BASE + LX_FLCN_ADDR2 + 0x668u);
        uint32_t wpr2_lo = gsp_mmio_rd32(0x001fa824u);
        uint32_t wpr2_hi = gsp_mmio_rd32(0x001fa828u);
        uint32_t sec2_resume = gsp_mmio_rd32(0x001180f8u);
        uint32_t msgq_wptr = 0;

        if (g_libos.ready && g_libos.shm.va) {
            struct gsp_msgq_headers *msgq =
                (struct gsp_msgq_headers *)((unsigned char *)g_libos.shm.va +
                                            g_libos.msgq_offset);
            msgq_wptr = msgq->tx.writePtr;
        }

        if (cpuctl & CPUCTL_ACTIVE_STAT) {
            lx_printk("nouveau-lx: Ampere booter ok, RISC-V activo "
                      "(cpuctl=0x%08x app=0x%08x msgq_wptr=%u)\n",
                      cpuctl, gsp_os, (unsigned)msgq_wptr);
        } else {
            lx_printk("nouveau-lx: Ampere booter ok, RISC-V inactivo "
                      "(cpuctl=0x%08x sec2@0x%x mbox=0x%x/0x%x bcr@0x1668=0x%x "
                      "GSP mbox=0x%x/0x%x os=0x%08x 1180f8=0x%08x msgq_wptr=%u "
                      "WPR2=0x%08x%08x meta=0x%llx heap=%u MiB) — sigue RPC\n",
                      cpuctl, sec2_base, sec2_m0, sec2_m1, bcr,
                      gsp_m0, gsp_m1, gsp_os, sec2_resume, (unsigned)msgq_wptr,
                      wpr2_hi, wpr2_lo,
                      (unsigned long long)g_wpr.meta_phys,
                      (unsigned)(g_wpr.heap_size >> 20));
        }
    }
    return 0;
}

/* Tras un GSP vivo (FMC o booter Ampere): RPC → objetos RM → VMM → CE → pool.
 * Devuelve 0 solo si GSP_INIT_DONE llegó y la cadena RM avanzó. */
static int run_gsp_rm_chain(void);

/* Ampere: layout WPR + FWSEC-FRTS + libos + cmdq, booter SEC2, RPC/GSP_INIT_DONE. */
static int run_ampere_boot(void)
{
    if (!g_rm.ready) {
        lx_printk("nouveau-lx: Ampere sin imagen GSP-RM\n");
        return -1;
    }

    g_phase = GSP_WPR_META;
    if (gsp_wpr_prepare_ampere(&g_rm, &g_wpr) != 0) {
        lx_printk("nouveau-lx: Ampere WPR no preparado\n");
        return -1;
    }

    if (gsp_fwsec_run_frts(g_wpr.meta->frtsOffset, g_wpr.meta->frtsSize) != 0) {
        lx_printk("nouveau-lx: Ampere FWSEC-FRTS falló\n");
        return -1;
    }

    g_phase = GSP_LIBOS_ARGS;
    if (gsp_libos_prepare(&g_wpr, &g_libos) != 0) {
        lx_printk("nouveau-lx: Ampere libos no preparado\n");
        return -1;
    }

    enqueue_boot_rpcs();

    /* Ampere: FWSEC-FRTS → booter SEC2 virgen. ACR (AHESASC) envenena SEC2 si
     * falla (NV_ERR_DMA_IN_USE); no ejecutar antes del booter. Post-GSP: P2. */

    g_phase = GSP_KICK;
    if (run_ampere_booter() != 0) {
        lx_printk("nouveau-lx: Ampere booter_load falló\n");
        return -1;
    }

    if (run_gsp_rm_chain() != 0) {
        return -1;
    }

    g_phase = GSP_BOOTED;
    lx_printk("nouveau-lx: GSP booted (hw, booter_load Ampere + RPC, %u MiB VRAM)\n",
              (unsigned)(g_vram_bytes / (1024ull * 1024ull)));
    return 0;
}

/* G4d (2/2): espacio de direcciones, un bloque de VRAM y una página de sysmem
 * mapeados, y la traducción releída de las tablas.
 *
 * La VA base son 1 TiB por dos razones. Está lejísimos de [4 GiB, 4,5 GiB), que
 * es la franja que RM se reserva para sí en un vaspace de los suyos
 * (`SPLIT_VAS_SERVER_RM_MANAGED_VA_*`) y que más vale no rozar aunque el
 * nuestro sea externo; y tiene el bit 40 puesto, así que obliga a crear tabla en
 * el nivel 3 en vez de dejar el recorrido pegado al índice 0 de todo, que es el
 * error que no se ve porque funciona igual.
 *
 * Lo que se mapea es lo que G4e va a necesitar: un bloque de VRAM para que la
 * copia del CE tenga destino, y una página de sysmem para leer de vuelta desde
 * la CPU sin ventana a la VRAM. */
/* Ver `GSP_VA_BASE` en gsp_vmm.h: la base es una y sale de allí. */
#define G4D_VA_BASE     GSP_VA_BASE
#define G4D_VRAM_BYTES  (2ull * 1024ull * 1024ull)
#define G4D_SCRATCH_VA  (G4D_VA_BASE + G4D_VRAM_BYTES)

/* Comprueba el mapeo por un camino distinto del que lo construyó: recorre las
 * tablas como haría la MMU y contrasta contra lo que se pidió. No es la GPU
 * traduciendo —eso no se sabrá hasta que el CE mueva bytes en G4e— pero sí
 * descarta el error tonto de haber escrito la entrada en el índice de al lado. */
static int vmm_selfcheck(void)
{
    const struct { uint64_t va; uint64_t want; const char *what; } probe[4] = {
        { G4D_VA_BASE,                        g_vram_block,                    "primera página de VRAM" },
        { G4D_VA_BASE + 4096ull,              g_vram_block + 4096ull,          "segunda página de VRAM" },
        { G4D_VA_BASE + G4D_VRAM_BYTES - 4096ull,
          g_vram_block + G4D_VRAM_BYTES - 4096ull,                             "última página de VRAM" },
        { G4D_SCRATCH_VA,                     g_scratch.phys,                  "página de sysmem" },
    };
    unsigned i;
    uint64_t phys = 0;
    uint64_t pte = 0;

    for (i = 0; i < 4; i++) {
        if (gsp_vmm_translate(&g_vmm, probe[i].va, &phys, &pte) != 0) {
            lx_printk("nouveau-lx: VA 0x%llx (%s) no traduce\n",
                      (unsigned long long)probe[i].va, probe[i].what);
            return -1;
        }
        if (phys != probe[i].want) {
            lx_printk("nouveau-lx: VA 0x%llx traduce a 0x%llx y esperaba 0x%llx (%s)\n",
                      (unsigned long long)probe[i].va, (unsigned long long)phys,
                      (unsigned long long)probe[i].want, probe[i].what);
            return -1;
        }
    }

    /* Y lo que NO está mapeado tiene que fallar. Sin esta comprobación, un
     * recorrido que devolviese siempre algo pasaría las cuatro de arriba. */
    if (gsp_vmm_translate(&g_vmm, G4D_SCRATCH_VA + 4096ull, &phys, &pte) == 0) {
        lx_printk("nouveau-lx: una VA sin mapear traduce a 0x%llx\n",
                  (unsigned long long)phys);
        return -1;
    }

    lx_printk("nouveau-lx: traducción verificada en 4 páginas (y una sin mapear "
              "falla como debe)\n");
    return 0;
}

/* Mirar las tablas de BAR1 de RM, sin tocarlas. Es lo único que hace falta para
 * decidir si la CPU puede escribir VRAM por la apertura en vez de por el CE, y
 * la respuesta no se puede deducir sin la tarjeta: hay que ver en qué nivel se
 * corta la cadena que RM dejó hecha. Best-effort puro — no condiciona nada. */
static void run_bar1_probe(void)
{
    uint64_t base, size = 0;

    if (!g_static.ready || !g_pdev) {
        return;
    }
    /* El tamaño de la apertura sale del BAR, no de RM: `sriovCaps.bar1Size` vino
     * a 0 en la GB205 (silicio, 2026-08-01), y sin tamaño no se puede elegir la
     * ventana alta. La base también, por coherencia: es el mismo registro. */
    base = lx_pci_bar1(g_pdev, &size);
    if (gsp_bar1_init(&g_bar1, base, size, g_static.bar1_pde_base) != 0) {
        return;
    }
    /* Ventana propia al final de la apertura, alineada a 2 MiB (una entrada de
     * PD0). Con los 16 GiB de esta tarjeta cae en PD1[31], lejísimos del PD1[0]
     * donde RM tiene lo suyo. */
    if (size >= 2ull * GSP_BAR1_WINDOW_BYTES) {
        g_bar1.window_va = (size - GSP_BAR1_WINDOW_BYTES) & ~(GSP_BAR1_WINDOW_BYTES - 1ull);
    }
    /* DOS sitios, no uno, porque el ciclo de placa cuesta un reinicio y con un
     * solo volcado no se decide nada:
     *
     *  - offset 0: cómo es la cadena que RM ya construyó (hasta qué nivel llega
     *    y en qué apertura viven sus tablas). Es lo que hay que imitar.
     *  - la última ventana de 2 MiB de la apertura: ahí es donde pensamos
     *    colgar NUESTRAS tablas, y lo que hace falta saber es si esa rama está
     *    vacía. Una entrada de PD3 cubre 2^47, así que TODA la apertura vive
     *    dentro de `PD3[0]` — no hay entrada libre que tomar y hay que
     *    descender por la cadena de RM hasta el primer nivel sin construir.
     *    Si esa rama ya estuviera ocupada, escribir ahí pisaría un mapeo que
     *    RM usa, y eso no se arregla: se cuelga la tarjeta. */
    gsp_bar1_dump(&g_bar1, 0);
    if (g_bar1.window_va) {
        gsp_bar1_dump(&g_bar1, g_bar1.window_va);
    } else {
        lx_printk("nouveau-lx: BAR1 — apertura de %llu B: no hay sitio para una "
                  "ventana propia\n", (unsigned long long)g_bar1.aperture_size);
    }
}

static int run_vmm_stage(void)
{
    if (gsp_vram_init(&g_vram_pool, &g_static) != 0) {
        return -1;
    }
    g_vram_block = gsp_vram_alloc(&g_vram_pool, G4D_VRAM_BYTES, G4D_VRAM_BYTES);
    if (!g_vram_block) {
        return -1;
    }
    if (gsp_dma_alloc(&g_scratch, 4096, "página de rebote de G4d") != 0) {
        return -1;
    }
    if (gsp_vmm_init(&g_cmdq, &g_rpc, &g_vmm, &g_vram_pool) != 0) {
        return -1;
    }
    if (gsp_vmm_map(&g_vmm, G4D_VA_BASE, g_vram_block, G4D_VRAM_BYTES,
                    GSP_VMM_VRAM) != 0 ||
        gsp_vmm_map(&g_vmm, G4D_SCRATCH_VA, g_scratch.phys, 4096,
                    GSP_VMM_SYSMEM) != 0) {
        return -1;
    }
    return vmm_selfcheck();
}

static int run_chan_ce_stage(void)
{
    if (gsp_chan_init(&g_vmm.rm, &g_vmm, &g_vram_pool, &g_chan, g_vmm.vaspace,
                      0u, NV2080_ENGINE_TYPE_COPY0) != 0) {
        return -1;
    }
    g_phase = GSP_RM_CHAN;

    if (gsp_ce_init(&g_vmm.rm, &g_chan, &g_ce) != 0) {
        gsp_chan_fini(&g_chan);
        return -1;
    }
    g_phase = GSP_RM_CE;

    /* Criterio GO de G4e: el selftest hace sysmem → VRAM → sysmem esperando el
     * semáforo en cada tramo y comparando el patrón, con el origen borrado
     * entre medias. Si esto pasa, la GPU tradujo nuestras tablas y movió bytes;
     * es la primera prueba de que G4d funciona de verdad y no solo sobre papel. */
    if (gsp_ce_selftest(&g_ce, G4D_SCRATCH_VA, G4D_VA_BASE, g_scratch.va, 4096) == 0) {
        g_ce_verified = 1;
        lx_printk("nouveau-lx: CE readback verificado (G4e GO)\n");
        /* Con el CE ya verificado se puede probar BAR1, que necesita justo eso
         * para ser falsable: escribir por la apertura y releer por el otro
         * camino. Best-effort — si BAR1 no va, el CE sigue siendo la ruta.
         *
         * DOS ventanas, y en este orden, porque separan dos averías distintas
         * con un solo ciclo de placa (la alta falló el 2026-08-01 con
         * `0xbad0ac00`, que es el centinela de acceso rechazado del chip):
         *
         *  - la alta (final de la apertura) no puede chocar con RM, pero puede
         *    caer FUERA del vaspace que RM abrió de verdad: los 16 GiB son el
         *    tamaño del BAR, no una promesa de que RM haya construido tanto.
         *  - la baja (segunda ventana de 2 MiB) está dentro de cualquier límite
         *    razonable y cuelga del PD0 que RM ya tiene, pero es territorio
         *    suyo: si esa entrada está ocupada el mapeo se niega y lo dice.
         *
         * Si la baja va y la alta no, el problema es el límite del vaspace. Si
         * fallan las dos igual, no es la VA. */
        /* ANTES de mapear nada: preguntarle al hardware qué raíz recorre BAR1 y
         * hasta dónde llega su vaspace, en vez de fiarnos de lo que RM contó en
         * `GspStaticConfigInfo`. Tres ciclos de placa se fueron en parchear
         * tablas que la MMU de BAR1 podía no estar mirando siquiera. */
        {
            uint64_t pdb = 0, limite = 0;

            if (gsp_bar1_inst_probe(&g_bar1, &pdb, &limite) != 0) {
                /* RM no ató BAR1 (0xb80f40 a cero). Bajo GSP lockdown esa
                 * escritura se descarta (relee 0; 2026-08-02). Linux en ruta GSP
                 * no hace `tu102_bar_bar1_init`: instala la PDE con
                 * `UPDATE_BAR_PDE` (`r535_bar`). Atarlo desde el host +
                 * invalidate HUB_ONLY|ALL_PDB es el camino nvkm *sin* GSP, y
                 * ALL_PDB barre también el PDB del CE. Sin BAR1 el CE sigue
                 * siendo la ruta a VRAM. */
                lx_printk("nouveau-lx: BAR1 — RM no ató 0xb80f40; no se escribe "
                          "(Linux GSP no hace tu102_bar_bar1_init)\n");
            } else {
                if (pdb && pdb != g_bar1.pd3) {
                    lx_printk("nouveau-lx: BAR1 — usando la raíz del bloque de "
                              "instancia (0x%llx) en vez de la de RM\n",
                              (unsigned long long)pdb);
                    (void)gsp_bar1_init(&g_bar1, g_bar1.aperture_phys,
                                        g_bar1.aperture_size, pdb);
                }
                /* El límite es `vmm->limit - 1`, o sea la última VA válida. Una
                 * ventana por encima no puede traducir por muy bien escrito que
                 * esté el PTE. */
                if (limite && g_bar1.window_va > limite) {
                    uint64_t nueva = ((limite + 1ull) - GSP_BAR1_WINDOW_BYTES) &
                                     ~(GSP_BAR1_WINDOW_BYTES - 1ull);

                    lx_printk("nouveau-lx: BAR1 — la ventana 0x%llx se sale del "
                              "límite 0x%llx; bajándola a 0x%llx\n",
                              (unsigned long long)g_bar1.window_va,
                              (unsigned long long)limite,
                              (unsigned long long)nueva);
                    g_bar1.window_va = nueva;
                }
                gsp_bar1_dump(&g_bar1, g_bar1.window_va);
                if (gsp_bar1_selftest(&g_bar1, &g_ce, g_vram_block, G4D_VA_BASE,
                                      G4D_SCRATCH_VA, g_scratch.va) != 0 &&
                    g_bar1.window_va != GSP_BAR1_WINDOW_BYTES) {
                    lx_printk("nouveau-lx: BAR1 — reintento con ventana BAJA "
                              "(0x%llx)\n",
                              (unsigned long long)GSP_BAR1_WINDOW_BYTES);
                    g_bar1.window_va = GSP_BAR1_WINDOW_BYTES;
                    if (gsp_bar1_selftest(&g_bar1, &g_ce, g_vram_block,
                                          G4D_VA_BASE, G4D_SCRATCH_VA,
                                          g_scratch.va) != 0) {
                        /* Las dos ventanas fallan igual y las escrituras a las
                         * tablas SÍ se quedan (el readback pasa): entonces la MMU
                         * de BAR1 no está mirando la raíz que estamos parcheando.
                         *
                         * La sospecha es la cola de `GspStaticConfigInfo`:
                         * `fb_length` y `gpuNameString` validan el principio y el
                         * medio del struct, pero entre el nombre y `bar1PdeBase`
                         * hay una tira de NvBool y dos NvU16 de RTD3 transcritos
                         * de r570 SIN contraste, y un solo campo de más o de
                         * menos ahí desplaza la raíz al campo vecino.
                         * `bar2PdeBase` también es una raíz válida y su recorrido
                         * sale igual de coherente, así que por el valor no se
                         * distinguen.
                         *
                         * Se prueba, que es más barato que discutirlo: si con la
                         * otra raíz la apertura empieza a funcionar, el struct
                         * está desplazado y hay que corregir la transcripción
                         * (no dejar esto así). */
                        lx_printk("nouveau-lx: BAR1 — las dos ventanas fallan y "
                                  "las tablas sí se escriben: probando con la "
                                  "OTRA raíz (bar2Pde=0x%llx) por si el struct "
                                  "está desplazado\n",
                                  (unsigned long long)g_static.bar2_pde_base);
                        if (gsp_bar1_init(&g_bar1, g_bar1.aperture_phys,
                                          g_bar1.aperture_size,
                                          g_static.bar2_pde_base) == 0) {
                            g_bar1.window_va = GSP_BAR1_WINDOW_BYTES;
                            gsp_bar1_dump(&g_bar1, g_bar1.window_va);
                            (void)gsp_bar1_selftest(&g_bar1, &g_ce, g_vram_block,
                                                    G4D_VA_BASE, G4D_SCRATCH_VA,
                                                    g_scratch.va);
                        }
                    }
                }
            }
        }
    } else {
        lx_printk("nouveau-lx: CE sin readback — canal vivo pero no movió datos\n");
        /* Los avisos de RM sobre el canal llegan por eventos, y si nadie escucha
         * se quedan en la cola: la vez anterior sus dos NOCAT aparecieron páginas
         * más abajo, dentro del alloc siguiente, y parecían de aquél. */
        gsp_rpc_drain(&g_rpc, 200u);
    }
    return 0;
}

static int run_compute_stage(void)
{
    /* El canal de GR0 va primero: sin él el `RM_ALLOC` del objeto de compute
     * vuelve a chocar con el INVALID_CLASS que dio en hardware. */
    if (gsp_chan_init(&g_vmm.rm, &g_vmm, &g_vram_pool, &g_chan_gr, g_vmm.vaspace,
                      1u, NV2080_ENGINE_TYPE_GR0) != 0) {
        lx_printk("nouveau-lx: sin canal de GR0 — no hay compute (el CE sigue "
                  "en pie)\n");
        return -1;
    }
    if (gsp_compute_init(&g_vmm.rm, &g_chan_gr, &g_compute) != 0) {
        gsp_chan_fini(&g_chan_gr);
        return -1;
    }
    g_phase = GSP_RM_COMPUTE;

    /* SASS no necesita el contexto de GR: es un blob en VRAM que copia el CE.
     * En silicio la primera copia CE *después* de PROMOTE_CTX no señalizaba
     * (PBDMA sí consumió el PB; GSP-RM sin RC). Linux no mete un blit de cliente
     * entre el golden ctx de FECS y el siguiente uso del COPY. Se stagea antes. */
    if (gsp_compute_stage_sass(&g_compute, &g_ce, &g_compute.saxpy,
                               G4D_SCRATCH_VA, g_scratch.va, 4096u) != 0) {
        lx_printk("nouveau-lx: SASS de saxpy no llegó a VRAM (el CE no señalizó)\n");
    } else if (gsp_compute_stage_sass(&g_compute, &g_ce, &g_compute.matvec,
                                      G4D_SCRATCH_VA, g_scratch.va, 4096u) != 0) {
        lx_printk("nouveau-lx: SASS de matvec no llegó a VRAM (el CE no señalizó)\n");
    } else {
        int q4k = gsp_compute_stage_sass(&g_compute, &g_ce, &g_compute.matvec_q4k,
                                         G4D_SCRATCH_VA, g_scratch.va, 4096u);
        int q80 = gsp_compute_stage_sass(&g_compute, &g_ce, &g_compute.matvec_q80,
                                         G4D_SCRATCH_VA, g_scratch.va, 4096u);

        lx_printk("nouveau-lx: SASS en VRAM — saxpy %u B, matvec %u B, "
                  "matvec_q4k %u B (%s), matvec_q80 %u B (%s)\n",
                  g_compute.saxpy.sass_len, g_compute.matvec.sass_len,
                  g_compute.matvec_q4k.sass_len, q4k == 0 ? "ok" : "FALLO",
                  g_compute.matvec_q80.sass_len, q80 == 0 ? "ok" : "FALLO");
    }

    /* El contexto del canal de GR, en el orden de upstream: `r535_gr_oneinit`
     * reserva el canal, le cuelga la clase y **después** promociona
     * (`chan.alloc` → `RM_ALLOC` de la clase → `promote_ctx`). Sin contexto
     * promocionado el canal existe y el primer QMD no puede correr.
     *
     * Best-effort como todo lo de esta fase: si falla, el CE y el resto del
     * arranque siguen en pie y el log dice dónde paró. */
    if (gsp_grctx_query(&g_vmm.rm, 0u, &g_grctx) < 0) {
        lx_printk("nouveau-lx: sin tamaños de contexto de GR — no se promociona\n");
    } else if (gsp_grctx_promote(&g_vmm.rm, &g_vmm, &g_vram_pool, &g_chan_gr,
                                 &g_grctx) != 0) {
        lx_printk("nouveau-lx: contexto de GR sin promocionar — el QMD no puede "
                  "correr todavía\n");
    } else if (!g_ce.stuck) {
        /* Misma copia 4 KiB que G4e: si esto no señaliza, el CE murió en el
         * promote (FECS / COPY0 compartido), no en el tamaño del SASS. */
        if (gsp_ce_copy_sync(&g_ce, G4D_VA_BASE, G4D_SCRATCH_VA, 4096u,
                             GSP_CE_WAIT_MS) != 0) {
            lx_printk("nouveau-lx: CE muerto tras PROMOTE_CTX (copia 4 KiB a la "
                      "VA que G4e sí movió)\n");
        } else {
            lx_printk("nouveau-lx: CE vivo tras PROMOTE_CTX\n");
        }
    }
    /* El rebote grande es un lujo, no un requisito: si no hay 1 MiB contiguo o
     * no se puede mapear, G6 sigue con la página de 4 KiB de G4d y lo dice. Lo
     * que no vale es quedarse a medias, con memoria reservada y sin mapear. */
    {
        uint64_t bounce_va = G4D_SCRATCH_VA;
        void *bounce_cpu = g_scratch.va;
        unsigned bounce_len = 4096u;

        if (gsp_dma_alloc_wb(&g_bounce, G6_BOUNCE_BYTES, "rebote de subidas G6") == 0) {
            if (gsp_vmm_map(&g_vmm, G6_BOUNCE_VA, g_bounce.phys, G6_BOUNCE_BYTES,
                            GSP_VMM_SYSMEM) == 0) {
                bounce_va = G6_BOUNCE_VA;
                bounce_cpu = g_bounce.va;
                bounce_len = G6_BOUNCE_BYTES;
            } else {
                lx_printk("nouveau-lx: G6 — rebote de %u KiB sin mapear; se sube "
                          "de 4 KiB en 4 KiB\n", G6_BOUNCE_BYTES >> 10);
                gsp_dma_free(&g_bounce);
            }
        }
        if (gsp_buf_init(&g_buf, &g_vram_pool, &g_vmm, &g_ce, bounce_va,
                         bounce_cpu, bounce_len) != 0) {
            lx_printk("nouveau-lx: G6 — pool de buffers VRAM no inicializado\n");
        } else {
            /* El techo se dice aquí y no sólo el pool: son cifras distintas (pool
             * 11 902 MiB, techo ~108 MiB en la GB205) y confundirlas es lo que hacía
             * que «sin sitio» fuese un misterio. Ver `gsp_buf_vram_free`. */
            lx_printk("nouveau-lx: G6 — buffers VRAM listos (techo residente "
                      "~%llu MiB de %llu MiB de pool; ventana VA %llu MiB, "
                      "tablas libres %u; rebote %u KiB)\n",
                      (unsigned long long)(gsp_buf_vram_free(&g_buf) >> 20),
                      (unsigned long long)((g_vram_pool.total - g_vram_pool.used) >> 20),
                      (unsigned long long)((G6_VA_LIMIT - G6_VA_BASE) >> 20),
                      GSP_VMM_MAX_PT - g_vmm.pt_nr,
                      bounce_len >> 10);
        }
    }
    /* El lanzamiento del QMD NO se hace aquí: el bring-up deja el compute
     * armado y sale. Un kernel que se lance en el arranque y falle deja la
     * tarjeta en un estado del que sólo se sale reseteando el equipo. */
    return 0;
}

/* Tras un GSP vivo (FMC o booter Ampere): RPC → objetos RM → VMM → CE → pool.
 * Devuelve 0 solo si GSP_INIT_DONE llegó y la cadena RM avanzó. */
static int run_gsp_rm_chain(void)
{
    struct gsp_cpu_seq_ctx seq_ctx;

    seq_ctx.libos_phys = g_libos.libos.phys;
    seq_ctx.app_version = g_wpr.boot.app_version;
    gsp_cpu_seq_set_ctx(&seq_ctx);

    if (gsp_rpc_init(&g_libos, &g_rpc) != 0 ||
        gsp_rpc_start(g_wpr.boot.app_version) != 0 ||
        gsp_rpc_wait_event(&g_rpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 4000) != 0) {
        lx_printk("nouveau-lx: GSP arrancado pero GSP-RM no responde por RPC\n");
        return -1;
    }
    gsp_rpc_postinit();
    g_phase = GSP_RM_READY;
    lx_printk("nouveau-lx: GSP-RM listo (RPC en marcha)\n");

    if (gsp_rm_init(&g_cmdq, &g_rpc, &g_rm_obj) == 0) {
        g_phase = GSP_RM_OBJECTS;

        (void)gsp_rm_classes_probe(&g_rm_obj);
        (void)gsp_rm_engines_probe(&g_rm_obj);

        if (gsp_static_info_get(&g_rm_obj, g_vram_bytes, &g_static) == 0 &&
            (run_bar1_probe(), run_vmm_stage() == 0)) {
            g_phase = GSP_RM_VMM;
            if (run_chan_ce_stage() == 0) {
                (void)run_compute_stage();
            }
        }
    }
    return 0;
}

int lx_nouveau_gsp_init(struct lx_pci_dev *pdev)
{
    void *bar;
    uint32_t boot0;

    if (!pdev) {
        return -1;
    }
    if (g_phase != GSP_NONE && g_phase != GSP_GONE) {
        lx_printk("nouveau-lx: GSP ya inicializado (phase=%s) — se ignora otro 10de\n",
                  lx_nouveau_gsp_status());
        return 0;
    }
    if (lx_pci_enable_device(pdev) != 0) {
        lx_printk("nouveau-lx: pci_enable falló\n");
        return -1;
    }
    lx_pci_set_master(pdev);

    gsp_nv_family_set(0, (uint16_t)lx_pci_device_id(pdev));

    bar = lx_pci_iomap(pdev, 0, 16u * 1024u * 1024u);
    if (!bar) {
        lx_printk("nouveau-lx: BAR0 no mapeable\n");
        return -1;
    }
    gsp_mmio_set_bar(bar, 16u * 1024u * 1024u);
    gsp_mmio_set_pci(pdev);   /* para poder mirar la configuración si el MMIO calla */
    g_pdev = pdev;
    boot0 = gsp_mmio_rd32(NV_PMC_BOOT_0_OFF);
    gsp_nv_family_set(boot0, gsp_nv_family_device_id());
    /* La VRAM de verdad la da el hardware (`ga102_fb_vidmem_size`); la tabla por
     * SKU es solo el respaldo para cuando no hay BAR0 que leer. */
    g_vram_bytes = gsp_wpr_vidmem_size();
    if (!g_vram_bytes) {
        g_vram_bytes = vram_for_device(gsp_nv_family_device_id());
    }
    g_phase = GSP_BAR0;
    lx_printk("nouveau-lx: BAR0 boot0=0x%08x dev=0x%04x familia=%s vram=%uMiB\n",
              boot0, gsp_nv_family_device_id(),
              gsp_nv_family_name(gsp_nv_family_of(boot0, gsp_nv_family_device_id())),
              (unsigned)(g_vram_bytes / (1024ull * 1024ull)));
    lx_fatlog_flush();

    if (!gsp_mmio_alive()) {
        g_phase = GSP_GONE;
        lx_printk("nouveau-lx: GPU fuera del bus / sin D0 — NV_PMC_BOOT_0=0x%08x\n",
                  boot0);
        return -1;
    }

    /* Esperar a que el firmware de la GPU acabe su arranque, como
     * `tu102_devinit_post` → `tu102_devinit_wait`. **Pero el registro depende de la
     * familia** y confundirlos cuesta un ciclo: el scratch de la isla GC6
     * (`0x118234`, `NV_PGC6_AON_SECURE_SCRATCH_GROUP_05_0_GFW_BOOT`) es de
     * Turing/Ampere, y en gb202 NVIDIA **no publica ni `dev_gc6_island.h`** — ahí
     * el indicador es `NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE` (0x00ad00bc,
     * SUCCESS = 0xff), que lee `fmc_lx_fsp_probe` y exige `fsp_ready_to_send`
     * antes del COT.
     *
     * Así que en Blackwell esto no se llama: esperaría 2 s a un registro que
     * siempre vale 0 y luego diría en el log que el devinit no ha terminado, que es
     * exactamente el error que se cometió el 2026-07-29. */
    if (gsp_nv_family_of(boot0, gsp_nv_family_device_id()) != NV_FAM_BLACKWELL) {
        (void)gsp_mmio_gfw_wait(GSP_GFW_WAIT_MS, "tras BAR0");
    }

    /* Y la topología según el chip, que es lo que da las direcciones de las
     * runlists sin inferir índices de la tabla de RM. Va aquí porque sólo necesita
     * BAR0 y porque su volcado sirve de referencia para todo lo que viene. */
    (void)gsp_top_probe();

    /* Ola 3: ejercita el grafo nvkm real (device+subdev GSP+falcon) con BAR0.
     * Best-effort — la construcción no toca MMIO; el boot HW real llega tras G1. */
    (void)lx_nvkm_build_gsp(bar);

    g_phase = GSP_FW_LOADING;
    /* Solo el juego de blobs de esta familia: cargar los dos duplicaba 60,6 MiB
     * de ucode (en linux-firmware el gsp de gb205 es symlink al de ga102). */
    if (gsp_fw_load_all(gsp_fw_chip_of(gsp_nv_family_of(boot0, gsp_nv_family_device_id()))) != 0) {
        return -1;
    }
    g_phase = GSP_FW_READY;

    if (gsp_fw_stage_all() != 0) {
        lx_printk("nouveau-lx: GEM staging falló\n");
        return -1;
    }
    g_phase = GSP_FW_STAGED;

    /* La imagen GSP-RM y su radix3: las dos familias la necesitan (el FMC de
     * Blackwell y el ACR de Ampere solo arrancan el cargador; quien lee estos
     * 60 MiB es el propio GSP, por DMA, siguiendo la tabla). Todo en memoria:
     * nada de esto escribe un registro. */
    g_phase = GSP_RM_RADIX3;
    if (gsp_rm_prepare(gsp_fw_chip_of(gsp_nv_family_of(boot0, gsp_nv_family_device_id())), &g_rm) == 0) {
        /* El ucode en bruto ya no hace falta: `g_rm.img` tiene la sección que
         * importa, alineada a página. Son 60,6 MiB de heap de vuelta. */
        gsp_fw_release_one(GSP_FW_UCODE);
    } else {
        lx_printk("nouveau-lx: GSP-RM/radix3 no preparada — sigue soft\n");
    }

    /* El ACR de `acr_fw.c` es el de Ampere (ucode ga102 en SEC2). En Blackwell el
     * falcon ni ejecutaba — `mbox0=0xbadf4100` — porque GB20x arranca por GSP-FMC/FSP.
     * Cada familia va por lo suyo. Tras un GSP vivo, RM → VMM → CE → pool. */
    {
        enum nv_family fam = gsp_nv_family_of(boot0, gsp_nv_family_device_id());

        if (fam == NV_FAM_BLACKWELL) {
            if (run_fmc_blackwell() == 0) {
                if (run_gsp_rm_chain() == 0) {
                    g_phase = GSP_BOOTED;
                    lx_printk("nouveau-lx: GSP booted (hw, GSP-FMC vía FSP, %u MiB VRAM)\n",
                              (unsigned)(g_vram_bytes / (1024ull * 1024ull)));
                    return 0;
                }
            }
        } else if (fam == NV_FAM_AMPERE) {
            if (run_ampere_boot() == 0) {
                return 0;
            }
        } else {
            run_acr_sec2();
        }
    }

    /* Con la tarjeta fuera del bus no hay nada que sondear. */
    if (!gsp_mmio_alive()) {
        g_phase = GSP_GONE;
        lx_printk("nouveau-lx: GPU fuera del bus — sin GSP\n");
        return -1;
    }

    g_phase = GSP_FAILED;
    lx_printk("nouveau-lx: GSP=fallo (phase=%s, %u MiB VRAM)\n",
              lx_nouveau_gsp_status(),
              (unsigned)(g_vram_bytes / (1024ull * 1024ull)));
    return -1;
}

/* Apaga GSP-RM y deja la tarjeta sin DMA (ver gsp_fini.h). Idempotente: una
 * segunda llamada no hace nada y lo dice.
 *
 * `GSP_FINI` NO está en `lx_nouveau_gsp_ready()`: tras esto el GSP no sirve
 * para nada y el cómputo debe irse a la CPU. Es el mismo cuidado que hubo que
 * tener con `set_boot0`, pero al revés — allí una fase buena se pisaba con una
 * peor; aquí hay que asegurarse de que la peor no se lee como buena. */
int lx_nouveau_gsp_fini(void)
{
    int rc;

    if (g_phase == GSP_FINI) {
        lx_printk("nouveau-lx: GSP ya estaba apagado\n");
        return 0;
    }
    if (!lx_nouveau_gsp_ready()) {
        lx_printk("nouveau-lx: nada que apagar (fase=%s)\n", lx_nouveau_gsp_status());
        return -1;
    }
    /* Primero el vaspace: mientras RM tenga apuntado nuestro directorio de
     * páginas, esas páginas de sysmem no se pueden soltar. Y tiene que ser
     * antes de `gsp_fini`, que es quien deja a RM sin RPC y a la tarjeta sin
     * DMA — después ya no habría con quién hablar. */
    /* compute → CE → canales → vaspace, antes de soltar RM y el directorio de
     * páginas. El canal de GR0 se suelta tras su objeto de compute y antes del
     * de COPY0, en orden inverso al de creación. */
    gsp_buf_fini(&g_buf);
    gsp_compute_fini(&g_compute);
    gsp_chan_fini(&g_chan_gr);
    gsp_ce_fini(&g_ce);
    gsp_chan_fini(&g_chan);
    gsp_vmm_fini(&g_vmm);
    gsp_dma_free(&g_bounce);
    gsp_dma_free(&g_scratch);

    rc = gsp_fini(&g_rm_obj, &g_cmdq, &g_rpc, g_pdev);
    /* La fase cambia haya salido bien o mal: el bus master está quitado en los
     * dos casos, así que la tarjeta ya no es utilizable de todas formas. */
    g_phase = GSP_FINI;
    return rc;
}

int lx_nouveau_gsp_ready(void)
{
    /* `rm_ready` es MEJOR que `booted` (el GSP arrancado *y* GSP-RM hablando por
     * RPC), pero al añadir la fase se quedó fuera de esta lista y el estado más
     * avanzado se reportaba como "no listo". */
    return g_phase == GSP_BOOTED || g_phase == GSP_RM_READY ||
           g_phase == GSP_RM_OBJECTS || g_phase == GSP_RM_VMM ||
           g_phase == GSP_RM_CHAN || g_phase == GSP_RM_CE ||
           g_phase == GSP_RM_COMPUTE ? 1 : 0;
}

const char *lx_nouveau_gsp_status(void)
{
    switch (g_phase) {
    case GSP_NONE:
        return "none";
    case GSP_BAR0:
        return "bar0";
    case GSP_FW_LOADING:
        return "fw_loading";
    case GSP_FW_READY:
        return "fw_ready";
    case GSP_FW_STAGED:
        return "fw_staged";
    case GSP_RM_RADIX3:
        return "rm_radix3";
    case GSP_WPR_META:
        return "wpr_meta";
    case GSP_LIBOS_ARGS:
        return "libos_args";
    case GSP_COT_READY:
        return "cot_ready";
    case GSP_COT_SENT:
        return "cot_sent";
    case GSP_FMC_PARSE:
        return "fmc_parse";
    case GSP_FMC_READY:
        return "fmc_ready";
    case GSP_ACR_LOAD:
        return "acr_load";
    case GSP_ACR_AHESASC:
        return "acr_ahesasc";
    case GSP_ACR_ASB:
        return "acr_asb";
    case GSP_KICK:
        return "kick";
    case GSP_POLL:
        return "poll";
    case GSP_BOOTED:
        return "booted";
    case GSP_FAILED:
        return "fallo";
    case GSP_RM_READY:
        return "rm_ready";
    case GSP_RM_OBJECTS:
        return "rm_objects";
    case GSP_RM_VMM:
        return "rm_vmm";
    case GSP_RM_CHAN:
        return "rm_chan";
    case GSP_RM_CE:
        return "rm_ce";
    case GSP_RM_COMPUTE:
        return "rm_compute";
    case GSP_BOOTED_SOFT:
        return "booted_soft";
    case GSP_GONE:
        return "gone";
    case GSP_FINI:
        return "fini";
    default:
        return "?";
    }
}

uint64_t lx_nouveau_vram_bytes(void)
{
    return g_vram_bytes ? g_vram_bytes : (8ull * 1024ull * 1024ull * 1024ull);
}

uint64_t lx_nouveau_buf_alloc(uint64_t size)
{
    if (!g_buf.ready) {
        return 0;
    }
    return gsp_buf_alloc(&g_buf, size);
}

/* ¿Hay pool de VRAM de verdad? Es distinto de `lx_nouveau_gsp_ready()`, que es
 * cierto ya con el GSP arrancado: el pool lo monta `gsp_buf_init` al final de
 * RM → VMM → canal/CE, que corre tras FMC (Blackwell) y tras booter (Ampere).
 * Si esa cadena no llega, el kernel no debe repartir búferes de su heap
 * haciéndolos pasar por VRAM (era lo que ocurría, y `MATVF` los multiplicaba con
 * su bucle de CPU mientras todo decía «offload»). */
int lx_nouveau_buf_ready(void)
{
    /* Pool montado no basta: si el CE se atascó, `GPU_ALLOC_VRAM` seguiría
     * diciendo que sí y askd subiría el 27B a un canal muerto (parece colgado). */
    if (!g_buf.ready || g_ce.stuck) {
        return 0;
    }
    return 1;
}

int lx_nouveau_buf_upload(uint64_t va, const void *src, uint64_t size)
{
    if (!g_buf.ready) {
        return -1;
    }
    return gsp_buf_upload(&g_buf, va, src, size);
}

int lx_nouveau_buf_upload_at(uint64_t va, uint64_t offset, const void *src,
                             uint64_t size)
{
    if (!g_buf.ready) {
        return -1;
    }
    return gsp_buf_upload_at(&g_buf, va, offset, src, size);
}

int lx_nouveau_buf_upload_dma(uint64_t va, uint64_t offset, const uint64_t *phys,
                              unsigned npages, unsigned src_off, uint64_t size)
{
    if (!g_buf.ready) {
        return -1;
    }
    return gsp_buf_upload_dma(&g_buf, va, offset, phys, npages, src_off, size);
}

int lx_nouveau_buf_free(uint64_t va)
{
    if (!g_buf.ready) {
        return -1;
    }
    return gsp_buf_free(&g_buf, va);
}

uint64_t lx_nouveau_buf_vram_free(void)
{
    if (!g_buf.ready) {
        return 0;
    }
    return gsp_buf_vram_free(&g_buf);
}

/* `g_ce_verified` es la puerta: si el CE no demostró en el arranque que mueve
 * bytes por nuestras tablas, lanzar un QMD es tirar trabajo a un canal que no
 * funciona, y eso en esta máquina se paga con un cuelgue sin traza. Sin esa
 * prueba, CPU y a otra cosa. Los dos canales tienen que estar en pie: el de COPY0
 * copia el SASS a VRAM y el de GR0 es el que ejecuta. */
/* Cortacircuitos del camino de GPU.
 *
 * Cada lanzamiento fallido cuesta el timeout del semáforo (2 s). Una inferencia
 * son cientos de matvec, así que un canal de compute roto convertiría "se cae a
 * CPU" en seis minutos de esperas que desde fuera parecen un cuelgue — y en un
 * ciclo de VFIO, que cuesta cerrar la sesión gráfica, eso es el ciclo entero
 * perdido. Tras `G4F_MAX_FALLOS` fallos seguidos se deja de intentar y se dice una
 * vez; un éxito lo reinicia. */
#define G4F_MAX_FALLOS 3
static unsigned g_compute_fallos;
static int g_compute_rendido;

static int compute_usable(void)
{
    if (g_compute_rendido || g_ce.stuck) {
        return 0;
    }
    return g_ce_verified && g_compute.ready && g_ce.ready && g_chan.ready &&
           g_chan_gr.ready && g_phase >= GSP_RM_COMPUTE;
}

/* Contabilidad del cortacircuitos: `ok` = el dispositivo calculó de verdad. */
static void compute_resultado(int ok)
{
    if (ok) {
        g_compute_fallos = 0;
        return;
    }
    if (++g_compute_fallos >= G4F_MAX_FALLOS) {
        g_compute_rendido = 1;
        lx_printk("nouveau-lx: %u lanzamientos seguidos fallidos — camino de GPU "
                  "desactivado, todo a CPU (reinicia soso para reintentarlo)\n",
                  g_compute_fallos);
    }
}

/* El valor de retorno es "esto lo ha calculado la GPU" (1) o "la CPU" (0), y es
 * el criterio GO de G4. Devolver 1 con el GSP arrancado era una mentira: aquí
 * abajo no hay más que un bucle de CPU, no existe todavía canal ni kernel. Un
 * criterio que se cumple solo porque el GSP arrancó no mide nada — es el mismo
 * error que dio `G3b GO` con la tarjeta fuera del bus. Mientras el cómputo sea
 * de CPU esto devuelve 0; pasará a 1 cuando haya canal y el resultado venga de
 * VRAM. */
int lx_nouveau_submit_saxpy(float a, const float *x, float *y, unsigned n)
{
    unsigned i;

    if (!x || !y || n == 0) {
        return -1;
    }
    if (compute_usable()) {
        int ok = gsp_compute_saxpy(&g_compute, &g_ce, a, x, y, n,
                                   G4D_SCRATCH_VA, g_scratch.va) == 0;
        compute_resultado(ok);
        if (ok) {
            return 1;
        }
    }
    for (i = 0; i < n; i++) {
        y[i] = a * x[i] + y[i];
    }
    return 0;
}

int lx_nouveau_submit_matvec_resident(uint64_t w_va, unsigned rows, unsigned cols,
                                      const float *x, float *y)
{
    if (!x || !y || rows == 0 || cols == 0 || w_va == 0) {
        return -1;
    }
    if (compute_usable() && g_compute.res_mapped && g_buf.ready) {
        int ok = gsp_compute_matvec_resident(&g_compute, &g_ce, w_va, rows, cols,
                                             x, y, G4D_SCRATCH_VA,
                                             g_scratch.va) == 0;
        compute_resultado(ok);
        if (ok) {
            return 1;
        }
    }
    return -1;
}

/* G7: igual, con la matriz cuantizada sin expandir. **No hay fallback de CPU aquí**
 * y es deliberado: sin BAR1 el kernel no puede leer esa VRAM, y userspace tiene su
 * matvec fusionado con AVX2 sobre el shard ya mapeado, que es mejor que cualquier
 * bucle escalar de aquí. El -1 significa «hazlo tú». */
int lx_nouveau_submit_matvec_q_resident(uint64_t w_va, unsigned dtype,
                                        unsigned rows, unsigned cols,
                                        const float *x, float *y)
{
    if (!x || !y || rows == 0 || cols == 0 || w_va == 0) {
        return -1;
    }
    if (compute_usable() && g_compute.res_mapped && g_buf.ready) {
        int ok = gsp_compute_matvec_q_resident(&g_compute, &g_ce, w_va, dtype, rows,
                                               cols, x, y, G4D_SCRATCH_VA,
                                               g_scratch.va, 4096u) == 0;
        compute_resultado(ok);
        if (ok) {
            return 1;
        }
    }
    return -1;
}

/* G5. Misma regla que saxpy: el 1 es "lo calculó la GPU" y sólo se devuelve con
 * el semáforo de cada tanda señalizado y el vector releído de la memoria que
 * escribió la GPU. El fallback de CPU está aquí abajo y no es un plan B teórico:
 * es el camino normal mientras `compute_usable()` no se cumpla, y `soso-llm`
 * distingue los dos por este valor de retorno (`on_gpu`), no por adivinarlo. */
int lx_nouveau_submit_matvec_f32(const float *w, unsigned rows, unsigned cols,
                                 const float *x, float *y)
{
    unsigned r, c;

    if (!w || !x || !y || rows == 0 || cols == 0) {
        return -1;
    }
    if (compute_usable()) {
        int ok = gsp_compute_matvec_f32(&g_compute, &g_ce, w, rows, cols, x, y,
                                       G4D_SCRATCH_VA, g_scratch.va) == 0;
        compute_resultado(ok);
        if (ok) {
            return 1;
        }
        /* La GPU no lo hizo. `y` puede tener tandas escritas a medias, así que el
         * bucle de abajo lo recalcula ENTERO: quedarse con lo que sobrevivió
         * sería mezclar dos resultados y no poder distinguirlos después. */
    }
    for (r = 0; r < rows; r++) {
        float sum = 0.0f;
        for (c = 0; c < cols; c++) {
            sum += w[(unsigned long)r * cols + c] * x[c];
        }
        y[r] = sum;
    }
    return 0;
}

int lx_nouveau_submit_matmul_resident(uint64_t w_va, unsigned rows, unsigned cols,
                                      unsigned n, const float *x, float *y)
{
    if (!x || !y || rows == 0 || cols == 0 || n == 0 || w_va == 0) {
        return -1;
    }
    if (compute_usable() && g_compute.res_mapped && g_buf.ready) {
        int ok = gsp_compute_matmul_resident(&g_compute, &g_ce, w_va, rows, cols,
                                             n, x, y, G4D_SCRATCH_VA,
                                             g_scratch.va) == 0;
        compute_resultado(ok);
        if (ok) {
            return 1;
        }
    }
    return -1;
}

int lx_nouveau_submit_softmax_rows(float *x, unsigned rows, unsigned cols)
{
    if (!x || rows == 0 || cols == 0) {
        return -1;
    }
    if (compute_usable() && g_compute.res_mapped) {
        int ok = gsp_compute_softmax_rows(&g_compute, &g_ce, x, rows, cols,
                                          G4D_SCRATCH_VA, g_scratch.va) == 0;
        compute_resultado(ok);
        if (ok) {
            return 1;
        }
    }
    return -1;
}

int lx_nouveau_submit_layernorm_rows(float *x, const float *weight,
                                     const float *bias, unsigned rows,
                                     unsigned cols, float eps)
{
    if (!x || !weight || !bias || rows == 0 || cols == 0) {
        return -1;
    }
    if (compute_usable() && g_compute.res_mapped) {
        int ok = gsp_compute_layernorm_rows(&g_compute, &g_ce, x, weight, bias,
                                              rows, cols, eps, G4D_SCRATCH_VA,
                                              g_scratch.va) == 0;
        compute_resultado(ok);
        if (ok) {
            return 1;
        }
    }
    return -1;
}

int lx_nouveau_compute_wait_fence(unsigned sem_slot)
{
    if (!compute_usable()) {
        return -1;
    }
    return gsp_compute_wait_fence(&g_compute, sem_slot);
}
