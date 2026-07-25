/* G3: GSP bring-up gb205 (Blackwell) + ga10x (Ampere, RTX 3060) —
 * firmware + ACR ola2 + poll MMIO. La ruta nvkm real usa ga102_gsp_new, que
 * cubre Ampere GA10x de forma madura en nouveau 6.6; GB205 es más reciente. */
#include "acr_lx.h"
#include "fmc_lx.h"
#include "gsp_fw.h"
#include "gsp_mmio.h"
#include "fsp_lx.h"
#include "gsp_cmdq.h"
#include "gsp_libos.h"
#include "gsp_rm.h"
#include "gsp_rm_obj.h"
#include "gsp_rpc.h"
#include "gsp_wpr.h"
#include "lx_emul.h"

#define NV_PMC_BOOT_0_OFF 0x0000u
#define GB205_DEVICE_ID   0x2f18u  /* RTX 5070 Ti Mobile (Blackwell) */
#define GSP_POLL_MS         2000u

/* Familia de chip para bring-up chip-aware. */
enum nv_family { NV_FAM_UNKNOWN = 0, NV_FAM_AMPERE, NV_FAM_ADA, NV_FAM_BLACKWELL };

/* NV_PMC_BOOT_0: bits 20-28 = arquitectura (>>20 & 0x1ff). Ampere=0x170,
 * Ada=0x190, Blackwell(GB20x)=0x1a0+. Fallback por device_id si boot0=0. */
static enum nv_family nv_family_of(uint32_t boot0, uint16_t dev_id)
{
    unsigned arch = (boot0 >> 20) & 0x1ffu;
    if (boot0) {
        if (arch >= 0x1a0u) return NV_FAM_BLACKWELL;
        if (arch >= 0x190u) return NV_FAM_ADA;
        if (arch >= 0x170u) return NV_FAM_AMPERE;
    }
    if (dev_id == GB205_DEVICE_ID) return NV_FAM_BLACKWELL;
    /* Ampere consumer: GA102/104/106/107 = 0x22xx..0x25xx (incl. RTX 3060). */
    if ((dev_id & 0xff00u) >= 0x2200u && (dev_id & 0xff00u) <= 0x2500u)
        return NV_FAM_AMPERE;
    return NV_FAM_UNKNOWN;
}

static const char *nv_family_name(enum nv_family f)
{
    switch (f) {
    case NV_FAM_AMPERE:    return "Ampere (ga10x)";
    case NV_FAM_ADA:       return "Ada (ad10x)";
    case NV_FAM_BLACKWELL: return "Blackwell (gb20x)";
    default:               return "desconocida";
    }
}

/* Qué juego de firmware pedir. Ada aún no tiene blobs empaquetados (ad10x); cae
 * en el juego Blackwell, que es el que trae el rootfs. */
static enum gsp_fw_chip gsp_fw_chip_of(enum nv_family f)
{
    return f == NV_FAM_AMPERE ? GSP_FW_CHIP_AMPERE : GSP_FW_CHIP_BLACKWELL;
}

/* Ola 3: construye el grafo de objetos nvkm real con BAR0 (nvkm_bringup_lx.c).
 * Best-effort: no altera la secuencia soft si falla. */
int lx_nvkm_build_gsp(void *bar0);

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
    GSP_RM_READY,
    GSP_RM_OBJECTS,
    GSP_BOOTED_SOFT,
    GSP_GONE,
};

static enum gsp_phase g_phase = GSP_NONE;
static struct gsp_rm_fw g_rm;   /* imagen GSP-RM + radix3, viva hasta el boot */
static struct gsp_wpr g_wpr;    /* bootloader + GspFwWprMeta (solo ruta FMC) */
static struct gsp_libos g_libos;    /* colas, logs, RMARGS y boot params */
static struct fmc_staged g_fmc;     /* imagen FMC + cadena de firma en sysmem */
static struct gsp_rpc g_rpc;        /* anillo de mensajes de GSP-RM */
static struct gsp_cmdq g_cmdq;      /* cola de comandos hacia GSP-RM */
static struct gsp_rm g_rm_obj;      /* cliente/device/subdevice de RM */
static struct gsp_static_info g_static;  /* VRAM utilizable y regalos de RM */
static struct lx_pci_dev *g_pdev;   /* para leer BARs y BDF del espacio de config */
static uint16_t g_device_id;
static uint32_t g_boot0;
static uint64_t g_vram_bytes;

void lx_nouveau_set_boot0(unsigned boot0, unsigned device_id)
{
    g_boot0 = (uint32_t)boot0;
    g_device_id = (uint16_t)device_id;
    /* NO retroceder la fase. `nvidia_probe::init()` corre en el kernel DESPUÉS
     * del bring-up (main.rs: lxdde::init → gpu::init → nvidia_probe::init) y
     * volvía a poner GSP_BAR0 encima de un `rm_ready` ya conseguido: el arranque
     * del 2026-07-25 llegó a `GSP-RM listo` y aun así el log decía `GSP=bar0`,
     * `gsp_ready()` daba falso y el compute se iba a la CPU sin avisar. Esta
     * función solo aporta boot0/device_id; la fase la manda el bring-up. */
    if (g_boot0 != 0 && g_phase == GSP_NONE) {
        g_phase = GSP_BAR0;
    }
}

/* VRAM heurística por SKU (en HW real la da nvkm_ram del fb). El RTX 3060 tiene
 * 12 GiB (GA106) o 8 GiB (3060 Ti/GA104); default Ampere = 12 GiB. */
static uint64_t vram_for_device(uint16_t dev_id)
{
    enum nv_family fam = nv_family_of(g_boot0, dev_id);
    if (dev_id == GB205_DEVICE_ID) {
        return 12ull * 1024ull * 1024ull * 1024ull;   /* 5070 Ti Mobile */
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

/* Ruta Blackwell: el GSP lo arranca el FSP con la imagen GSP-FMC, no el ACR de
 * SEC2. Validamos el ELF firmado, leemos el estado del FSP y construimos el WPR
 * meta (`gsp_wpr.c`) sobre la radix3 de `gsp_rm.c`. Para enviar el COT falta el
 * último escalón: los libos boot args. Ver fmc_lx.c. */
static int run_fmc_blackwell(void)
{
    const struct gsp_fw_blob *fmc = gsp_fw_get(GSP_FW_FMC);
    struct fmc_image img;

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
     * cmdq ANTES de arrancar el GSP (`r535_gsp_oneinit`). Sin ellas se
     * inicializa a ciegas: cientos de NOCAT y NV_ERR_OPERATING_SYSTEM. */
    if (gsp_cmdq_init(&g_libos, &g_cmdq) == 0) {
        struct gsp_sysinfo si;
        collect_sysinfo(&si);
        if (!sysinfo_plausible(&si)) {
            /* Mejor arrancar sin system info que con direcciones falsas: lo
             * segundo mata la GPU, lo primero solo hace fallar el init. */
            lx_printk("nouveau-lx: BARs no creíbles — no se manda SET_SYSTEM_INFO\n");
        } else if (gsp_cmdq_set_system_info(&g_cmdq, &si) != 0 ||
            gsp_cmdq_set_registry(&g_cmdq) != 0) {
            lx_printk("nouveau-lx: no se pudieron encolar SET_SYSTEM_INFO/SET_REGISTRY\n");
        }
    } else {
        lx_printk("nouveau-lx: cmdq no inicializable — GSP-RM arrancará a ciegas\n");
    }

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

static int try_hw_boot(void)
{
    g_phase = GSP_KICK;
    if (gsp_mmio_kick_boot() != 0) {
        return -1;
    }
    g_phase = GSP_POLL;
    if (gsp_mmio_poll_ready(GSP_POLL_MS) == 0) {
        g_phase = GSP_BOOTED;
        lx_printk("nouveau-lx: GSP booted (hw poll ok, %s)\n",
                  nv_family_name(nv_family_of(g_boot0, g_device_id)));
        return 0;
    }
    return -1;
}

int lx_nouveau_gsp_init(struct lx_pci_dev *pdev)
{
    void *bar;
    uint32_t boot0;

    if (!pdev) {
        return -1;
    }
    if (lx_pci_enable_device(pdev) != 0) {
        lx_printk("nouveau-lx: pci_enable falló\n");
        return -1;
    }
    lx_pci_set_master(pdev);

    g_device_id = (uint16_t)lx_pci_device_id(pdev);

    bar = lx_pci_iomap(pdev, 0, 16u * 1024u * 1024u);
    if (!bar) {
        lx_printk("nouveau-lx: BAR0 no mapeable\n");
        return -1;
    }
    gsp_mmio_set_bar(bar, 16u * 1024u * 1024u);
    gsp_mmio_set_pci(pdev);   /* para poder mirar la configuración si el MMIO calla */
    g_pdev = pdev;
    boot0 = gsp_mmio_rd32(NV_PMC_BOOT_0_OFF);
    g_boot0 = boot0;
    /* La VRAM de verdad la da el hardware (`ga102_fb_vidmem_size`); la tabla por
     * SKU es solo el respaldo para cuando no hay BAR0 que leer. */
    g_vram_bytes = gsp_wpr_vidmem_size();
    if (!g_vram_bytes) {
        g_vram_bytes = vram_for_device(g_device_id);  /* usa boot0 para la familia */
    }
    g_phase = GSP_BAR0;
    lx_printk("nouveau-lx: BAR0 boot0=0x%08x dev=0x%04x familia=%s vram=%uMiB\n",
              boot0, g_device_id, nv_family_name(nv_family_of(boot0, g_device_id)),
              (unsigned)(g_vram_bytes / (1024ull * 1024ull)));

    /* Ola 3: ejercita el grafo nvkm real (device+subdev GSP+falcon) con BAR0.
     * Best-effort — la construcción no toca MMIO; el boot HW real llega tras G1. */
    (void)lx_nvkm_build_gsp(bar);

    g_phase = GSP_FW_LOADING;
    /* Solo el juego de blobs de esta familia: cargar los dos duplicaba 60,6 MiB
     * de ucode (en linux-firmware el gsp de gb205 es symlink al de ga102). */
    if (gsp_fw_load_all(gsp_fw_chip_of(nv_family_of(boot0, g_device_id))) != 0) {
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
    if (gsp_rm_prepare(gsp_fw_chip_of(nv_family_of(boot0, g_device_id)), &g_rm) == 0) {
        /* El ucode en bruto ya no hace falta: `g_rm.img` tiene la sección que
         * importa, alineada a página. Son 60,6 MiB de heap de vuelta. */
        gsp_fw_release_one(GSP_FW_UCODE);
    } else {
        lx_printk("nouveau-lx: GSP-RM/radix3 no preparada — sigue soft\n");
    }

    /* El ACR de `acr_fw.c` es el de Ampere (ucode ga102 en SEC2). En Blackwell el
     * falcon ni ejecutaba — `mbox0=0xbadf4100` — porque GB20x arranca por GSP-FMC/FSP.
     * Cada familia va por lo suyo. */
    if (nv_family_of(boot0, g_device_id) == NV_FAM_BLACKWELL) {
        if (run_fmc_blackwell() == 0) {
            /* El GSP lo arrancó el FMC: el kick/poll de tu102 no pinta nada. */
            g_phase = GSP_BOOTED;
            lx_printk("nouveau-lx: GSP booted (hw, GSP-FMC vía FSP, %u MiB VRAM)\n",
                      (unsigned)(g_vram_bytes / (1024ull * 1024ull)));

            /* Ya arrancado, GSP-RM habla por las colas. Escuchar su primer
             * mensaje es best-effort: si no llega, el GSP sigue arrancado. */
            if (gsp_rpc_init(&g_libos, &g_rpc) == 0 &&
                gsp_rpc_start(g_wpr.boot.app_version) == 0 &&
                gsp_rpc_wait_event(&g_rpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 4000) == 0) {
                g_phase = GSP_RM_READY;
                lx_printk("nouveau-lx: GSP-RM listo (RPC en marcha)\n");

                /* G4c: con el RPC vivo en las dos direcciones, pedir los objetos
                 * base de RM. También best-effort: si RM los rechaza, el GSP
                 * sigue arrancado y el diagnóstico queda en el log. */
                if (gsp_rm_init(&g_cmdq, &g_rpc, &g_rm_obj) == 0) {
                    g_phase = GSP_RM_OBJECTS;

                    /* G4d: el mapa de VRAM utilizable sale de aquí, no del WPR
                     * meta — en la ruta FMC esos offsets los pone el FMC y no
                     * los devuelve. Se contrasta contra la VRAM que ya leímos
                     * por registro: si no cuadra, la transcripción del struct
                     * está desplazada y lo demás no es de fiar. */
                    gsp_static_info_get(&g_rm_obj, g_vram_bytes, &g_static);
                }
            } else {
                lx_printk("nouveau-lx: GSP arrancado pero GSP-RM no responde por RPC\n");
            }
            return 0;
        }
    } else {
        run_acr_sec2();
    }

    /* Con la tarjeta fuera del bus no hay nada que sondear: el kick/poll de
     * tu102 leería 0xffffffff y lo tomaría por "listo". */
    if (!gsp_mmio_alive()) {
        g_phase = GSP_GONE;
        lx_printk("nouveau-lx: GPU fuera del bus — sin GSP\n");
        return -1;
    }

    if (try_hw_boot() == 0) {
        return 0;
    }

    g_phase = GSP_BOOTED_SOFT;
    lx_printk("nouveau-lx: GSP booted (soft, %u MiB VRAM)\n",
              (unsigned)(g_vram_bytes / (1024ull * 1024ull)));
    return 0;
}

int lx_nouveau_gsp_ready(void)
{
    /* `rm_ready` es MEJOR que `booted` (el GSP arrancado *y* GSP-RM hablando por
     * RPC), pero al añadir la fase se quedó fuera de esta lista y el estado más
     * avanzado se reportaba como "no listo". */
    return g_phase == GSP_BOOTED || g_phase == GSP_RM_READY ||
           g_phase == GSP_RM_OBJECTS || g_phase == GSP_BOOTED_SOFT ? 1 : 0;
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
    case GSP_RM_READY:
        return "rm_ready";
    case GSP_RM_OBJECTS:
        return "rm_objects";
    case GSP_BOOTED_SOFT:
        return "booted_soft";
    case GSP_GONE:
        return "gone";
    default:
        return "?";
    }
}

uint64_t lx_nouveau_vram_bytes(void)
{
    return g_vram_bytes ? g_vram_bytes : (8ull * 1024ull * 1024ull * 1024ull);
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
    for (i = 0; i < n; i++) {
        y[i] = a * x[i] + y[i];
    }
    return 0;
}

int lx_nouveau_submit_matvec_f32(const float *w, unsigned rows, unsigned cols,
                                 const float *x, float *y)
{
    unsigned r, c;
    if (!w || !x || !y || rows == 0 || cols == 0) {
        return -1;
    }
    for (r = 0; r < rows; r++) {
        float sum = 0.0f;
        for (c = 0; c < cols; c++) {
            sum += w[r * cols + c] * x[c];
        }
        y[r] = sum;
    }
    return 0;   /* CPU — ver la nota de lx_nouveau_submit_saxpy */
}
