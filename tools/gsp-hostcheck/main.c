/* Banco de pruebas en host para los pasos 3 a 6 de la cadena FSP/COT (gsp_rm.c,
 * gsp_wpr.c, gsp_libos.c, fmc_lx.c y fsp_lx.c): los mismos fuentes, con la capa
 * lx y un FSP simulado detrás del MMIO, sobre los blobs reales. Comprueba el
 * parseo de los ELF y cabeceras NVIDIA, la radix3, el GspFwWprMeta, las colas
 * compartidas, los boot args de libos y el paquete COT byte a byte.
 *
 * No sustituye a la prueba en hardware: valida parseo, layout y protocolo, no el
 * arranque real del GSP. Pero el paso 6 escribe en la GPU, así que aquí es donde
 * se caza un paquete mal formado sin arriesgar un cuelgue. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdarg.h>
#include <stddef.h>

#define GFP_KERNEL 0x40u
struct lx_pci_dev;

/* VRAM que finge el registro 0x1183a4 (en MiB), como la RTX 5070 de la máquina. */
#define FAKE_VRAM_MB 12288u

static int lx_printk(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int n = vprintf(fmt, ap);
    va_end(ap);
    return n;
}

/* "Física" simulada: base ficticia + offset del puntero, alineada a página. */
#define FAKE_PHYS_BASE 0x100000000ull
static uint64_t lx_virt_to_phys(const void *p)
{
    return p ? FAKE_PHYS_BASE + ((uintptr_t)p & 0xffffffffull) : 0;
}

static void *lx_alloc_pages_exact(size_t size)
{
    void *p = NULL;
    if (posix_memalign(&p, 4096, (size + 4095) & ~(size_t)4095))
        return NULL;
    memset(p, 0, (size + 4095) & ~(size_t)4095);
    return p;
}
static void lx_free_pages_exact(void *p, size_t size) { (void)size; free(p); }

static void *lx_dma_alloc_coherent(struct lx_pci_dev *d, size_t size, uint64_t *dma, unsigned gfp)
{
    (void)d; (void)gfp;
    void *p = lx_alloc_pages_exact(size);
    if (p && dma)
        *dma = lx_virt_to_phys(p);
    return p;
}
/* En el host no hay diferencia entre WB y UC: es la misma malloc. */
static void *lx_dma_alloc_wb(struct lx_pci_dev *d, size_t size, uint64_t *dma, unsigned gfp)
{
    return lx_dma_alloc_coherent(d, size, dma, gfp);
}
static void lx_dma_flush_range(const void *p, size_t len) { (void)p; (void)len; }
/* La apertura de BAR1 no existe en el host: el selftest de BAR1 no se ejecuta
 * aquí (necesita tarjeta), pero el enlazador quiere el símbolo. */
static void *lx_map_wc(unsigned long phys, unsigned long size)
{ (void)phys; (void)size; return NULL; }
static void lx_dma_free_coherent(struct lx_pci_dev *d, size_t size, void *va, uint64_t dma)
{ (void)d; (void)dma; lx_free_pages_exact(va, size); }

/* Estado de "la GPU no contesta", declarado aquí arriba porque `lx_mdelay` es
 * quien hace avanzar el tiempo simulado y necesita verlo. */
static int fake_gpu_gone;                    /* todo el MMIO a unos */
static unsigned fake_revive_after_mdelays;   /* vuelve tras N ms simulados */

/* El reloj no existe aquí, así que cada `lx_mdelay(1)` cuenta como 1 ms
 * simulado. Sirve para modelar un reset del enlace: la tarjeta vuelve sola tras
 * N milisegundos, que es lo que hace en hardware (207 ms medidos el 2026-07-28)
 * y lo que el driver tiene que esperar en vez de rendirse en la primera lectura. */
static void lx_mdelay(unsigned ms)
{
    (void)ms;
    if (fake_revive_after_mdelays && --fake_revive_after_mdelays == 0u) {
        fake_gpu_gone = 0;
    }
}

static void lx_udelay(unsigned us)
{
    (void)us;
}

uint64_t lx_ktime_get_ns(void)
{
    return 0;
}

/* El apagado quita el bus master; aquí solo se cuenta que lo pida. */
static unsigned pci_master_cleared;
static void lx_pci_clear_master(struct lx_pci_dev *d) { (void)d; pci_master_cleared++; }
static void *lx_kzalloc(size_t size, unsigned gfp)
{ (void)gfp; void *p = malloc(size); if (p) memset(p, 0, size); return p; }
static void lx_kfree(void *p) { free(p); }

/* --- FSP simulado -----------------------------------------------------------
 * Suficiente para ejercitar el camino completo del paso 6 sin GPU: captura el
 * paquete que se escribe en EMEM, responde como respondería el FSP y libera el
 * lockdown tras unas cuantas vueltas. */
#define R_VRAM   0x001183a4u
#define R_THERM  0x00ad00bcu
#define R_QHEAD  0x008f2c00u
#define R_QTAIL  0x008f2c04u
#define R_MHEAD  0x008f2c80u
#define R_MTAIL  0x008f2c84u
#define R_EMEMC  0x008f2ac0u
#define R_EMEMD  0x008f2ac4u
#define R_MBOX0  0x00110040u
#define R_MBOX1  0x00110044u
#define R_HWCFG2 0x001100f4u
#define R_CPUCTL 0x00111388u

static uint32_t fsp_emem[512];
static unsigned fsp_emem_ptr;
static uint32_t fsp_qhead, fsp_qtail, fsp_mhead, fsp_mtail;
static uint32_t fsp_sent[512];
static unsigned fsp_sent_dwords;
static unsigned fsp_mbox_reads;
static uint32_t fsp_reply_error;   /* !=0 para probar el rechazo del FSP */
static int fake_die_after_mtail;   /* 0 = no morir; N = morir tras N lecturas de MSGQ_TAIL */
static unsigned fake_mtail_reads;
static unsigned fake_recover_calls;
static int fake_unload_pending;    /* !=0 → MAILBOX0 acaba dando 0x80000000 */
/* Muerte DENTRO del bucle de arranque del FMC: N = morir en la N-ésima lectura de
 * MAILBOX0, que es la que hace `lockdown_released`. Distinto de
 * `fake_die_after_mtail`, que mata antes, en la respuesta al COT. */
static int fake_die_after_mbox0;
static int fake_died_once;         /* la muerte se dispara una vez, no en bucle */
/* Qué contesta el espacio de configuración. -1 = ni eso responde; 0 = responde
 * (que es lo que hace vfio-pci, emulando el id, aunque el MMIO esté muerto). */
static int fake_recover_ret = -1;
static uint32_t fake_pramin_win;
/* MMU invalidate (tu102_vmm_flush → 0xb830a0/a4/b0). La lectura de INVALIDATE
 * devuelve el valor sin el bit TRIGGER: invalidación instantánea en el mock. */
#define FAKE_MMU_INVAL_PDB       0xb830a0u
#define FAKE_MMU_INVAL_UPPER_PDB 0xb830a4u
#define FAKE_MMU_INVAL           0xb830b0u
static uint32_t fake_mmu_inval_pdb;
static uint32_t fake_mmu_inval_upper;
static uint32_t fake_mmu_inval;

static void fake_fsp_reset(void)
{
    memset(fsp_emem, 0, sizeof(fsp_emem));
    memset(fsp_sent, 0, sizeof(fsp_sent));
    fsp_emem_ptr = fsp_qhead = fsp_qtail = fsp_mhead = fsp_mtail = 0;
    fsp_sent_dwords = fsp_mbox_reads = 0;
    fake_gpu_gone = fake_die_after_mtail = 0;
    fake_mtail_reads = fake_recover_calls = 0;
    fake_die_after_mbox0 = fake_died_once = 0;
    fake_revive_after_mdelays = 0;
    fake_recover_ret = -1;
    fake_pramin_win = 0;
    fake_mmu_inval_pdb = fake_mmu_inval_upper = fake_mmu_inval = 0;
}

/* Escribir QUEUE_HEAD es el timbre: el FSP lee el mensaje y contesta. */
static void fake_fsp_consume(void)
{
    fsp_sent_dwords = fsp_qtail / 4u + 1u;
    if (fsp_sent_dwords > 512) fsp_sent_dwords = 512;
    memcpy(fsp_sent, fsp_emem, fsp_sent_dwords * 4u);
    fsp_qhead = fsp_qtail;            /* consumido */

    fsp_emem[0] = (1u << 31) | (1u << 30);                  /* MCTP SOM|EOM */
    fsp_emem[1] = (0x15u << 24) | (0x10deu << 8) | 0x7eu;   /* NVDM FSP_RESPONSE */
    fsp_emem[2] = 0;                                        /* taskId */
    fsp_emem[3] = 0x14u;                                    /* commandNvdmType = COT */
    fsp_emem[4] = fsp_reply_error;
    fsp_mhead = 0;
    fsp_mtail = 4u * 4u;                                    /* último DWORD escrito */
    /* Las lecturas de MSGQ_TAIL se cuentan desde que hay respuesta encolada: las
     * de fsp_ready_to_send(), anteriores al envío, no deben gastar el contador. */
    fake_mtail_reads = 0;
}

/* Igual que `FAKE_DOORBELL_REG`: literales porque este mock va antes del
 * `#include "nvrm_r570.h"`, atados abajo con asserts de compilación. */
#define FAKE_USERMODE_TIME 0xbb0080u
#define FAKE_PTIMER_TIME   0x009400u
static unsigned fake_usermode_reads;
static int fake_usermode_dead;
static uint32_t fake_clock;

/* Tabla PTOP simulada, con el formato de `ga100_top_parse`: tres palabras por
 * motor, las dos primeras con el bit 31 puesto para encadenar. Se montan GR0 y dos
 * CE con runlists DISTINTAS a propósito: un parser que devolviera siempre la
 * primera entrada, o que ignorase la instancia, pasaría una tabla de un solo
 * motor. La palabra a cero del medio es un hueco, que la tabla real también tiene
 * y no debe cortar el recorrido. */
#define FAKE_PTOP_SCAL   0x0224fcu
#define FAKE_PTOP_INFO   0x022800u
#define FAKE_PTOP_WORDS  10u
#define FAKE_TOP_GR_RUNL   0x00d00000u
#define FAKE_TOP_CE0_RUNL  0x00d00000u
#define FAKE_TOP_CE1_RUNL  0x00d00400u
#define LX_FLCN_SEC2_BASE_LEGACY 0x00840000u

/* PTOP de la ROG GA107 (SOSOLOG 2026-09-09): SEC20 addr=0x087000, GSP0 0x110000. */
#define GA107_PTOP_WORDS 6u
static const uint32_t fake_ptop_ga107[GA107_PTOP_WORDS] = {
    0x8d00000eu, 0x80087001u, 0x00c02000u,
    0x80140002u, 0x80110000u, 0x00000000u,
};

static const uint32_t *fake_ptop_active = NULL;
static unsigned fake_ptop_active_words;

static const uint32_t fake_ptop[FAKE_PTOP_WORDS] = {
    /* GR0: tipo 0x00, inst 0, fault 0x1a | addr 0x400000 reset 2 | runlist */
    0x8000001au, 0x80400002u, FAKE_TOP_GR_RUNL | 0u,
    0u,                                     /* hueco */
    /* CE0: tipo 0x13, inst 0, fault 0x1b | addr 0x104000 reset 3 | runlist, engine 1 */
    0x9300001bu, 0x80104003u, FAKE_TOP_CE0_RUNL | 1u,
    /* CE1: tipo 0x13, inst 1, fault 0x1c | addr 0x105000 reset 4 | runlist, engine 1 */
    0x9301001cu, 0x80105004u, FAKE_TOP_CE1_RUNL | 1u,
};

/* PRAMIN: nv50 (Ampere) 0x001700 o gh100 (Blackwell) 0x10fd40 → FB 0x700000+off. */
#define FAKE_PRAMIN_WINDOW_NV50  0x001700u
#define FAKE_PRAMIN_WINDOW_GB    0x0010fd40u
#define FAKE_PRAMIN_BASE         0x00700000u
#define FAKE_PRAMIN_MMIO_SIZE    0x00100000u
#define FAKE_USERD_OFF_GPPUT 0x8cu
static uint8_t fake_vram[0x400000];

static uint64_t fake_pramin_vaddr(unsigned win_off)
{
    return ((uint64_t)fake_pramin_win << 16) | (uint64_t)win_off;
}

static uint32_t fake_pramin_mmio_rd(unsigned win_off)
{
    uint64_t vaddr = fake_pramin_vaddr(win_off);

    if (vaddr + 4u > sizeof(fake_vram)) {
        return 0;
    }
    return *(const uint32_t *)(fake_vram + vaddr);
}

static void fake_pramin_mmio_wr(unsigned win_off, uint32_t val)
{
    uint64_t vaddr = fake_pramin_vaddr(win_off);

    if (vaddr + 4u > sizeof(fake_vram)) {
        return;
    }
    *(uint32_t *)(fake_vram + vaddr) = val;
}

static uint32_t gsp_mmio_rd32(uint32_t off)
{
    /* Tarjeta fuera del bus: TODO se lee a unos, sin excepciones. */
    if (fake_gpu_gone) {
        return 0xffffffffu;
    }
    switch (off) {
    case R_VRAM:  return FAKE_VRAM_MB;
    case R_THERM: return 0xffu;                 /* secure boot completo */
    case R_QHEAD: return fsp_qhead;
    case R_QTAIL: return fsp_qtail;
    case R_MHEAD: return fsp_mhead;
    case R_MTAIL:
        /* La muerte se arma en la lectura N y surte efecto a partir de la N+1:
         * así el poll de fsp_wait_reply ve la cola con datos y el de fsp_recv,
         * una vuelta después, se encuentra el all-ones. Ése es el orden exacto
         * en que ocurrió el 2026-07-27.
         *
         * Sólo se cuenta con respuesta ya encolada (mhead != mtail). Si no,
         * la lectura que fsp_ready_to_send() hace ANTES del envío gastaba el
         * contador y la tarjeta moría antes de mandar el COT: otro fallo, no el
         * que se quiere reproducir. */
        if (fake_die_after_mtail && fsp_mhead != fsp_mtail &&
            ++fake_mtail_reads >= (unsigned)fake_die_after_mtail) {
            fake_gpu_gone = 1;
        }
        return fsp_mtail;
    case R_EMEMD: return fsp_emem_ptr < 512 ? fsp_emem[fsp_emem_ptr++] : 0;
    /* El bootrom tarda unas vueltas en dejar leer el falcon. Tras el aviso de
     * descarga, en cambio, MAILBOX0 tiene que acabar valiendo 0x80000000: el
     * banco lo simula con unas vueltas de retardo para que el sondeo de
     * `gsp_fini` se ejercite de verdad y no acierte en la primera lectura. */
    case R_MBOX0:
        if (fake_unload_pending) {
            return fsp_mbox_reads++ < 3 ? 0u : 0x80000000u;
        }
        /* Muerte a mitad del arranque del FMC: `lockdown_released` sondea aquí,
         * así que la N-ésima lectura es un punto de corte controlado. Se dispara
         * una sola vez para poder simular que la tarjeta vuelve. */
        if (fake_die_after_mbox0 && !fake_died_once &&
            (int)fsp_mbox_reads + 1 >= fake_die_after_mbox0) {
            fake_died_once = 1;
            fake_gpu_gone = 1;
        }
        return fsp_mbox_reads++ < 3 ? 0xbadf4100u : 0u;
    case R_MBOX1: return 0;
    case R_HWCFG2: return 0;                    /* lockdown liberado */
    case R_CPUCTL: return 0x180u;               /* RISC-V activo (bit 7) */
    /* Los dos relojes de `chan_probe_usermode`. Avanzan en cada lectura para que
     * la comprobación recorra su camino bueno; el contador de lecturas del de
     * usermode es la señal observable de que el port mira ESE offset y no otro
     * (mismo truco que `FAKE_DOORBELL_REG`). */
    case FAKE_USERMODE_TIME:
        fake_usermode_reads++;
        /* Aperture muerto: el anillo PRI contesta con un `0xbadfxxxx`, que es lo
         * que se vería si el usermode de este chip no estuviera en 0xbb0000. */
        return fake_usermode_dead ? 0xbadf1000u : ++fake_clock;
    case FAKE_PTIMER_TIME:   return ++fake_clock;
    /* Devinit del firmware de la GPU ya terminado: `tu102_devinit_wait` quiere el
     * bit 0 de 0x118128 y 0xff en 0x118234. */
    case 0x118128u: return 1u;
    case 0x118234u: return 0xffu;
    case FAKE_PTOP_SCAL: {
        unsigned w = fake_ptop_active_words ? fake_ptop_active_words : FAKE_PTOP_WORDS;
        return w << 20;
    }
    case FAKE_PRAMIN_WINDOW_NV50:
    case FAKE_PRAMIN_WINDOW_GB:
        return fake_pramin_win;
    case FAKE_MMU_INVAL_PDB: return fake_mmu_inval_pdb;
    case FAKE_MMU_INVAL_UPPER_PDB: return fake_mmu_inval_upper;
    case FAKE_MMU_INVAL: return fake_mmu_inval;
    default:
        if (off >= FAKE_PRAMIN_BASE &&
            off + 4u <= FAKE_PRAMIN_BASE + FAKE_PRAMIN_MMIO_SIZE) {
            return fake_pramin_mmio_rd(off - FAKE_PRAMIN_BASE);
        }
        if (off >= FAKE_PTOP_INFO) {
            const uint32_t *tab = fake_ptop_active ? fake_ptop_active : fake_ptop;
            unsigned w = fake_ptop_active_words ? fake_ptop_active_words : FAKE_PTOP_WORDS;
            unsigned idx = (off - FAKE_PTOP_INFO) / 4u;
            if (idx < w) {
                return tab[idx];
            }
        }
        return 0;
    }
}

/* La caída del bus sí se simula: es el fallo del 2026-07-27 y se colaba por el
 * agujero de que all-ones cumple `head == tail`. `fake_recover_calls` es la señal
 * observable de que el driver la diagnostica como caída (intenta recuperar por
 * espacio de configuración) y no como cola vacía. */
static int gsp_mmio_alive(void) { return !fake_gpu_gone; }
static int gsp_mmio_pci_recover(void) { fake_recover_calls++; return fake_recover_ret; }
static int gsp_mmio_pri_error(uint32_t v) { return (v & 0xffff0000u) == 0xbadf0000u; }

/* Doorbell del canal. El literal es inevitable —este mock está ANTES del
 * `#include "nvrm_r570.h"`, así que aquí `NV_VFN_DOORBELL` todavía no existe—,
 * pero abajo hay un assert de compilación que ata los dos números: un mock que
 * escucha en un registro y un port que escribe en otro darían verde sin que se
 * pateara nada. */
#define FAKE_DOORBELL_REG  0xbb0090u
static unsigned fake_doorbell_writes;
static uint32_t fake_doorbell_last;

static void gsp_mmio_wr32(uint32_t off, uint32_t val)
{
    switch (off) {
    case R_EMEMC: fsp_emem_ptr = (val & 0xffffffu) / 4u; break;
    case R_EMEMD: if (fsp_emem_ptr < 512) fsp_emem[fsp_emem_ptr++] = val; break;
    case R_QTAIL: fsp_qtail = val; break;
    case R_QHEAD: fsp_qhead = val; fake_fsp_consume(); break;
    case R_MHEAD: fsp_mhead = val; break;
    case R_MTAIL: fsp_mtail = val; break;
    case FAKE_DOORBELL_REG:
        fake_doorbell_writes++;
        fake_doorbell_last = val;
        break;
    case FAKE_PRAMIN_WINDOW_NV50:
    case FAKE_PRAMIN_WINDOW_GB:
        fake_pramin_win = val;
        break;
    case FAKE_MMU_INVAL_PDB:
        fake_mmu_inval_pdb = val;
        break;
    case FAKE_MMU_INVAL_UPPER_PDB:
        fake_mmu_inval_upper = val;
        break;
    case FAKE_MMU_INVAL:
        fake_mmu_inval = val & ~0x80000000u;
        break;
    default:
        if (off >= FAKE_PRAMIN_BASE &&
            off + 4u <= FAKE_PRAMIN_BASE + FAKE_PRAMIN_MMIO_SIZE) {
            fake_pramin_mmio_wr(off - FAKE_PRAMIN_BASE, val);
            break;
        }
        break;
    }
}

/* gsp_fw.h simulado. */
enum gsp_fw_kind { GSP_FW_BOOTLOADER = 0, GSP_FW_FMC, GSP_FW_UCODE, GSP_FW_BOOTER_LOAD,
                   GSP_FW_BOOTER_UNLOAD, GSP_FW_COUNT };
enum gsp_fw_chip { GSP_FW_CHIP_BLACKWELL = 0, GSP_FW_CHIP_AMPERE };
struct gsp_fw_blob { const char *path; unsigned char *data; unsigned long len;
                     unsigned gem_handle; int valid; };
static struct gsp_fw_blob g_blobs[GSP_FW_COUNT];
static const struct gsp_fw_blob *gsp_fw_get(enum gsp_fw_kind k)
{ return k < GSP_FW_COUNT && g_blobs[k].valid ? &g_blobs[k] : NULL; }

#include "nvrm_r570.h"

/* El mock del doorbell escucha en un literal porque se define antes que el
 * header. Esto es lo que impide que los dos números se separen. */
typedef char fake_doorbell_reg_check[
    FAKE_DOORBELL_REG == NV_VFN_DOORBELL ? 1 : -1];
/* Y el reloj de usermode, por lo mismo: si el aperture se mueve, el mock tiene que
 * moverse con él o la sonda daría verde leyendo un registro que ya no es. */
typedef char fake_usermode_reg_check[
    FAKE_USERMODE_TIME == NV_VFN_USERMODE_BASE + 0x80u ? 1 : -1];

/* La decodificación de bloques cuantizados, compilada con clang en C plano: la
 * MISMA cabecera que compila nvcc dentro de los `.cu`. Es el único amarre sin GPU
 * entre lo que decodifica el silicio y lo que decodifica la CPU (el Rust de
 * `sosomodel::dequant`), y por eso está aquí y no dentro del kernel. */
#include "q4k_decode.h"

#include "gsp_dma_body.inc"
#include "gsp_rm_body.inc"
#include "gsp_wpr_body.inc"
#include "gsp_libos_body.inc"
#include "fmc_lx_body.inc"
#include "fsp_lx_body.inc"

#ifndef LX_FLCN_SEC2_BASE
#define LX_FLCN_SEC2_BASE 0x00840000u
#define LX_FLCN_GSP_BASE  0x00110000u
#define LX_FLCN_ADDR2     0x00001000u
#endif

static int falcon_lx_reset(unsigned base)
{
    (void)base;
    return 0;
}

static int falcon_lx_gsp_reset_riscv(unsigned base)
{
    (void)base;
    return 0;
}

static int falcon_lx_start(unsigned base)
{
    (void)base;
    return 0;
}

#include "gsp_cpu_seq_body.inc"
#include "gsp_rpc_body.inc"
#include "gsp_cmdq_body.inc"
#include "gsp_rm_obj_body.inc"
#include "gsp_vram_body.inc"
#include "gsp_chip_body.inc"
#include "gsp_pramin_body.inc"
#include "gsp_vmm_body.inc"
#include "gsp_top_body.inc"
#include "gsp_chan_body.inc"
#include "gsp_ce_body.inc"
#include "gsp_bar1_body.inc"
#include "gsp_grctx_body.inc"
#include "gsp_buf_body.inc"
/* R5: los cuerpos van sin sus `#include`, así que las definiciones de los
 * juegos de SASS y de las familias se traen aquí (autónomas a propósito). */
#include "gsp_sass.h"
#include "sass_sets.h"
#include "gsp_compute_body.inc"
#include "gsp_fini_body.inc"

static int load_blob(enum gsp_fw_kind kind, const char *path)
{
    FILE *f = fopen(path, "rb");
    if (!f) { perror(path); return -1; }
    fseek(f, 0, SEEK_END);
    long len = ftell(f);
    fseek(f, 0, SEEK_SET);
    unsigned char *buf = malloc(len);
    if (fread(buf, 1, len, f) != (size_t)len) { perror("read"); return -1; }
    fclose(f);
    g_blobs[kind].data = buf;
    g_blobs[kind].len = len;
    g_blobs[kind].valid = 1;
    g_blobs[kind].path = path;
    printf("blob %s: %ld bytes\n", path, len);
    return 0;
}

/* G4f/G5: el blob SASS que se enlaza al kernel tiene que ser el fichero entero.
 * El generador se comía la última línea de `xxd -i` (8 B = media instrucción de
 * 16 B) y el kernel lanzaba un programa cortado; el único síntoma habría sido un
 * cuelgue de máquina sin traza. Se compara contra el .bin, byte a byte, y se
 * repite por kernel: cuando esto miraba solo saxpy, un matvec sin compilar (blob
 * a cero) habría pasado el banco entero. */
static int check_one_sass(const char *path, const char *what,
                          const unsigned char *embed, unsigned embed_len)
{
    unsigned char *buf;
    long len;
    FILE *f = fopen(path, "rb");

    if (!f) { perror(path); return -1; }
    fseek(f, 0, SEEK_END);
    len = ftell(f);
    fseek(f, 0, SEEK_SET);
    buf = malloc(len > 0 ? len : 1);
    if (fread(buf, 1, len, f) != (size_t)len) { perror("read sass"); fclose(f); return -1; }
    fclose(f);

    if (len < 16 || embed_len < 16) {
        printf("FALLO: SASS de %s vacío (fichero %ld B, embed %u B) — falta "
               "scripts/l6-g4f-build-sass.sh\n", what, len, embed_len);
        return -1;
    }
    if ((long)embed_len != len) {
        printf("FALLO: el embed de %s tiene %u B y el .bin %ld B (truncado)\n",
               what, embed_len, len);
        return -1;
    }
    if (embed_len % 16 != 0) {
        printf("FALLO: %s — %u B no es múltiplo de 16 (instrucción SASS)\n",
               what, embed_len);
        return -1;
    }
    if (memcmp(buf, embed, len) != 0) {
        printf("FALLO: el embed de %s no coincide con su .bin\n", what);
        return -1;
    }
    free(buf);
    printf("OK: %s SASS blob %u B (= .bin, %u instrucciones)\n",
           what, embed_len, embed_len / 16);
    return 0;
}

/* R5: un juego de SASS por arquitectura, y cada blob igual a su .bin. */
static int check_sass_sets(void)
{
    static const struct gsp_sass_set *const sets[] = GSP_SASS_SETS;
    static const char *const nombres[] = {
        "saxpy", "matvec", "matvec_q4k", "matvec_q80",
        "matmul", "softmax_rows", "layernorm_rows",
    };
    unsigned n = (unsigned)(sizeof(nombres) / sizeof(nombres[0]));
    unsigned s_i, k_i;

    if (GSP_SASS_SET_COUNT < 2u) {
        printf("FALLO: solo %u juego(s) de SASS; hacen falta Ampere y "
               "Blackwell\n", GSP_SASS_SET_COUNT);
        return -1;
    }
    for (s_i = 0; s_i < GSP_SASS_SET_COUNT; s_i++) {
        const struct gsp_sass_set *set = sets[s_i];

        if (!set || !set->arch || set->count != n) {
            printf("FALLO: juego %u incompleto (%u kernels, esperados %u)\n",
                   s_i, set ? set->count : 0u, n);
            return -1;
        }
        if (set->family == GSP_FAM_DESCONOCIDA) {
            printf("FALLO: el juego %s no dice a qué familia sirve\n", set->arch);
            return -1;
        }
        for (k_i = 0; k_i < n; k_i++) {
            const struct gsp_sass_variant *v = gsp_sass_variant_of(set, nombres[k_i]);
            char path[512];

            if (!v) {
                printf("FALLO: %s no trae %s\n", set->arch, nombres[k_i]);
                return -1;
            }
            snprintf(path, sizeof(path), "%s/lxdde/ports/nouveau/sass/%s/%s.sass.bin",
                     getenv("SOSO_ROOT") ? getenv("SOSO_ROOT") : ".",
                     set->arch, nombres[k_i]);
            if (check_one_sass(path, nombres[k_i], v->sass, v->sass_len) != 0) {
                return -1;
            }
        }
        /* Kernels distintos dentro del mismo juego: un copia-pega en el
         * generador daría verde en todo lo anterior. */
        {
            const struct gsp_sass_variant *a = gsp_sass_variant_of(set, "saxpy");
            const struct gsp_sass_variant *b = gsp_sass_variant_of(set, "matvec");

            if (a->sass_len == b->sass_len &&
                memcmp(a->sass, b->sass, a->sass_len) == 0) {
                printf("FALLO: %s — saxpy y matvec son el mismo blob\n", set->arch);
                return -1;
            }
            if (a->param_count != 4u || b->param_count != 5u) {
                printf("FALLO: %s — params del cubin: saxpy %u (4), matvec %u (5)\n",
                       set->arch, a->param_count, b->param_count);
                return -1;
            }
        }
        printf("OK: juego %s (familia %u) con %u kernels\n", set->arch,
               set->family, set->count);
    }
    /* Los juegos NO son intercambiables: si dos arquitecturas dieran el mismo
     * blob o el mismo `param_base`, elegir por familia no serviría de nada. */
    {
        const struct gsp_sass_variant *a = gsp_sass_variant_of(sets[0], "matvec");
        const struct gsp_sass_variant *b = gsp_sass_variant_of(sets[1], "matvec");

        if (a->sass_len == b->sass_len &&
            memcmp(a->sass, b->sass, a->sass_len) == 0) {
            printf("FALLO: %s y %s traen el mismo matvec\n", sets[0]->arch,
                   sets[1]->arch);
            return -1;
        }
        if (a->param_base == b->param_base) {
            printf("aviso: %s y %s comparten param_base=%u\n", sets[0]->arch,
                   sets[1]->arch, a->param_base);
        } else {
            printf("OK: param_base distinto por arquitectura (%s=%u, %s=%u): "
                   "lanzar el juego equivocado leería basura\n",
                   sets[0]->arch, a->param_base, sets[1]->arch, b->param_base);
        }
    }
    return 0;
}

static int check_qmd_v02_fields(const struct gsp_compute *cp,
                                const struct gsp_kernel *k, unsigned grid,
                                const GspQmdV02 *q);

/* R5: la familia decide clase, QMD y SASS, y un desajuste se rechaza ANTES
 * de enviar. Aceptar la clase en RM no basta. */
static int check_family_caps(void)
{
    static const struct gsp_sass_set *const sets[] = GSP_SASS_SETS;
    const struct gsp_family_caps *amp = gsp_family_caps_of(GSP_FAM_AMPERE);
    const struct gsp_family_caps *bw = gsp_family_caps_of(GSP_FAM_BLACKWELL);
    const struct gsp_sass_set *set_amp;
    const struct gsp_sass_set *set_bw;

    if (gsp_family_from_class(AMPERE_COMPUTE_B) != GSP_FAM_AMPERE ||
        gsp_family_from_class(AMPERE_COMPUTE_A) != GSP_FAM_AMPERE ||
        gsp_family_from_class(BLACKWELL_COMPUTE_B) != GSP_FAM_BLACKWELL ||
        gsp_family_from_class(ADA_COMPUTE_A) != GSP_FAM_ADA ||
        gsp_family_from_class(0x1234u) != GSP_FAM_DESCONOCIDA) {
        printf("FALLO: familia mal deducida de la clase de compute\n");
        return -1;
    }
    if (!amp || !bw) {
        printf("FALLO: falta la tabla de capacidades de Ampere o Blackwell\n");
        return -1;
    }
    if (bw->qmd_version != GSP_QMD_VERSION_CURRENT ||
        bw->qmd_bytes != sizeof(GspQmdV05)) {
        printf("FALLO: Blackwell debería usar el QMD v%u de %zu B\n",
               GSP_QMD_VERSION_CURRENT, sizeof(GspQmdV05));
        return -1;
    }
    if (amp->qmd_version != GSP_QMD_VERSION_AMPERE ||
        amp->qmd_bytes != sizeof(GspQmdV02)) {
        printf("FALLO: Ampere debería usar QMD v2 de %zu B\n",
               sizeof(GspQmdV02));
        return -1;
    }
    set_amp = gsp_sass_pick(sets, GSP_SASS_SET_COUNT, GSP_FAM_AMPERE);
    set_bw = gsp_sass_pick(sets, GSP_SASS_SET_COUNT, GSP_FAM_BLACKWELL);
    if (!set_amp || !set_bw || set_amp == set_bw) {
        printf("FALLO: cada familia debe elegir su propio juego de SASS\n");
        return -1;
    }
    if (strcmp(set_amp->arch, amp->sass_arch) != 0 ||
        strcmp(set_bw->arch, bw->sass_arch) != 0) {
        printf("FALLO: el juego elegido (%s/%s) no es el que pide la tabla "
               "(%s/%s)\n", set_amp->arch, set_bw->arch, amp->sass_arch,
               bw->sass_arch);
        return -1;
    }
    if (gsp_sass_pick(sets, GSP_SASS_SET_COUNT, GSP_FAM_HOPPER) != 0) {
        printf("FALLO: no hay SASS de Hopper y se ha elegido uno\n");
        return -1;
    }

    /* El rechazo antes de submit, con un `gsp_compute` de mentira. */
    {
        struct gsp_compute cp;
        struct gsp_kernel k;

        memset(&cp, 0, sizeof(cp));
        memset(&k, 0, sizeof(k));
        cp.ready = 1;
        k.name = "matvec";
        k.staged = 1;

        cp.family = GSP_FAM_BLACKWELL;
        cp.caps = bw;
        cp.sass = set_bw;
        k.arch = set_bw->arch;
        if (gsp_compute_launch_ready(&cp, &k, "prueba") != 0) {
            printf("FALLO: Blackwell con su SASS debería poder lanzar\n");
            return -1;
        }
        k.arch = set_amp->arch;
        if (gsp_compute_launch_ready(&cp, &k, "prueba") == 0) {
            printf("FALLO: se admitió SASS de %s en Blackwell\n", set_amp->arch);
            return -1;
        }
        k.arch = set_bw->arch;
        k.staged = 0;
        if (gsp_compute_launch_ready(&cp, &k, "prueba") == 0) {
            printf("FALLO: se admitió un kernel que no está en VRAM\n");
            return -1;
        }
        k.staged = 1;
        cp.family = GSP_FAM_AMPERE;
        cp.caps = amp;
        cp.sass = set_amp;
        k.arch = set_amp->arch;
        if (gsp_compute_launch_ready(&cp, &k, "prueba") != 0) {
            printf("FALLO: Ampere con sm_86 debería poder lanzar (QMD v2)\n");
            return -1;
        }
        k.arch = set_bw->arch;
        if (gsp_compute_launch_ready(&cp, &k, "prueba") == 0) {
            printf("FALLO: se admitió SASS de %s en Ampere\n", set_bw->arch);
            return -1;
        }
        {
            GspQmdV02 q2;
            struct gsp_compute cp_amp;
            struct gsp_kernel k_amp;
            const struct gsp_sass_variant *sv =
                gsp_sass_variant_of(set_amp, "saxpy");

            if (!sv) {
                printf("FALLO: sin saxpy sm_86 para QMD v2\n");
                return -1;
            }
            memset(&cp_amp, 0, sizeof(cp_amp));
            memset(&k_amp, 0, sizeof(k_amp));
            cp_amp.ready = 1;
            cp_amp.data_va = G4F_DATA_VA;
            cp_amp.family = GSP_FAM_AMPERE;
            cp_amp.caps = amp;
            cp_amp.sass = set_amp;
            k_amp.name = sv->name;
            k_amp.arch = set_amp->arch;
            k_amp.regcount = sv->regcount;
            k_amp.cbank_size = sv->cbank_size;
            k_amp.sass_va = G4F_SASS_VA;
            k_amp.staged = 1;
            gsp_compute_fill_qmd_grid(&cp_amp, &k_amp, (GspQmdV05 *)&q2, 4, 1, 0);
            if (check_qmd_v02_fields(&cp_amp, &k_amp, 4, &q2) != 0)
                return -1;
        }
        cp.caps = 0;
        if (gsp_compute_launch_ready(&cp, &k, "prueba") == 0) {
            printf("FALLO: familia sin capacidades y aun así lanza\n");
            return -1;
        }
    }
    printf("OK: por familia — clase, QMD y SASS; Ampere usa QMD v2 (256 B)\n");
    return 0;
}

/* Paso 3: imagen GSP-RM + radix3, en las dos familias. */
static int check_radix3(struct gsp_rm_fw *keep)
{
    const enum gsp_fw_chip chips[] = { GSP_FW_CHIP_BLACKWELL, GSP_FW_CHIP_AMPERE };

    for (unsigned c = 0; c < 2; c++) {
        struct gsp_rm_fw fw;
        if (gsp_rm_prepare(chips[c], &fw) != 0) {
            printf("FALLO: gsp_rm_prepare chip=%u\n", chips[c]);
            return -1;
        }
        unsigned long pages = (fw.img_len + 4095) / 4096;
        if (fw.rx3.mem[2].size != ((pages * 8 + 4095) & ~4095ul)) { printf("FALLO: tamaño hoja\n"); return -1; }
        if (fw.rx3.mem[1].size != 4096 || fw.rx3.mem[0].size != 4096) { printf("FALLO: tamaño l1/raíz\n"); return -1; }
        if ((uintptr_t)fw.img & 4095) { printf("FALLO: imagen no alineada\n"); return -1; }
        const uint64_t *l2 = fw.rx3.mem[2].va;
        for (unsigned long j = 0; j < pages; j++) {
            if (l2[j] != lx_virt_to_phys(fw.img + j * 4096)) { printf("FALLO: hoja %lu\n", j); return -1; }
        }
        /* Las entradas sobrantes de la hoja quedan a cero (página parcial). */
        for (unsigned long j = pages; j < fw.rx3.mem[2].size / 8; j++) {
            if (l2[j] != 0) { printf("FALLO: relleno hoja %lu no nulo\n", j); return -1; }
        }
        printf("OK: radix3 %lu páginas, todas las hojas cuadran\n", pages);
        if (c == 0)
            *keep = fw;          /* la de Blackwell alimenta el paso 4 */
        else
            gsp_rm_release(&fw);
    }
    return 0;
}

/* Paso 4: bootloader en sysmem + GspFwWprMeta. */
static int check_wpr(const struct gsp_rm_fw *rm, struct gsp_wpr *keep)
{
    struct gsp_wpr w;

    if (sizeof(struct gsp_wpr_meta) != 256) { printf("FALLO: meta no mide 256 B\n"); return -1; }
    if (gsp_wpr_prepare(rm, &w) != 0) { printf("FALLO: gsp_wpr_prepare\n"); return -1; }

    const struct gsp_wpr_meta *m = w.meta;
    if (w.fb_bytes != (uint64_t)FAKE_VRAM_MB << 20) { printf("FALLO: VRAM\n"); return -1; }
    /* 22 + 14 MiB + ALIGN(96 KiB * 12 GiB, 1 MiB) + 96 MiB = 134 MiB. */
    uint64_t want_heap = (22ull << 20) + (14ull << 20) + (2ull << 20) + (96ull << 20);
    if (m->gspFwHeapSize != want_heap) {
        printf("FALLO: heap %llu != %llu\n", (unsigned long long)m->gspFwHeapSize,
               (unsigned long long)want_heap);
        return -1;
    }
    /* Offsets del bootloader: los del blob real, y dentro de la imagen copiada. */
    if (m->bootloaderManifestOffset != 0x0 || m->bootloaderDataOffset != 0xa00 ||
        m->bootloaderCodeOffset != 0xb200) {
        printf("FALLO: offsets del bootloader inesperados\n");
        return -1;
    }
    if (m->bootloaderCodeOffset >= m->sizeOfBootloader) { printf("FALLO: code fuera\n"); return -1; }
    /* Todo lo que debe fijar el FMC sigue a cero — lo comprueba también
     * wpr_meta_verify(); aquí se repite por si esa comprobación se relaja. */
    if (m->gspFwWprStart || m->gspFwOffset || m->frtsOffset || m->fbSize) {
        printf("FALLO: campos que fija el FMC vienen escritos\n");
        return -1;
    }
    if (m->pmuReservedSize != 0x1820000u) { printf("FALLO: pmuReservedSize\n"); return -1; }
    printf("OK: WPR meta 256 B, heap %llu MiB, offsets del bootloader correctos\n",
           (unsigned long long)(m->gspFwHeapSize >> 20));

    *keep = w;   /* el paso 5 lo necesita vivo */
    return 0;
}

/* Layout Ampere (`tu102_gsp_oneinit`): offsets en FB, no los ceros del FMC. */
static int check_wpr_ampere(const struct gsp_rm_fw *rm)
{
    struct gsp_wpr_fb_layout L;
    struct gsp_wpr w;
    uint64_t fb4 = 4ull << 30;
    uint64_t heap4;
    const struct gsp_wpr_meta *m;

    heap4 = gsp_wpr_heap_size_ampere(fb4);
    if (gsp_wpr_layout_ampere(fb4, 0x31000ull, 0x3c99000ull, heap4, 0, &L) != 0) {
        printf("FALLO: layout Ampere 4 GiB\n");
        return -1;
    }
    if (L.vga_addr != fb4 - 0x100000ull || L.frts_size != 0x100000ull) {
        printf("FALLO: VGA/FRTS Ampere\n");
        return -1;
    }
    if (L.wpr_end != (L.vga_addr & ~0x1ffffull) || L.wpr_end > L.vga_addr) {
        printf("FALLO: wpr_end Ampere 0x%llx\n", (unsigned long long)L.wpr_end);
        return -1;
    }
    if (L.nonwpr_size != 0x100000ull ||
        L.nonwpr_addr + L.nonwpr_size != L.wpr_start) {
        printf("FALLO: nonWpr Ampere\n");
        return -1;
    }
    if (!(L.wpr_start <= L.heap_addr &&
          L.heap_addr + L.heap_size <= L.elf_addr &&
          L.elf_addr + L.elf_size <= L.boot_addr &&
          L.boot_addr + L.boot_size <= L.frts_addr &&
          L.frts_addr + L.frts_size == L.wpr_end &&
          L.wpr_end <= L.vga_addr &&
          L.vga_addr + L.vga_size == fb4)) {
        printf("FALLO: anidado Ampere\n");
        return -1;
    }
    printf("OK: layout Ampere 4 GiB wpr=[0x%llx,0x%llx) heap=%llu MiB\n",
           (unsigned long long)L.wpr_start, (unsigned long long)L.wpr_end,
           (unsigned long long)(L.heap_size >> 20));

    if (gsp_wpr_prepare_ampere(rm, &w) != 0) {
        printf("FALLO: gsp_wpr_prepare_ampere\n");
        return -1;
    }
    m = w.meta;
    if (!m->gspFwWprStart || !m->gspFwWprEnd || !m->fbSize || !m->frtsOffset ||
        !m->gspFwOffset || !m->bootBinOffset) {
        printf("FALLO: Ampere WPR offsets a cero\n");
        gsp_wpr_release(&w);
        return -1;
    }
    if (m->gspFwWprEnd > m->fbSize || m->gspFwWprStart >= m->gspFwWprEnd) {
        printf("FALLO: Ampere WPR fuera de FB\n");
        gsp_wpr_release(&w);
        return -1;
    }
    if (m->nonWprHeapSize != 0x100000ull) {
        printf("FALLO: Ampere nonWpr %llu\n", (unsigned long long)m->nonWprHeapSize);
        gsp_wpr_release(&w);
        return -1;
    }
    if (m->pmuReservedSize != 0) {
        printf("FALLO: Ampere pmuReservedSize=0x%llx (esperaba 0)\n",
               (unsigned long long)m->pmuReservedSize);
        gsp_wpr_release(&w);
        return -1;
    }
    /* La ruta FMC tiene que seguir con ceros: este meta es otro bloque. */
    printf("OK: WPR Ampere offsets en FB (%u MiB) heap=%llu MiB wpr=[0x%llx,0x%llx)\n",
           (unsigned)(m->fbSize >> 20),
           (unsigned long long)(m->gspFwHeapSize >> 20),
           (unsigned long long)m->gspFwWprStart,
           (unsigned long long)m->gspFwWprEnd);
    gsp_wpr_release(&w);
    return 0;
}

/* Paso 5: colas compartidas, logs, RMARGS, boot params y staging del FMC. */
static int check_libos(const struct gsp_wpr *wpr)
{
    struct gsp_libos lo;
    struct fmc_image img;
    struct fmc_staged staged;
    const struct gsp_fw_blob *fmc = gsp_fw_get(GSP_FW_FMC);

    if (gsp_libos_prepare(wpr, &lo) != 0) { printf("FALLO: gsp_libos_prepare\n"); return -1; }

    /* 128 páginas de colas + 1 de tabla, y la tabla se describe a sí misma. */
    if (lo.shm_ptes_nr != 129 || lo.shm_ptes_size != 4096) {
        printf("FALLO: PTEs de la shm (%u, %lu)\n", lo.shm_ptes_nr, lo.shm_ptes_size);
        return -1;
    }
    if (lo.shm.size != 4096 + 2 * 0x40000ul) { printf("FALLO: tamaño shm\n"); return -1; }
    if (lo.cmdq_offset != 4096 || lo.msgq_offset != 4096 + 0x40000ul) {
        printf("FALLO: offsets de las colas\n"); return -1;
    }
    /* Cada PTE de la shm describe su propia página. */
    const uint64_t *ptes = lo.shm.va;
    for (unsigned i = 0; i < lo.shm_ptes_nr; i++) {
        if (ptes[i] != lo.shm.phys + ((uint64_t)i << 12)) { printf("FALLO: PTE shm %u\n", i); return -1; }
    }
    /* "LOGINIT" empaquetado big-endian en un u64. */
    const struct gsp_libos_region *r = lo.libos.va;
    if (r[0].id8 != 0x4c4f47494e4954ull) {
        printf("FALLO: id8 LOGINIT = 0x%llx\n", (unsigned long long)r[0].id8);
        return -1;
    }
    if (r[3].pa != lo.rmargs.phys) { printf("FALLO: región RMARGS\n"); return -1; }
    const struct gsp_fmc_boot_params *p = lo.boot_params.va;
    if (p->bootGspRmParams.gspRmDescOffset != wpr->meta_phys ||
        p->gspRmParams.bootArgsOffset != lo.libos.phys) {
        printf("FALLO: boot params no enlazan\n"); return -1;
    }
    printf("OK: libos 4 regiones, colas 2x256 KiB (%u PTEs), boot params enlazados\n",
           lo.shm_ptes_nr);

    /* Staging del FMC: parseo del ELF firmado + copias a memoria DMA. */
    if (!fmc || fmc_lx_parse(fmc->data, fmc->len, &img) != 0) {
        printf("FALLO: fmc_lx_parse\n"); return -1;
    }
    if (fmc_lx_verify_sizes(&img) != 0) { printf("FALLO: tamaños COT gb20x\n"); return -1; }
    if (fmc_lx_stage(&img, &staged) != 0) { printf("FALLO: fmc_lx_stage\n"); return -1; }
    if (staged.img.size != img.image_len || staged.hash.size != 48 ||
        staged.pkey.size != 97 || staged.sig.size != 96) {
        printf("FALLO: tamaños stageados\n"); return -1;
    }
    if (memcmp(staged.img.va, img.image, img.image_len)) {
        printf("FALLO: la imagen FMC copiada no coincide\n"); return -1;
    }
    printf("OK: FMC stageado (imagen %lu B + 48/97/96)\n", staged.img.size);

    fmc_lx_stage_release(&staged);
    gsp_libos_release(&lo);
    if (lo.shm.va || lo.boot_params.va || staged.img.va) {
        printf("FALLO: release deja punteros\n"); return -1;
    }
    printf("OK: release limpio\n");
    return 0;
}

/* Deja un mensaje de GSP-RM en la página `idx` del anillo, con `plen` bytes de
 * payload detrás de las cabeceras. Escribe dando la vuelta al anillo igual que
 * lo haría el firmware, para poder ejercitar ese caso. */
static void fake_rpc_post_payload(const struct gsp_libos *lo, unsigned idx,
                                  uint32_t fn, uint32_t result,
                                  const unsigned char *payload, uint32_t plen)
{
    unsigned char *entries = (unsigned char *)lo->shm.va + lo->msgq_offset + 4096;
    unsigned long ring = 63ul * 4096ul;
    unsigned long pos = (unsigned long)idx * 4096ul;
    struct gsp_rpc_hdr hdr;
    unsigned char zero[4096];
    uint32_t total = (uint32_t)sizeof(struct gsp_msg_elem) + sizeof(hdr) + plen;
    uint32_t pages = (total + 4095u) / 4096u;
    unsigned p;
    unsigned long w;
    uint32_t i;

    memset(zero, 0, sizeof(zero));
    for (p = 0; p < pages; p++) {
        memcpy(entries + ((pos + (unsigned long)p * 4096ul) % ring), zero, 4096);
    }

    memset(&hdr, 0, sizeof(hdr));
    hdr.length = (uint32_t)sizeof(hdr) + plen;
    hdr.function = fn;
    hdr.rpc_result = result;

    /* Byte a byte para no tener que partir cada memcpy en el borde del anillo. */
    w = (pos + sizeof(struct gsp_msg_elem)) % ring;
    for (i = 0; i < sizeof(hdr); i++) {
        entries[(w + i) % ring] = ((const unsigned char *)&hdr)[i];
    }
    w = (w + sizeof(hdr)) % ring;
    for (i = 0; i < plen; i++) {
        entries[(w + i) % ring] = payload ? payload[i] : 0;
    }
}

static void fake_rpc_post(const struct gsp_libos *lo, unsigned idx, uint32_t fn,
                          uint32_t result)
{
    fake_rpc_post_payload(lo, idx, fn, result, NULL, 0);
}

/* Recepción de RPCs de GSP-RM sobre las colas del paso 5. */
static int check_rpc(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: gsp_rpc_init\n"); return -1; }
    if (rpc.cnt != 63) { printf("FALLO: cnt=%u\n", rpc.cnt); return -1; }
    if (gsp_rpc_start(0) != 0) { printf("FALLO: gsp_rpc_start\n"); return -1; }

    /* Un evento cualquiera por delante del que esperamos: debe consumirlo. */
    fake_rpc_post(lo, 0, NV_VGPU_MSG_EVENT_UCODE_LIBOS_PRINT, 0);
    fake_rpc_post(lo, 1, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 0);
    msgq->tx.writePtr = 2;

    if (gsp_rpc_wait_event(&rpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 100) != 0) {
        printf("FALLO: no vio GSP_INIT_DONE\n");
        return -1;
    }
    if (*rpc.rptr != 2) { printf("FALLO: rptr=%u tras consumir 2\n", *rpc.rptr); return -1; }
    printf("OK: RPC consume el evento previo y ve GSP_INIT_DONE (rptr=%u)\n", *rpc.rptr);

    /* Un rpc_result distinto de cero tiene que salir como error, no colarse. */
    fake_rpc_post(lo, 2, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 0x55);
    msgq->tx.writePtr = 3;
    if (gsp_rpc_wait_event(&rpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 100) == 0) {
        printf("FALLO: un RPC con error se dio por bueno\n");
        return -1;
    }
    printf("OK: un RPC con rpc_result != 0 se detecta\n");

    /* Y sin mensajes, el tiempo se agota en vez de inventarse uno. */
    if (gsp_rpc_wait_event(&rpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 5) == 0) {
        printf("FALLO: dio por recibido un mensaje que no existe\n");
        return -1;
    }
    printf("OK: sin mensajes, expira\n");
    return 0;
}

/* Llamada síncrona (G4b): payload de respuesta y anillo circular.
 *
 * Estos dos casos no los tocaba nada: `GSP_INIT_DONE` mide 32 B y no lleva
 * payload, así que ni la copia ni el borde del anillo se ejercitaban. Una
 * respuesta de RM sí puede ocupar varias páginas y envolver. */
static int check_rpc_sync(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_cmdq q;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);
    unsigned char pattern[6000];
    unsigned char got[6000];
    uint32_t plen = 0;
    uint32_t status = 0xdeadbeefu;
    uint32_t i;

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: rpc_init (sync)\n"); return -1; }
    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: cmdq_init (sync)\n"); return -1; }

    for (i = 0; i < sizeof(pattern); i++) {
        pattern[i] = (unsigned char)(i * 7u + 3u);
    }

    /* 1) Payload corto, sin envolver: tiene que llegar entero. */
    *rpc.rptr = 10;
    fake_rpc_post_payload(lo, 10, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 0, pattern, 64);
    msgq->tx.writePtr = 11;
    memset(got, 0, sizeof(got));
    if (gsp_rpc_recv(&rpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, got, sizeof(got),
                     &plen, &status, 100) != 0) {
        printf("FALLO: recv con payload\n");
        return -1;
    }
    if (plen != 64 || status != 0 || memcmp(got, pattern, 64) != 0) {
        printf("FALLO: payload corto (len=%u status=0x%x)\n", plen, status);
        return -1;
    }
    printf("OK: recv copia el payload (%u B) y expone el status\n", plen);

    /* 2) El caso que importa: mensaje de 2 páginas que empieza en la última
     * entrada del anillo y continúa en la primera. */
    *rpc.rptr = 62;
    fake_rpc_post_payload(lo, 62, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 0, pattern,
                          (uint32_t)sizeof(pattern));
    msgq->tx.writePtr = 1;
    memset(got, 0, sizeof(got));
    if (gsp_rpc_recv(&rpc, NV_VGPU_MSG_EVENT_GSP_INIT_DONE, got, sizeof(got),
                     &plen, &status, 100) != 0) {
        printf("FALLO: recv de un mensaje que envuelve\n");
        return -1;
    }
    if (plen != sizeof(pattern) || memcmp(got, pattern, sizeof(pattern)) != 0) {
        printf("FALLO: payload envuelto corrupto (len=%u)\n", plen);
        for (i = 0; i < sizeof(pattern); i++) {
            if (got[i] != pattern[i]) {
                printf("       primer byte mal: %u (0x%02x != 0x%02x)\n",
                       i, got[i], pattern[i]);
                break;
            }
        }
        return -1;
    }
    if (*rpc.rptr != 1) { printf("FALLO: rptr=%u tras envolver\n", *rpc.rptr); return -1; }
    printf("OK: mensaje de 2 páginas que envuelve el anillo, %u B intactos (rptr=%u)\n",
           plen, *rpc.rptr);

    /* 3) La llamada completa: encola y consume su respuesta. */
    fake_rpc_post_payload(lo, 1, 72u, 0, pattern, 128);
    msgq->tx.writePtr = 2;
    memset(got, 0, sizeof(got));
    if (gsp_cmdq_call(&q, &rpc, 72u, NULL, 0, got, sizeof(got), &plen,
                      &status, 100) != 0) {
        printf("FALLO: gsp_cmdq_call\n");
        return -1;
    }
    if (plen != 128 || memcmp(got, pattern, 128) != 0) {
        printf("FALLO: respuesta de la llamada (len=%u)\n", plen);
        return -1;
    }
    printf("OK: gsp_cmdq_call encola y recoge su respuesta (%u B)\n", plen);

    /* 4) Un NV_STATUS de error tiene que salir como fallo, pero legible. */
    *rpc.rptr = 5;
    fake_rpc_post_payload(lo, 5, 72u, 0x56u, NULL, 0);
    msgq->tx.writePtr = 6;
    status = 0;
    if (gsp_cmdq_call(&q, &rpc, 72u, NULL, 0, NULL, 0, NULL, &status, 100) == 0) {
        printf("FALLO: una respuesta con NV_STATUS != 0 se dio por buena\n");
        return -1;
    }
    if (status != 0x56u) {
        printf("FALLO: status no propagado (0x%x)\n", status);
        return -1;
    }
    printf("OK: NV_STATUS de error se propaga (0x%x = NOT_SUPPORTED)\n", status);
    return 0;
}

/* G4c: la cadena cliente → device → subdevice.
 *
 * El banco es de un solo hilo, así que no puede "reaccionar" a cada push: se
 * pre-encolan las tres respuestas y las tres llamadas las consumen en orden. Lo
 * que de verdad se valida son las **peticiones** que quedan en la cmdq, que es
 * lo que verá RM: clase, padre y tamaño de parámetros de cada una. */
static int check_rm_objects(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_cmdq q;
    struct gsp_rm rm;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);
    unsigned char *cmdq_base = (unsigned char *)lo->shm.va + lo->cmdq_offset;
    rpc_gsp_rm_alloc ok;
    uint32_t base;
    uint32_t wptr0;
    unsigned i;
    /* Lo que debe pedir, en orden: clase, padre y tamaño de params. */
    const struct { uint32_t cls; uint32_t parent; uint32_t psize; const char *name; } want[3] = {
        { NV01_ROOT,        NVKM_RM_CLIENT(0), (uint32_t)sizeof(NV0000_ALLOC_PARAMETERS), "cliente" },
        { NV01_DEVICE_0,    NVKM_RM_CLIENT(0), (uint32_t)sizeof(NV0080_ALLOC_PARAMETERS), "device" },
        { NV20_SUBDEVICE_0, NVKM_RM_DEVICE,    (uint32_t)sizeof(NV2080_ALLOC_PARAMETERS), "subdevice" },
    };

    printf("sizeof rpc_gsp_rm_alloc=%zu rm_control=%zu NV0000=%zu NV0080=%zu NV2080=%zu\n",
           sizeof(rpc_gsp_rm_alloc), sizeof(rpc_gsp_rm_control),
           sizeof(NV0000_ALLOC_PARAMETERS), sizeof(NV0080_ALLOC_PARAMETERS),
           sizeof(NV2080_ALLOC_PARAMETERS));

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: rpc_init (rm)\n"); return -1; }
    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: cmdq_init (rm)\n"); return -1; }

    /* Tres respuestas OK seguidas, a partir de donde esté el puntero. */
    memset(&ok, 0, sizeof(ok));
    ok.status = 0;
    base = *rpc.rptr;
    for (i = 0; i < 3; i++) {
        fake_rpc_post_payload(lo, (base + i) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                              0, (const unsigned char *)&ok, (uint32_t)sizeof(ok));
    }
    msgq->tx.writePtr = (base + 3) % 63;

    wptr0 = *q.wptr;
    if (gsp_rm_init(&q, &rpc, &rm) != 0) {
        printf("FALLO: gsp_rm_init\n");
        return -1;
    }
    if (!rm.ready || rm.client != NVKM_RM_CLIENT(0) || rm.device != NVKM_RM_DEVICE ||
        rm.subdevice != NVKM_RM_SUBDEVICE) {
        printf("FALLO: handles cli=0x%08x dev=0x%08x sub=0x%08x\n",
               rm.client, rm.device, rm.subdevice);
        return -1;
    }

    /* Releer las tres peticiones encoladas y comprobarlas una a una. */
    for (i = 0; i < 3; i++) {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + i) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_alloc *a = (const rpc_gsp_rm_alloc *)(hdr + 1);

        if (hdr->function != NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC) {
            printf("FALLO: %s con function=%u\n", want[i].name, hdr->function);
            return -1;
        }
        if (a->hClass != want[i].cls) {
            printf("FALLO: %s hClass=0x%x (esperaba 0x%x)\n",
                   want[i].name, a->hClass, want[i].cls);
            return -1;
        }
        if (a->hParent != want[i].parent) {
            printf("FALLO: %s hParent=0x%08x (esperaba 0x%08x)\n",
                   want[i].name, a->hParent, want[i].parent);
            return -1;
        }
        if (a->paramsSize != want[i].psize) {
            printf("FALLO: %s paramsSize=%u (esperaba %u)\n",
                   want[i].name, a->paramsSize, want[i].psize);
            return -1;
        }
        if (a->hClient != NVKM_RM_CLIENT(0)) {
            printf("FALLO: %s hClient=0x%08x\n", want[i].name, a->hClient);
            return -1;
        }
        if (hdr->length != sizeof(*hdr) + sizeof(*a) + want[i].psize) {
            printf("FALLO: %s length=%u\n", want[i].name, hdr->length);
            return -1;
        }
    }
    /* El cliente lleva su propio handle dentro de los parámetros y processID=~0. */
    {
        const unsigned char *entry = cmdq_base + 4096 + (unsigned long)(wptr0 % 63) * 4096;
        const rpc_gsp_rm_alloc *a = (const rpc_gsp_rm_alloc *)
            (entry + sizeof(struct gsp_msg_elem) + sizeof(struct gsp_rpc_hdr));
        const NV0000_ALLOC_PARAMETERS *p = (const NV0000_ALLOC_PARAMETERS *)(a + 1);

        if (p->hClient != NVKM_RM_CLIENT(0) || p->processID != 0xffffffffu) {
            printf("FALLO: params del cliente hClient=0x%08x pid=0x%x\n",
                   p->hClient, p->processID);
            return -1;
        }
    }
    printf("OK: cadena RM cliente→device→subdevice (clases 0x0/0x80/0x2080, "
           "params %u/%u/%u B)\n", want[0].psize, want[1].psize, want[2].psize);

    /* Y un NV_STATUS dentro del wrapper tiene que salir como fallo — es el que
     * llega con el RPC impecable y un error de RM dentro. */
    memset(&ok, 0, sizeof(ok));
    /* 0x22, que es INVALID_CLASS de verdad. Aquí ponía 0x2b siguiendo la tabla
     * de nombres del port, que lo llamaba así y no lo es: el 0x2b es
     * INVALID_HEAP. El valor daba igual para lo que prueba este caso —que un
     * NV_STATUS dentro de un wrapper impecable se detecte— pero un nombre falso
     * en una prueba se copia luego a un diagnóstico. */
    ok.status = 0x22u;   /* INVALID_CLASS */
    base = *rpc.rptr;
    fake_rpc_post_payload(lo, base % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC, 0,
                          (const unsigned char *)&ok, (uint32_t)sizeof(ok));
    msgq->tx.writePtr = (base + 1) % 63;
    {
        uint32_t st = 0;
        NV2080_ALLOC_PARAMETERS sub;

        memset(&sub, 0, sizeof(sub));
        if (gsp_rm_alloc(&rm, rm.device, 0x5d1d0001u, NV20_SUBDEVICE_0, &sub,
                         (uint32_t)sizeof(sub), &st) == 0) {
            printf("FALLO: un NV_STATUS de RM se dio por bueno\n");
            return -1;
        }
        if (st != 0x22u) {
            printf("FALLO: NV_STATUS del wrapper no propagado (0x%x)\n", st);
            return -1;
        }
    }
    printf("OK: el NV_STATUS de dentro del wrapper se detecta (0x22 = INVALID_CLASS)\n");

    /* G4d: GET_GSP_STATIC_INFO. Se fabrica una respuesta con VRAM y nombre
     * conocidos y se comprueba que los campos se leen de donde deben — que es
     * justo lo que no se puede dar por hecho en una transcripción a mano. */
    {
        GspStaticConfigInfo *fake = calloc(1, sizeof(*fake));
        struct gsp_static_info si;
        const uint64_t vram = 12227ull * 1024ull * 1024ull;

        if (!fake) { printf("FALLO: sin memoria\n"); return -1; }
        printf("sizeof(GspStaticConfigInfo) = %zu\n", sizeof(*fake));

        fake->fb_length = vram;
        fake->l2_cache_size = 32u * 1024u;
        fake->bar1PdeBase = 0x1234000ull;
        fake->bar2PdeBase = 0x5678000ull;
        fake->hInternalClient = 0xc1d00001u;
        fake->hInternalDevice = 0xde1d0001u;
        fake->hInternalSubdevice = 0x5d1d0001u;
        memcpy(fake->gpuNameString, "NVIDIA GeForce RTX 5070", 24);
        /* Dos regiones: una utilizable y una reservada que debe descartarse. */
        fake->fbRegionInfoParams.numFBRegions = 2;
        fake->fbRegionInfoParams.fbRegion[0].base = 0;
        fake->fbRegionInfoParams.fbRegion[0].limit = (2048ull << 20) - 1;
        fake->fbRegionInfoParams.fbRegion[1].base = 2048ull << 20;
        fake->fbRegionInfoParams.fbRegion[1].limit = (2049ull << 20) - 1;
        fake->fbRegionInfoParams.fbRegion[1].reserved = 1;

        base = *rpc.rptr;
        fake_rpc_post_payload(lo, base % 63, NV_VGPU_MSG_FUNCTION_GET_GSP_STATIC_INFO,
                              0, (const unsigned char *)fake, (uint32_t)sizeof(*fake));
        msgq->tx.writePtr = (base + ((sizeof(*fake) + 80 + 4095) / 4096)) % 63;

        wptr0 = *q.wptr;
        if (gsp_static_info_get(&rm, vram, &si) != 0) {
            printf("FALLO: gsp_static_info_get\n");
            free(fake);
            return -1;
        }

        /* La petición NO puede ir pelada. Upstream la manda con el struct
         * entero de payload (`nvkm_gsp_rpc_rd(gsp, fn, sizeof(*rpc))` →
         * `rpc->length = 32 + 1656`), y RM lo usa de hueco para la respuesta.
         * Mandándola con length=32 el transporte la rechaza con
         * `0xff100002 = RPC_INVALID_MESSAGE_FORMAT`, que es lo que pasó en el
         * HW el 2026-07-25 y dejó G4d bloqueado. */
        {
            const unsigned char *entry = cmdq_base + 4096 +
                                         (unsigned long)(wptr0 % 63) * 4096;
            const struct gsp_rpc_hdr *hdr =
                (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
            uint32_t want = (uint32_t)(sizeof(*hdr) + sizeof(*fake));

            if (hdr->function != NV_VGPU_MSG_FUNCTION_GET_GSP_STATIC_INFO) {
                printf("FALLO: static info pedido con function=%u\n", hdr->function);
                free(fake);
                return -1;
            }
            if (hdr->length != want) {
                printf("FALLO: static info pedido con length=%u, esperaba %u "
                       "(pelado = 0xff100002 RPC_INVALID_MESSAGE_FORMAT)\n",
                       hdr->length, want);
                free(fake);
                return -1;
            }
            printf("OK: GET_GSP_STATIC_INFO se pide con %u B (cabecera + hueco "
                   "para la respuesta), no pelado\n", hdr->length);
        }
        if (si.fb_length != vram || si.region_nr != 1 ||
            si.usable_bytes != (2048ull << 20) ||
            si.bar1_pde_base != 0x1234000ull || si.bar2_pde_base != 0x5678000ull ||
            si.internal_client != 0xc1d00001u || si.internal_subdevice != 0x5d1d0001u ||
            strcmp(si.name, "NVIDIA GeForce RTX 5070") != 0) {
            printf("FALLO: campos mal leídos — vram=0x%llx regs=%u usable=0x%llx "
                   "bar1=0x%llx cli=0x%08x name='%s'\n",
                   (unsigned long long)si.fb_length, si.region_nr,
                   (unsigned long long)si.usable_bytes,
                   (unsigned long long)si.bar1_pde_base, si.internal_client, si.name);
            free(fake);
            return -1;
        }
        printf("OK: static info — '%s' %llu MiB, 1 de 2 regiones utilizable, "
               "handles y bases de PDE en su offset\n",
               si.name, (unsigned long long)(si.fb_length >> 20));

        /* Y el contraste: si la VRAM no cuadra, hay que rechazarlo, no seguir. */
        fake->fb_length = vram + 4096;
        base = *rpc.rptr;
        fake_rpc_post_payload(lo, base % 63, NV_VGPU_MSG_FUNCTION_GET_GSP_STATIC_INFO,
                              0, (const unsigned char *)fake, (uint32_t)sizeof(*fake));
        msgq->tx.writePtr = (base + ((sizeof(*fake) + 80 + 4095) / 4096)) % 63;
        if (gsp_static_info_get(&rm, vram, &si) == 0) {
            printf("FALLO: una fb_length que no cuadra se dio por buena\n");
            free(fake);
            return -1;
        }
        printf("OK: fb_length que no cuadra con la VRAM conocida se rechaza\n");
        free(fake);
    }
    return 0;
}

/* Envío por la cmdq: las dos RPCs que GSP-RM consume durante su init. */
/* G4: el apagado (`gsp_fini`).
 *
 * Lo que se valida son las **peticiones** que quedan en la cmdq, que es lo que
 * vería GSP-RM: tres FREE en orden inverso al de la reserva y el aviso de
 * descarga. Importa especialmente el número de función: `FREE` valía 27 en este
 * árbol (= `DMA_FILL_PTE_MEM`), y con el 27 esta prueba fallaría en el primer
 * `hdr->function`. Y el bus master tiene que quedar quitado pase lo que pase,
 * que es lo único que de verdad protegía al host el día del cuelgue. */
/* G4d (2/2): el vaspace, el directorio de páginas y la codificación VER3.
 *
 * Lo que aporta este banco sobre el `gsp_vmm_translate` que ya corre en el
 * arranque: `translate` recorre las tablas con las mismas rutinas que las
 * escribió, así que un PTE mal codificado —el bit que sobra, la apertura del
 * PDE puesta con los valores del PTE— le cuadraría igual. Aquí los valores
 * están escritos a mano, sacados de dev_mmu.h campo a campo. */
/* La misma que usa el bring-up — derivada, no copiada. El comentario decía "la
 * misma que usa el bring-up" y era verdad sólo mientras nadie la moviera: cuando
 * `GSP_VA_BASE` bajó a 512 GiB por el techo de 40 bits del GPFIFO, esta se quedó
 * en 1 TiB y el banco siguió probando la VA que ya no se usa. */
#define VMM_T_VA        GSP_VA_BASE
#define VMM_T_VRAM_SZ   (2ull * 1024ull * 1024ull)
#define VMM_T_VRAM_PA   0x0000000240000000ull   /* 9 GiB, alineado a 2 MiB */

/* PTE de 4 KiB: VALID(bit 0) | APERTURE(2:1) | PCF(7:3) | KIND(11:8) | ADDR(51:12).
 * VRAM   → aper 0, pcf 0x10 (REGULAR_RW_ATOMIC_CACHED_ACD)   → 0x81
 * sysmem → aper 2, pcf 0x11 (REGULAR_RW_ATOMIC_UNCACHED_ACD) → 0x8d */
#define VMM_T_PTE_VRAM_LOW   0x81ull
#define VMM_T_PTE_SYS_LOW    0x8dull
/* Y los de sólo lectura, que sólo cambian el bit 2 del PCF (0x10→0x14, 0x11→0x15):
 * VRAM RO → 0xa1, sysmem RO → 0xad. Los números están puestos a mano a propósito;
 * derivarlos con la misma expresión que el código haría que un PCF mal elegido
 * pasara la prueba. */
#define VMM_T_PTE_VRAM_RO_LOW 0xa1ull
#define VMM_T_PTE_SYS_RO_LOW  0xadull
/* PDE: **bit 0 a cero** (ahí vive IS_PTE, no VALID) | APERTURE(2:1) | PCF(5:3).
 * sysmem coherente → aper 2, pcf 1 (VALID_UNCACHED_ATS_ALLOWED) → 0xc */
#define VMM_T_PDE_SYS_LOW    0x0cull
#define VMM_T_ADDR_MASK      0x000ffffffffff000ull

/* gp100 Ampere: (phys>>4)|type; VRAM solo VALID=1; sysmem HOST|VOL → 0x0d en bajos. */
#define VMM_T_GP100_PTE_VRAM_LOW  0x01ull
#define VMM_T_GP100_PTE_SYS_LOW   0x0dull
#define VMM_T_GP100_PTE_VRAM_RO   0x41ull
#define VMM_T_GP100_ADDR_MASK     0x000000fffffffff0ull

/* GA107 Ampere: numEntries=4 (gp100), raíz en VRAM + aperture VIDMEM. */
static int check_vmm_ampere(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_cmdq q;
    struct gsp_vmm v;
    struct gsp_vram pool;
    struct gsp_static_info si;
    struct gsp_dma_buf scratch;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);
    unsigned char *cmdq_base = (unsigned char *)lo->shm.va + lo->cmdq_offset;
    rpc_gsp_rm_alloc alloc_ok;
    unsigned char ctrl_ok[sizeof(rpc_gsp_rm_control)];
    uint32_t base, wptr0;
    uint64_t root_phys, pte = 0;

    gsp_nv_family_set(0xb74000a1u, 0x249cu);

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: rpc_init (vmm ampere)\n"); return -1; }
    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: cmdq_init (vmm ampere)\n"); return -1; }

    memset(&si, 0, sizeof(si));
    si.ready = 1;
    si.region_nr = 1;
    si.region[0].base = 0ull;
    si.region[0].size = 16ull * 1024ull * 1024ull;
    if (gsp_vram_init(&pool, &si) != 0) {
        printf("FALLO: gsp_vram_init (vmm ampere)\n");
        return -1;
    }

    memset(&alloc_ok, 0, sizeof(alloc_ok));
    memset(ctrl_ok, 0, sizeof(ctrl_ok));
    base = *rpc.rptr;
    for (unsigned i = 0; i < 4; i++) {
        fake_rpc_post_payload(lo, (base + i) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                              0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    }
    fake_rpc_post_payload(lo, (base + 4) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    msgq->tx.writePtr = (base + 5) % 63;

    wptr0 = *q.wptr;
    if (gsp_vmm_init(&q, &rpc, &v, &pool, 1u) != 0) {
        printf("FALLO: gsp_vmm_init (Ampere)\n");
        return -1;
    }
    if (v.fmt != GSP_VMM_FMT_GP100 || v.root_entries != 4 || !v.pt[0].in_vram) {
        printf("FALLO: VMM Ampere fmt=%u root_entries=%u in_vram=%d "
               "(esperaba gp100/4/1)\n",
               (unsigned)v.fmt, v.root_entries, v.pt[0].in_vram);
        return -1;
    }
    root_phys = v.pt[0].mem.phys;
    {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 4) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_control *c = (const rpc_gsp_rm_control *)(hdr + 1);
        const NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS *p =
            (const NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS *)(c + 1);

        if (p->numEntries != 4) {
            printf("FALLO: Ampere numEntries=%u (esperaba 4)\n", p->numEntries);
            return -1;
        }
        if (p->flags != NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_FLAGS_APERTURE_VIDMEM) {
            printf("FALLO: Ampere aperture=%u (esperaba VIDMEM=0)\n", p->flags);
            return -1;
        }
        if (p->physAddress != root_phys) {
            printf("FALLO: Ampere directorio 0x%llx raíz 0x%llx\n",
                   (unsigned long long)p->physAddress,
                   (unsigned long long)root_phys);
            return -1;
        }
    }
    if (gsp_vmm_pte_encode(0x240000000ull, GSP_VMM_VRAM, 0) !=
        (VMM_T_GP100_PTE_VRAM_LOW | (0x240000000ull >> 4))) {
        printf("FALLO: gp100 PTE VRAM codificación\n");
        return -1;
    }
    if (gsp_vmm_pte_encode(0x2000000ull, GSP_VMM_SYSMEM, 0) !=
        (VMM_T_GP100_PTE_SYS_LOW | (0x2000000ull >> 4))) {
        printf("FALLO: gp100 PTE sysmem codificación\n");
        return -1;
    }
    if (gsp_dma_alloc(&scratch, 4096, "scratch ampere") != 0) {
        printf("FALLO: scratch ampere\n");
        return -1;
    }
    if (gsp_vmm_map(&v, VMM_T_VA, VMM_T_VRAM_PA, VMM_T_VRAM_SZ, GSP_VMM_VRAM) != 0 ||
        gsp_vmm_map(&v, VMM_T_VA + VMM_T_VRAM_SZ, scratch.phys, 4096,
                    GSP_VMM_SYSMEM) != 0) {
        printf("FALLO: gsp_vmm_map (Ampere)\n");
        return -1;
    }
    {
        uint32_t want_pdb = (uint32_t)(root_phys >> 8);

        if (fake_mmu_inval_pdb != want_pdb) {
            printf("FALLO: Ampere MMU INVALIDATE_PDB=0x%08x (esperaba 0x%08x, "
                   "sin aperture SYS)\n",
                   fake_mmu_inval_pdb, want_pdb);
            return -1;
        }
    }
    if (gsp_vmm_translate(&v, VMM_T_VA, NULL, &pte) != 0 ||
        (pte & ~VMM_T_GP100_ADDR_MASK) != VMM_T_GP100_PTE_VRAM_LOW) {
        printf("FALLO: gp100 translate VRAM pte=0x%llx\n", (unsigned long long)pte);
        return -1;
    }
    gsp_dma_free(&scratch);
    gsp_vmm_fini(&v);
    printf("OK: SET_PAGE_DIRECTORY Ampere raíz=0x%llx VIDMEM 4 entradas gp100\n",
           (unsigned long long)root_phys);
    return 0;
}

static int check_vmm(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_cmdq q;
    struct gsp_vmm v;
    struct gsp_vram pool;
    struct gsp_static_info si;
    struct gsp_dma_buf scratch;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);
    unsigned char *cmdq_base = (unsigned char *)lo->shm.va + lo->cmdq_offset;
    rpc_gsp_rm_alloc alloc_ok;
    unsigned char ctrl_ok[sizeof(rpc_gsp_rm_control)];
    uint32_t base, wptr0;
    uint64_t root_phys, phys = 0, pte = 0;
    unsigned i;

    printf("sizeof NV_VASPACE_ALLOCATION_PARAMETERS=%zu SET_PAGE_DIRECTORY=%zu\n",
           sizeof(NV_VASPACE_ALLOCATION_PARAMETERS),
           sizeof(NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS));

    /* Blackwell VER3: raíz con 2 entradas. */
    gsp_nv_family_set(0x1b5000a1u, 0x2f18u);

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: rpc_init (vmm)\n"); return -1; }
    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: cmdq_init (vmm)\n"); return -1; }

    /* Cuatro RM_ALLOC (cliente, device, subdevice, vaspace) y un RM_CONTROL. */
    memset(&alloc_ok, 0, sizeof(alloc_ok));
    memset(ctrl_ok, 0, sizeof(ctrl_ok));
    base = *rpc.rptr;
    for (i = 0; i < 4; i++) {
        fake_rpc_post_payload(lo, (base + i) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                              0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    }
    fake_rpc_post_payload(lo, (base + 4) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    msgq->tx.writePtr = (base + 5) % 63;

    wptr0 = *q.wptr;
    if (gsp_vmm_init(&q, &rpc, &v, NULL, 1u) != 0) {
        printf("FALLO: gsp_vmm_init\n");
        return -1;
    }
    if (!v.ready || !v.bound || v.vaspace != NVKM_RM_VASPACE) {
        printf("FALLO: vmm ready=%d bound=%d vaspace=0x%08x\n",
               v.ready, v.bound, v.vaspace);
        return -1;
    }
    /* Cliente propio, como en upstream — y el device puede repetir handle
     * porque los handles se cuentan por cliente. */
    if (v.rm.client != NVKM_RM_CLIENT(1) || v.rm.device != NVKM_RM_DEVICE) {
        printf("FALLO: el vaspace no cuelga de un cliente propio "
               "(cli=0x%08x dev=0x%08x)\n", v.rm.client, v.rm.device);
        return -1;
    }

    /* La cuarta petición encolada es la del vaspace. */
    {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 3) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_alloc *a = (const rpc_gsp_rm_alloc *)(hdr + 1);
        const NV_VASPACE_ALLOCATION_PARAMETERS *p =
            (const NV_VASPACE_ALLOCATION_PARAMETERS *)(a + 1);

        if (a->hClass != FERMI_VASPACE_A || a->hObject != NVKM_RM_VASPACE ||
            a->hParent != NVKM_RM_DEVICE || a->hClient != NVKM_RM_CLIENT(1)) {
            printf("FALLO: vaspace cls=0x%x obj=0x%08x padre=0x%08x cli=0x%08x\n",
                   a->hClass, a->hObject, a->hParent, a->hClient);
            return -1;
        }
        if (a->paramsSize != sizeof(*p)) {
            printf("FALLO: vaspace paramsSize=%u (esperaba %zu)\n",
                   a->paramsSize, sizeof(*p));
            return -1;
        }
        if (p->index != NV_VASPACE_ALLOCATION_INDEX_GPU_NEW ||
            p->flags != NV_VASPACE_ALLOCATION_FLAGS_IS_EXTERNALLY_OWNED) {
            printf("FALLO: vaspace index=%u flags=0x%x (esperaba 0/%u — sin "
                   "EXTERNALLY_OWNED las tablas las querría construir RM)\n",
                   p->index, p->flags,
                   NV_VASPACE_ALLOCATION_FLAGS_IS_EXTERNALLY_OWNED);
            return -1;
        }
        if (p->vaSize || p->vaBase || p->bigPageSize) {
            printf("FALLO: vaspace con geometría puesta (vaSize=%llu base=%llu "
                   "big=%u); upstream los deja a cero\n",
                   (unsigned long long)p->vaSize, (unsigned long long)p->vaBase,
                   p->bigPageSize);
            return -1;
        }
    }
    printf("OK: FERMI_VASPACE_A (0x%04x) externo, cliente propio 0x%08x\n",
           FERMI_VASPACE_A, NVKM_RM_CLIENT(1));

    /* Y la quinta, el control que entrega el directorio. */
    root_phys = v.pt[0].mem.phys;
    {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 4) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_control *c = (const rpc_gsp_rm_control *)(hdr + 1);
        const NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS *p =
            (const NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS *)(c + 1);

        if (hdr->function != NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL ||
            c->cmd != NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY) {
            printf("FALLO: SET_PAGE_DIRECTORY fn=%u cmd=0x%08x\n",
                   hdr->function, c->cmd);
            return -1;
        }
        if (c->hObject != NVKM_RM_DEVICE) {
            printf("FALLO: SET_PAGE_DIRECTORY sobre 0x%08x (va sobre el device)\n",
                   c->hObject);
            return -1;
        }
        if (p->physAddress != root_phys) {
            printf("FALLO: directorio en 0x%llx, la raíz está en 0x%llx\n",
                   (unsigned long long)p->physAddress,
                   (unsigned long long)root_phys);
            return -1;
        }
        if (p->numEntries != 2) {
            printf("FALLO: numEntries=%u; la raíz VER3 indexa con UN bit de la "
                   "VA, son 2 entradas y no 512\n", p->numEntries);
            return -1;
        }
        if (p->flags != NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_FLAGS_APERTURE_SYSMEM_COH) {
            printf("FALLO: aperture=%u; nuestro directorio vive en sysmem, no en "
                   "VRAM como el de upstream\n", p->flags);
            return -1;
        }
        if (p->hVASpace != NVKM_RM_VASPACE) {
            printf("FALLO: hVASpace=0x%08x\n", p->hVASpace);
            return -1;
        }
    }
    printf("OK: SET_PAGE_DIRECTORY raíz=0x%llx 2 entradas en sysmem coherente\n",
           (unsigned long long)root_phys);

    /* Mapear lo mismo que el bring-up y releerlo. */
    if (gsp_dma_alloc(&scratch, 4096, "scratch") != 0) {
        printf("FALLO: scratch\n");
        return -1;
    }
    if (gsp_vmm_map(&v, VMM_T_VA, VMM_T_VRAM_PA, VMM_T_VRAM_SZ, GSP_VMM_VRAM) != 0 ||
        gsp_vmm_map(&v, VMM_T_VA + VMM_T_VRAM_SZ, scratch.phys, 4096,
                    GSP_VMM_SYSMEM) != 0) {
        printf("FALLO: gsp_vmm_map\n");
        return -1;
    }
    {
        uint32_t want_pdb = (uint32_t)(root_phys >> 8);
        uint32_t want_upper = (uint32_t)(root_phys >> 40);

        if (fake_mmu_inval_pdb != want_pdb) {
            printf("FALLO: MMU INVALIDATE_PDB=0x%08x (esperaba 0x%08x)\n",
                   fake_mmu_inval_pdb, want_pdb);
            return -1;
        }
        if (fake_mmu_inval_upper != want_upper) {
            printf("FALLO: MMU INVALIDATE_UPPER_PDB=0x%08x (esperaba 0x%08x)\n",
                   fake_mmu_inval_upper, want_upper);
            return -1;
        }
        if (fake_mmu_inval != 0x1u) {
            printf("FALLO: MMU INVALIDATE=0x%08x (esperaba 0x1 ALL_VA sin trigger)\n",
                   fake_mmu_inval);
            return -1;
        }
    }
    printf("OK: MMU invalidate tras mapeo (PDB sysmem 0x%08x, trigger ALL_VA)\n",
           fake_mmu_inval_pdb);
    if (v.pages_mapped != 512 + 1) {
        printf("FALLO: %u páginas mapeadas (esperaba 513)\n", v.pages_mapped);
        return -1;
    }
    /* Raíz + 4 niveles intermedios + 2 hojas: la segunda hoja aparece porque la
     * página de sysmem cae justo en el siguiente tramo de 2 MiB. */
    if (v.pt_nr != 7) {
        printf("FALLO: %u tablas (esperaba 7: raíz, 4 niveles y 2 hojas)\n", v.pt_nr);
        return -1;
    }

    for (i = 0; i < 512; i++) {
        uint64_t at = VMM_T_VA + (uint64_t)i * 4096ull;

        if (gsp_vmm_translate(&v, at, &phys, &pte) != 0) {
            printf("FALLO: VA 0x%llx no traduce\n", (unsigned long long)at);
            return -1;
        }
        if (phys != VMM_T_VRAM_PA + (uint64_t)i * 4096ull) {
            printf("FALLO: VA 0x%llx → 0x%llx\n", (unsigned long long)at,
                   (unsigned long long)phys);
            return -1;
        }
        if ((pte & ~VMM_T_ADDR_MASK) != VMM_T_PTE_VRAM_LOW) {
            printf("FALLO: PTE de VRAM con banderas 0x%llx (esperaba 0x%llx)\n",
                   (unsigned long long)(pte & ~VMM_T_ADDR_MASK),
                   (unsigned long long)VMM_T_PTE_VRAM_LOW);
            return -1;
        }
    }
    printf("OK: 512 PTEs de VRAM, banderas 0x%llx (VALID, aper 0, PCF 0x10)\n",
           (unsigned long long)VMM_T_PTE_VRAM_LOW);

    if (gsp_vmm_translate(&v, VMM_T_VA + VMM_T_VRAM_SZ, &phys, &pte) != 0 ||
        phys != scratch.phys ||
        (pte & ~VMM_T_ADDR_MASK) != VMM_T_PTE_SYS_LOW) {
        printf("FALLO: PTE de sysmem 0x%llx → 0x%llx\n",
               (unsigned long long)pte, (unsigned long long)phys);
        return -1;
    }
    printf("OK: PTE de sysmem, banderas 0x%llx (aper 2, PCF 0x11)\n",
           (unsigned long long)VMM_T_PTE_SYS_LOW);

    /* PÁGINAS DE 2 MiB. Es lo que rompe el techo de residencia: con PTEs de 4 KiB
     * cada tabla hoja cubre 2 MiB y `GSP_VMM_MAX_PT` son 96 (menos las ~42 del
     * bring-up y el grctx), o sea ~108 MiB de pesos residentes como mucho. Una
     * entrada de PD0 cubre 2 MiB **sin tabla hoja** y su tabla, 512 MiB.
     *
     * Lo que se fija aquí es exactamente lo que en hardware no dejaría rastro: que
     * el PTE grande va en la mitad BAJA de la entrada (la alta es el PDE hacia la
     * hoja), que tiene el mismo encoding que un PTE normal, que NO se crea tabla
     * hoja, y que las dos mitades **no** pueden estar válidas a la vez — eso último
     * es comportamiento indefinido de la MMU, o sea el fallo que sólo se vería como
     * la GPU leyendo memoria ajena. */
    {
        const uint64_t va_big = VMM_T_VA + 0x4000000ull;   /* +64 MiB, alineado a 2 MiB */
        const uint64_t pa_big = 0x0000000280000000ull;     /* 10 GiB, alineado a 2 MiB */
        const uint64_t sz_big = 4ull * 1024ull * 1024ull;  /* dos páginas grandes */
        unsigned tablas_antes = v.pt_nr;
        unsigned paginas_antes = v.pages_mapped;
        struct gsp_vmm_pt *pd0;
        uint64_t bajo, alto;

        if (gsp_vmm_map_big(&v, va_big, pa_big, sz_big, GSP_VMM_VRAM) != 0) {
            printf("FALLO: gsp_vmm_map_big\n");
            return -1;
        }
        /* Ni una tabla hoja: sólo las intermedias que falten hasta PD0. Con 4 KiB
         * estos 4 MiB habrían costado DOS hojas. */
        {
            unsigned i, hojas = 0;

            for (i = tablas_antes; i < v.pt_nr; i++) {
                if (v.pt[i].level == 0u) {
                    hojas++;
                }
            }
            if (hojas != 0u) {
                printf("FALLO: el mapeo grande creó %u tablas hoja\n", hojas);
                return -1;
            }
        }
        if (v.pages_mapped - paginas_antes != 2u * 512u) {
            printf("FALLO: el mapeo grande contó %u páginas (esperaba 1024)\n",
                   v.pages_mapped - paginas_antes);
            return -1;
        }
        /* El PTE en crudo, en la mitad baja, y la alta intacta. */
        pd0 = pt_find(&v, 1u, va_big);
        if (!pd0) {
            printf("FALLO: no hay tabla de PD0 para la página grande\n");
            return -1;
        }
        bajo = pt_read_big(&v, pd0, lvl_index(&v, 1u, va_big));
        alto = pt_read(&v, pd0, lvl_index(&v, 1u, va_big));
        if ((bajo & ~VMM_T_ADDR_MASK) != VMM_T_PTE_VRAM_LOW ||
            (bajo & VMM_T_ADDR_MASK) != pa_big) {
            printf("FALLO: PTE grande 0x%llx (esperaba banderas 0x%llx y phys 0x%llx)\n",
                   (unsigned long long)bajo, (unsigned long long)VMM_T_PTE_VRAM_LOW,
                   (unsigned long long)pa_big);
            return -1;
        }
        if (alto != 0ull) {
            printf("FALLO: la mitad alta de la entrada de PD0 no está vacía (0x%llx) "
                   "— con las dos válidas la MMU es indefinida\n",
                   (unsigned long long)alto);
            return -1;
        }
        /* Y que `gsp_vmm_translate` la entienda: si no, la función con la que el
         * bring-up relee sus propios mapeos diría «no traduce» de una VA mapeada. */
        if (gsp_vmm_translate(&v, va_big + 0x1000ull, &phys, &pte) != 0 ||
            phys != pa_big || (pte & ~VMM_T_ADDR_MASK) != VMM_T_PTE_VRAM_LOW) {
            printf("FALLO: translate de una página grande → 0x%llx (pte 0x%llx)\n",
                   (unsigned long long)phys, (unsigned long long)pte);
            return -1;
        }
        if (gsp_vmm_translate(&v, va_big + sz_big - 0x1000ull, &phys, &pte) != 0 ||
            phys != pa_big + 0x200000ull) {
            printf("FALLO: translate del final del mapeo grande → 0x%llx\n",
                   (unsigned long long)phys);
            return -1;
        }
        printf("OK: 2 páginas de 2 MiB en la mitad baja de PD0 — sin tabla hoja, "
               "banderas 0x%llx, y translate las resuelve\n",
               (unsigned long long)VMM_T_PTE_VRAM_LOW);

        /* LA EXCLUSIÓN MUTUA, en los dos sentidos. */
        if (gsp_vmm_map(&v, va_big, pa_big, 4096ull, GSP_VMM_VRAM) == 0) {
            printf("FALLO: se colgó una tabla hoja de 4 KiB de una entrada que ya "
                   "es página grande\n");
            return -1;
        }
        if (gsp_vmm_map_big(&v, VMM_T_VA, VMM_T_VRAM_PA, 2ull * 1024ull * 1024ull,
                            GSP_VMM_VRAM) == 0) {
            printf("FALLO: se puso una página grande donde ya hay tabla hoja\n");
            return -1;
        }
        /* Y los desalineados. */
        if (gsp_vmm_map_big(&v, va_big + 4096ull, pa_big, sz_big, GSP_VMM_VRAM) == 0 ||
            gsp_vmm_map_big(&v, va_big, pa_big + 4096ull, sz_big, GSP_VMM_VRAM) == 0 ||
            gsp_vmm_map_big(&v, va_big, pa_big, 4096ull, GSP_VMM_VRAM) == 0) {
            printf("FALLO: el mapeo grande acepta VA, física o tamaño sin alinear "
                   "a 2 MiB\n");
            return -1;
        }
        printf("OK: página grande y tabla hoja se excluyen en la misma entrada de "
               "PD0 (los dos sentidos), y se rechaza lo no alineado a 2 MiB\n");
    }

    /* Sólo lectura, en las dos aperturas. Lo que se prueba no es que la bandera
     * llegue, sino que **cambia el PCF y nada más**: un `ro` que además tocara la
     * apertura o el caché sería otra cosa mapeada de otra forma, y desde fuera se
     * vería igual de "read-only". */
    {
        uint64_t ro_va = VMM_T_VA + VMM_T_VRAM_SZ + 0x10000ull;

        if (gsp_vmm_map_flags(&v, ro_va, VMM_T_VRAM_PA, 4096ull, GSP_VMM_VRAM,
                              GSP_VMM_RO) != 0 ||
            gsp_vmm_map_flags(&v, ro_va + 4096ull, scratch.phys, 4096ull,
                              GSP_VMM_SYSMEM, GSP_VMM_RO) != 0) {
            printf("FALLO: gsp_vmm_map_flags con GSP_VMM_RO\n");
            return -1;
        }
        if (gsp_vmm_translate(&v, ro_va, &phys, &pte) != 0 ||
            (pte & ~VMM_T_ADDR_MASK) != VMM_T_PTE_VRAM_RO_LOW) {
            printf("FALLO: PTE de VRAM sólo lectura con banderas 0x%llx (esperaba "
                   "0x%llx)\n", (unsigned long long)(pte & ~VMM_T_ADDR_MASK),
                   (unsigned long long)VMM_T_PTE_VRAM_RO_LOW);
            return -1;
        }
        if (gsp_vmm_translate(&v, ro_va + 4096ull, &phys, &pte) != 0 ||
            (pte & ~VMM_T_ADDR_MASK) != VMM_T_PTE_SYS_RO_LOW) {
            printf("FALLO: PTE de sysmem sólo lectura con banderas 0x%llx "
                   "(esperaba 0x%llx)\n",
                   (unsigned long long)(pte & ~VMM_T_ADDR_MASK),
                   (unsigned long long)VMM_T_PTE_SYS_RO_LOW);
            return -1;
        }
        /* Y que sin la bandera siga saliendo el de lectura y escritura: si el `ro`
         * se quedara pegado en algún estado, esto lo caza. */
        if (gsp_vmm_map(&v, ro_va + 8192ull, VMM_T_VRAM_PA, 4096ull,
                        GSP_VMM_VRAM) != 0 ||
            gsp_vmm_translate(&v, ro_va + 8192ull, &phys, &pte) != 0 ||
            (pte & ~VMM_T_ADDR_MASK) != VMM_T_PTE_VRAM_LOW) {
            printf("FALLO: tras un mapeo RO, el siguiente sin bandera sale 0x%llx\n",
                   (unsigned long long)(pte & ~VMM_T_ADDR_MASK));
            return -1;
        }
        printf("OK: sólo lectura cambia sólo el PCF (VRAM 0x%llx, sysmem 0x%llx) y "
               "no se queda pegado\n", (unsigned long long)VMM_T_PTE_VRAM_RO_LOW,
               (unsigned long long)VMM_T_PTE_SYS_RO_LOW);
    }

    /* Lo que no está mapeado tiene que fallar; si no, el recorrido no prueba nada. */
    if (gsp_vmm_translate(&v, VMM_T_VA + VMM_T_VRAM_SZ + 4096ull, &phys, &pte) == 0) {
        printf("FALLO: una VA sin mapear traduce a 0x%llx\n",
               (unsigned long long)phys);
        return -1;
    }
    if (gsp_vmm_translate(&v, VMM_T_VA - 4096ull, &phys, &pte) == 0) {
        printf("FALLO: la VA justo antes del mapeo traduce a 0x%llx\n",
               (unsigned long long)phys);
        return -1;
    }
    printf("OK: fuera del mapeo no traduce (ni por arriba ni por abajo)\n");

    /* Los PDE, uno a uno y en crudo. El de nivel 1 es doble: la mitad de páginas
     * grandes tiene que quedarse a cero. */
    for (i = 0; i < v.pt_nr; i++) {
        const struct gsp_vmm_pt *pt = &v.pt[i];
        const uint64_t *raw = (const uint64_t *)pt->mem.va;
        uint32_t idx;
        uint64_t e;

        if (pt->level == 0) {
            continue;
        }
        idx = lvl_index(&v, pt->level, VMM_T_VA);
        if (pt->level == 1) {
            if (raw[(unsigned long)idx * 2u] != 0) {
                printf("FALLO: la mitad de páginas grandes de la PDE doble no "
                       "está a cero (0x%llx)\n",
                       (unsigned long long)raw[(unsigned long)idx * 2u]);
                return -1;
            }
            e = raw[(unsigned long)idx * 2u + 1u];
        } else {
            e = raw[idx];
        }
        if (e & 1ull) {
            printf("FALLO: PDE de nivel %u con el bit 0 puesto — eso es IS_PTE, "
                   "no VALID: la MMU lo leería como traducción final\n", pt->level);
            return -1;
        }
        if ((e & ~VMM_T_ADDR_MASK) != VMM_T_PDE_SYS_LOW) {
            printf("FALLO: PDE de nivel %u con banderas 0x%llx (esperaba 0x%llx)\n",
                   pt->level, (unsigned long long)(e & ~VMM_T_ADDR_MASK),
                   (unsigned long long)VMM_T_PDE_SYS_LOW);
            return -1;
        }
        if ((e & VMM_T_ADDR_MASK) == 0) {
            printf("FALLO: PDE de nivel %u sin dirección\n", pt->level);
            return -1;
        }
    }
    printf("OK: PDEs con bit 0 a cero, aper 2 y PCF 1; la mitad grande vacía\n");

    /* Promoción del grctx de gb205 (tamaños del ciclo 2026-07-29): con el pool de
     * 24 tablas esto fallaba al mapear ATTRIBUTE_CB; aquí se comprueba que cabe. */
    {
        static const struct {
            uint64_t size;
            uint64_t align;
            unsigned ro;
            unsigned skip_map;
            unsigned is_attr_cb;
            const char *name;
        } gb205_grctx[] = {
            { 0x349000ull, 0x200000ull, 0, 0, 0, "MAIN" },
            { 36ull * 1024ull, 0x1000ull, 0, 0, 0, "PATCH" },
            { 12ull * 1024ull, 0x1000ull, 0, 0, 0, "BUNDLE_CB" },
            { 128ull * 1024ull, 0x10000ull, 0, 0, 0, "PAGEPOOL" },
            { 51552ull * 1024ull, 0x4000000ull, 0, 0, 1, "ATTRIBUTE_CB" },
            { 512ull * 1024ull, 0x10000ull, 0, 0, 0, "RTV_CB_GLOBAL" },
            { 64ull * 1024ull, 0x10000ull, 0, 0, 0, "FECS_EVENT" },
            { 512ull * 1024ull, 0x10000ull, 0, 1, 0, "PRIV_ACCESS_MAP" },
            { 512ull * 1024ull, 0x10000ull, 1, 0, 0, "UNRESTRICTED_PRIV_ACCESS_MAP" },
        };
        uint64_t va_next = GSP_GRCTX_VA_BASE;
        uint64_t fake_phys = 0x600000ull;
        uint64_t attr_va = 0;
        unsigned n;
        unsigned pt_grctx_start = v.pt_nr;

        for (n = 0; n < sizeof(gb205_grctx) / sizeof(gb205_grctx[0]); n++) {
            const uint64_t size = gb205_grctx[n].size;
            const uint64_t align = gb205_grctx[n].align;
            uint64_t va = align_up_u64(va_next, align);
            unsigned flags = gb205_grctx[n].ro ? GSP_VMM_RO : 0u;

            if (gb205_grctx[n].skip_map) {
                va_next = va + size;
                continue;
            }
            if (gsp_vmm_map_flags(&v, va, fake_phys, size, GSP_VMM_VRAM, flags) != 0) {
                printf("FALLO: grctx gb205 no mapea %s en 0x%llx (%llu KiB, "
                       "tablas=%u/%u)\n", gb205_grctx[n].name,
                       (unsigned long long)va,
                       (unsigned long long)(size / 1024ull), v.pt_nr,
                       GSP_VMM_MAX_PT);
                return -1;
            }
            if (gsp_vmm_translate(&v, va, &phys, &pte) != 0 ||
                phys != fake_phys) {
                printf("FALLO: grctx %s VA 0x%llx → phys 0x%llx\n",
                       gb205_grctx[n].name, (unsigned long long)va,
                       (unsigned long long)phys);
                return -1;
            }
            if (gb205_grctx[n].is_attr_cb) {
                attr_va = va;
                if (gsp_vmm_translate(&v, va + size - 4096ull, &phys, &pte) != 0 ||
                    phys != fake_phys + size - 4096ull) {
                    printf("FALLO: grctx ATTRIBUTE_CB extremo alto no traduce\n");
                    return -1;
                }
            }
            fake_phys += size;
            va_next = va + size;
        }
        if (v.pt_nr >= GSP_VMM_MAX_PT) {
            printf("FALLO: grctx agotó el pool (%u >= %u)\n", v.pt_nr,
                   GSP_VMM_MAX_PT);
            return -1;
        }
        if (v.pt_nr <= 24u) {
            printf("FALLO: grctx usa %u tablas — no supera el límite viejo de 24\n",
                   v.pt_nr);
            return -1;
        }
        if (attr_va != 0x8044000000ull) {
            printf("FALLO: ATTRIBUTE_CB VA 0x%llx (esperaba 0x8044000000)\n",
                   (unsigned long long)attr_va);
            return -1;
        }
        printf("OK: grctx gb205 mapeado (%u tablas, +%u desde %u; ATTRIBUTE_CB "
               "0x%llx)\n", v.pt_nr, v.pt_nr - pt_grctx_start, pt_grctx_start,
               (unsigned long long)attr_va);
    }

    /* Y el reparto de VRAM. La región basada en 0 es EL caso de esta tarjeta,
     * no un caso raro: los offsets de VRAM cuentan desde el inicio del
     * framebuffer, así que la única región utilizable del GB205 empieza en 0.
     * Esta prueba afirmaba justo lo contrario ("base 0: inservible a
     * propósito") y por eso estaba verde mientras el bring-up en hardware se
     * quedaba sin un solo byte repartible (2026-07-27). */
    memset(&si, 0, sizeof(si));
    si.ready = 1;
    si.region_nr = 3;
    si.region[0].base = 0;                      /* el caso real: base 0 */
    si.region[0].size = 0x100000ull;            /* 1 MiB */
    si.region[1].base = 0x200000ull;
    si.region[1].size = 0x300000ull;            /* 3 MiB */
    si.region[2].base = 0x800000ull;
    si.region[2].size = 0x800ull;               /* < 1 página: esta sí sobra */
    if (gsp_vram_init(&pool, &si) != 0 || pool.region_nr != 2 ||
        pool.total != 0x100000ull - 4096ull + 0x300000ull) {
        printf("FALLO: gsp_vram_init con región base 0 (nr=%u total=%llu)\n",
               pool.region_nr, (unsigned long long)pool.total);
        return -1;
    }
    {
        /* La primera página se reserva para que el 0 siga significando "no hay
         * sitio" y nada más: un reparto válido nunca puede devolver 0. */
        uint64_t a = gsp_vram_alloc(&pool, 4096, 4096);
        uint64_t b = gsp_vram_alloc(&pool, 0x300000ull, 0x100000ull);

        if (a != 4096ull) {
            printf("FALLO: el primer bloque de la región base 0 salió en 0x%llx\n",
                   (unsigned long long)a);
            return -1;
        }
        if (b != 0x200000ull || a == b) {
            printf("FALLO: reparto de VRAM a=0x%llx b=0x%llx\n",
                   (unsigned long long)a, (unsigned long long)b);
            return -1;
        }
        if (gsp_vram_alloc(&pool, 0x300000ull, 4096) != 0) {
            printf("FALLO: se repartió VRAM que no cabía\n");
            return -1;
        }
    }
    printf("OK: reparto de VRAM (región base 0 utilizable, 1ª página reservada)\n");

    /* El desmontaje: UNSET_PAGE_DIRECTORY y los cuatro FREE (vaspace, subdevice,
     * device, cliente). Se le dan respuestas para que no gaste los plazos. */
    base = *rpc.rptr;
    fake_rpc_post_payload(lo, base % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL, 0,
                          ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    for (i = 0; i < 4; i++) {
        fake_rpc_post_payload(lo, (base + 1 + i) % 63, NV_VGPU_MSG_FUNCTION_FREE,
                              0, NULL, 0);
    }
    msgq->tx.writePtr = (base + 5) % 63;

    wptr0 = *q.wptr;
    gsp_dma_free(&scratch);
    gsp_vmm_fini(&v);
    if (v.ready || v.bound || v.pt_nr != 0) {
        printf("FALLO: tras el fini quedan ready=%d bound=%d tablas=%u\n",
               v.ready, v.bound, v.pt_nr);
        return -1;
    }
    {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)(wptr0 % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_control *c = (const rpc_gsp_rm_control *)(hdr + 1);

        if (c->cmd != NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY) {
            printf("FALLO: el fini no quita el directorio (cmd=0x%08x)\n", c->cmd);
            return -1;
        }
    }
    printf("OK: el fini quita el directorio antes de soltar las tablas\n");
    return 0;
}

/* Lee un campo del QMD por (lo, hi), igual que lo escribe `qmd_set_bits`. */
static uint64_t qmd_get_bits(const uint32_t *qmd, unsigned lo, unsigned hi)
{
    uint64_t v = 0;
    unsigned i;

    for (i = lo; i <= hi; i++)
        v |= (uint64_t)((qmd[i / 32u] >> (i % 32u)) & 1u) << (i - lo);
    return v;
}

/* G4f/G5: el QMD tal y como lo va a leer el SM. Un campo a cero aquí no da error
 * en ningún sitio — simplemente lanza mal, y en la GPU eso es un cuelgue sin
 * traza. Se contrasta contra clcec0qmd.h campo a campo.
 *
 * Va parametrizado por kernel y por malla porque los dos números que de verdad
 * cambian entre saxpy y matvec —el regcount (10 vs 37) y la VA del programa— son
 * justo los que un banco escrito contra un solo kernel daría por buenos en el
 * otro. */
static int check_qmd_fields(const struct gsp_compute *cp,
                            const struct gsp_kernel *k, unsigned grid,
                            const GspQmdV05 *q)
{
    const uint32_t *w = q->words;
    uint64_t prog, cbank, sem;

    if (qmd_get_bits(w, QMDV05_QMD_TYPE) != NVCEC0_QMDV05_00_QMD_TYPE_GRID_CTA ||
        qmd_get_bits(w, QMDV05_QMD_MAJOR_VERSION) !=
            NVCEC0_QMDV05_00_QMD_MAJOR_VERSION_V05 ||
        qmd_get_bits(w, QMDV05_QMD_GROUP_ID) != 0x1fu ||
        qmd_get_bits(w, QMDV05_API_VISIBLE_CALL_LIMIT) !=
            NVCEC0_QMDV05_00_API_VISIBLE_CALL_LIMIT_NO_CHECK) {
        printf("FALLO: QMD type/version/group/call_limit\n");
        return -1;
    }

    prog = (qmd_get_bits(w, QMDV05_PROGRAM_ADDRESS_UPPER_S4) << 32) |
           qmd_get_bits(w, QMDV05_PROGRAM_ADDRESS_LOWER_S4);
    if (prog << 4 != k->sass_va) {
        printf("FALLO: PROGRAM_ADDRESS=0x%llx, esperaba 0x%llx (%s)\n",
               (unsigned long long)(prog << 4), (unsigned long long)k->sass_va,
               k->name);
        return -1;
    }

    if (qmd_get_bits(w, QMDV05_REGISTER_COUNT) != k->regcount) {
        printf("FALLO: REGISTER_COUNT=%llu, el cubin de %s dice %u\n",
               (unsigned long long)qmd_get_bits(w, QMDV05_REGISTER_COUNT),
               k->name, k->regcount);
        return -1;
    }
    if (qmd_get_bits(w, QMDV05_CTA_THREAD_DIMENSION0) != G4F_CTA_THREADS ||
        qmd_get_bits(w, QMDV05_CTA_THREAD_DIMENSION1) != 1 ||
        qmd_get_bits(w, QMDV05_CTA_THREAD_DIMENSION2) != 1 ||
        qmd_get_bits(w, QMDV05_GRID_WIDTH) != grid ||
        qmd_get_bits(w, QMDV05_GRID_HEIGHT) != 1 ||
        qmd_get_bits(w, QMDV05_GRID_DEPTH) != 1) {
        printf("FALLO: dimensiones de malla/CTA\n");
        return -1;
    }

    cbank = (qmd_get_bits(w, QMDV05_CBANK0_ADDR_UPPER_S6) << 32) |
            qmd_get_bits(w, QMDV05_CBANK0_ADDR_LOWER_S6);
    if (cbank << 6 != cp->data_va + G4F_CBANK_OFF) {
        printf("FALLO: CBANK0 addr=0x%llx, esperaba 0x%llx\n",
               (unsigned long long)(cbank << 6),
               (unsigned long long)(cp->data_va + G4F_CBANK_OFF));
        return -1;
    }
    if ((qmd_get_bits(w, QMDV05_CBANK0_SIZE_S4) << 4) < k->cbank_size) {
        printf("FALLO: CBANK0 size=%llu < %u\n",
               (unsigned long long)(qmd_get_bits(w, QMDV05_CBANK0_SIZE_S4) << 4),
               k->cbank_size);
        return -1;
    }
    if (qmd_get_bits(w, QMDV05_CBANK0_VALID) != 1) {
        printf("FALLO: CBANK0 no marcado válido\n");
        return -1;
    }

    sem = (qmd_get_bits(w, QMDV05_RELEASE_SEM0_ADDR_UPPER) << 32) |
          qmd_get_bits(w, QMDV05_RELEASE_SEM0_ADDR_LOWER);
    if (qmd_get_bits(w, QMDV05_RELEASE_ENABLE0) != 1 ||
        sem != cp->data_va + G4F_SEM_OFF ||
        qmd_get_bits(w, QMDV05_RELEASE_SEM0_PAYLOAD_LOWER) != G4F_SEM_PAYLOAD) {
        printf("FALLO: semáforo de fin del QMD (enable=%llu addr=0x%llx)\n",
               (unsigned long long)qmd_get_bits(w, QMDV05_RELEASE_ENABLE0),
               (unsigned long long)sem);
        return -1;
    }

    printf("OK: QMD v05 de %s — prog 0x%llx, %u regs, malla %ux1x1 de CTA %ux1x1, "
           "cbank0 y semáforo\n", k->name, (unsigned long long)k->sass_va,
           k->regcount, grid, G4F_CTA_THREADS);
    return 0;
}

static int check_qmd_v02_fields(const struct gsp_compute *cp,
                                const struct gsp_kernel *k, unsigned grid,
                                const GspQmdV02 *q)
{
    const uint32_t *w = q->words;
    uint64_t prog, cbank, sem;

    if (qmd_get_bits(w, QMDV02_QMD_MAJOR_VERSION) !=
            NVA0C0_QMDV01_07_QMD_MAJOR_VERSION_V01 ||
        qmd_get_bits(w, QMDV02_SEMAPHORE_RELEASE_ENABLE0) != 1) {
        printf("FALLO: QMD v2 major/semaphore\n");
        return -1;
    }
    prog = qmd_get_bits(w, QMDV02_PROGRAM_OFFSET);
    if (prog != ((k->sass_va - GSP_VA_BASE) >> 4)) {
        printf("FALLO: PROGRAM_OFFSET=0x%llx, esperaba 0x%llx rel (%s)\n",
               (unsigned long long)prog,
               (unsigned long long)((k->sass_va - GSP_VA_BASE) >> 4),
               k->name);
        return -1;
    }
    if (qmd_get_bits(w, QMDV02_REGISTER_COUNT) != k->regcount ||
        qmd_get_bits(w, QMDV02_CTA_RASTER_WIDTH) != grid ||
        qmd_get_bits(w, QMDV02_CTA_THREAD_DIMENSION0) != G4F_CTA_THREADS) {
        printf("FALLO: QMD v2 regs/malla/CTA de %s\n", k->name);
        return -1;
    }
    cbank = (qmd_get_bits(w, QMDV02_CONSTANT_BUFFER_ADDR_UPPER0) << 32) |
            qmd_get_bits(w, QMDV02_CONSTANT_BUFFER_ADDR_LOWER0);
    if (GSP_VA_BASE + (cbank << 6) != cp->data_va + G4F_CBANK_OFF) {
        printf("FALLO: CBANK0 addr rel=0x%llx\n", (unsigned long long)(cbank << 6));
        return -1;
    }
    sem = (qmd_get_bits(w, QMDV02_RELEASE0_ADDRESS_UPPER) << 32) |
          qmd_get_bits(w, QMDV02_RELEASE0_ADDRESS_LOWER);
    if (sem != cp->data_va + G4F_SEM_OFF ||
        qmd_get_bits(w, QMDV02_RELEASE0_PAYLOAD) != G4F_SEM_PAYLOAD) {
        printf("FALLO: semáforo QMD v2\n");
        return -1;
    }
    printf("OK: QMD v2 de %s — prog>>4, %u regs, malla %ux1x1\n",
           k->name, k->regcount, grid);
    return 0;
}

/* Los parámetros van donde el cubin dice, no donde nos venga bien. */
static int check_compute_params(struct gsp_compute *cp)
{
    const unsigned char *p;
    const uint64_t xv = 0x1111222233334444ull, yv = 0x5555666677778888ull;

    gsp_compute_set_params(cp, 2.5f, xv, yv, 77);
    p = (const unsigned char *)cp->data.va + G4F_CBANK_OFF + cp->saxpy.param_base;

    if (cp->saxpy.param_base + cp->saxpy.param_size != cp->saxpy.cbank_size) {
        printf("FALLO: params en %u+%u no acaban en el final del cbank (%u)\n",
               cp->saxpy.param_base, cp->saxpy.param_size, cp->saxpy.cbank_size);
        return -1;
    }
    if (*(const float *)(p + cp->saxpy.param_off[0]) != 2.5f ||
        *(const uint64_t *)(p + cp->saxpy.param_off[1]) != xv ||
        *(const uint64_t *)(p + cp->saxpy.param_off[2]) != yv ||
        *(const uint32_t *)(p + cp->saxpy.param_off[3]) != 77u) {
        printf("FALLO: parámetros mal colocados en el constant bank\n");
        return -1;
    }
    printf("OK: params a/x/y/n en cbank0+0x%x (+%u/+%u/+%u/+%u)\n",
           cp->saxpy.param_base, cp->saxpy.param_off[0], cp->saxpy.param_off[1],
           cp->saxpy.param_off[2], cp->saxpy.param_off[3]);
    return 0;
}

/* G5: los cinco parámetros de matvec. `rows` y `cols` son `int` en el .cu y van
 * pegados (offsets 24 y 28): escribir 8 B en el de `rows` —el error natural
 * viniendo de los tres punteros de arriba— pisa `cols` con ceros y el kernel
 * calcula filas de longitud 0 sin quejarse. Por eso se comprueba el vecino. */
static int check_mv_params(struct gsp_compute *cp)
{
    const unsigned char *cb = (const unsigned char *)cp->data.va + G4F_CBANK_OFF;
    const unsigned char *p = cb + cp->matvec.param_base;
    const uint64_t wv = 0xaaaabbbbccccddddull, xv = 0x1111222233334444ull;
    const uint64_t yv = 0x5555666677778888ull;

    gsp_compute_set_mv_params(cp, &cp->matvec, wv, xv, yv, 33u, 1024u);

    if (cp->matvec.param_base + cp->matvec.param_size != cp->matvec.cbank_size) {
        printf("FALLO: params de matvec en %u+%u no acaban en el final del cbank "
               "(%u)\n", cp->matvec.param_base, cp->matvec.param_size,
               cp->matvec.cbank_size);
        return -1;
    }
    if (*(const uint64_t *)(p + cp->matvec.param_off[0]) != wv ||
        *(const uint64_t *)(p + cp->matvec.param_off[1]) != xv ||
        *(const uint64_t *)(p + cp->matvec.param_off[2]) != yv ||
        *(const uint32_t *)(p + cp->matvec.param_off[3]) != 33u ||
        *(const uint32_t *)(p + cp->matvec.param_off[4]) != 1024u) {
        printf("FALLO: parámetros de matvec mal colocados en el constant bank\n");
        return -1;
    }
    /* ntid, que es de dónde saca el kernel su blockDim.x. Si esto se queda a cero
     * el índice de fila sale siempre 0 y todos los hilos escriben y[0]. */
    if (*(const uint32_t *)(cb + 0x0) != G4F_CTA_THREADS ||
        *(const uint32_t *)(cb + 0x4) != 1u ||
        *(const uint32_t *)(cb + 0x8) != 1u) {
        printf("FALLO: ntid en el prólogo del cbank0 (%u,%u,%u)\n",
               *(const uint32_t *)(cb + 0x0), *(const uint32_t *)(cb + 0x4),
               *(const uint32_t *)(cb + 0x8));
        return -1;
    }
    printf("OK: params w/x/y/rows/cols de matvec en cbank0+0x%x "
           "(+%u/+%u/+%u/+%u/+%u) y ntid=%u\n",
           cp->matvec.param_base, cp->matvec.param_off[0], cp->matvec.param_off[1],
           cp->matvec.param_off[2], cp->matvec.param_off[3],
           cp->matvec.param_off[4], G4F_CTA_THREADS);
    return 0;
}

/* Los kernels cuantizados: mismos parámetros que matvec (w, x, y, rows, cols), pero
 * cada uno con su `param_off`/`cbank_size` del cubin. Se comprueban los TRES porque
 * `gsp_compute_set_mv_params` llevaba `&cp->matvec` incrustado y hoy los offsets
 * coinciden por casualidad estructural: en cuanto un `.cu` cambie de firma, la
 * versión con el kernel fijo escribiría los parámetros en los offsets de otro y no
 * daría un solo error. */
static int check_mvq_params(struct gsp_compute *cp)
{
    const unsigned char *cb = (const unsigned char *)cp->data.va + G4F_CBANK_OFF;
    const uint64_t wv = 0x0102030405060708ull, xv = 0x1112131415161718ull;
    const uint64_t yv = 0x2122232425262728ull;
    const struct gsp_kernel *ks[2];
    unsigned i;

    ks[0] = &cp->matvec_q4k;
    ks[1] = &cp->matvec_q80;

    for (i = 0; i < 2u; i++) {
        const struct gsp_kernel *k = ks[i];
        const unsigned char *p = cb + k->param_base;

        if (k->param_count != 5u) {
            printf("FALLO: %s declara %u parámetros (esperaba 5)\n", k->name,
                   k->param_count);
            return -1;
        }
        if (k->param_base + k->param_size != k->cbank_size) {
            printf("FALLO: params de %s en %u+%u no acaban en el final del cbank "
                   "(%u)\n", k->name, k->param_base, k->param_size, k->cbank_size);
            return -1;
        }
        gsp_compute_set_mv_params(cp, k, wv, xv, yv, 7u, 2048u);
        if (*(const uint64_t *)(p + k->param_off[0]) != wv ||
            *(const uint64_t *)(p + k->param_off[1]) != xv ||
            *(const uint64_t *)(p + k->param_off[2]) != yv ||
            *(const uint32_t *)(p + k->param_off[3]) != 7u ||
            *(const uint32_t *)(p + k->param_off[4]) != 2048u) {
            printf("FALLO: parámetros de %s mal colocados en el constant bank\n",
                   k->name);
            return -1;
        }
    }
    printf("OK: params de matvec_q4k y matvec_q80 en su propio cbank (+%u/+%u), "
           "escritos por kernel y no por el fijo\n",
           cp->matvec_q4k.param_base, cp->matvec_q80.param_base);
    return 0;
}

/* La aritmética que decide QUÉ TROZO DE VRAM lee un warp. Es la parte que en
 * hardware no deja rastro: una fila mal medida lee los bytes del tensor de al lado y
 * devuelve un vector perfectamente creíble. Se comprueba con los tamaños reales de
 * los modelos que interesan (2048 y 5632 son hidden y ffn de TinyLlama) y que los
 * `cols` que no son número entero de bloques se RECHACEN. */
static int check_mvq_layout(void)
{
    static const unsigned cols_ok[] = { 256u, 1024u, 2048u, 5632u, 11008u };
    unsigned i;

    for (i = 0; i < sizeof(cols_ok) / sizeof(cols_ok[0]); i++) {
        unsigned c = cols_ok[i];
        unsigned long q4k = q_row_bytes(GSP_DTYPE_Q4_K, c);
        unsigned long q80 = q_row_bytes(GSP_DTYPE_Q8_0, c);

        if (q4k != (unsigned long)(c / Q4K_BLOCK_ELEMS) * Q4K_BLOCK_BYTES ||
            q80 != (unsigned long)(c / Q80_BLOCK_ELEMS) * Q80_BLOCK_BYTES) {
            printf("FALLO: row_bytes(%u) = %lu / %lu\n", c, q4k, q80);
            return -1;
        }
    }
    /* Rechazos: 255 y 0 no son superbloques enteros; 33 no es bloque Q8_0 entero;
     * F32 y MXFP4 no tienen kernel. */
    if (q_row_bytes(GSP_DTYPE_Q4_K, 255u) != 0ul ||
        q_row_bytes(GSP_DTYPE_Q4_K, 0u) != 0ul ||
        q_row_bytes(GSP_DTYPE_Q8_0, 33u) != 0ul ||
        q_row_bytes(GSP_DTYPE_F32, 256u) != 0ul ||
        q_row_bytes(GSP_DTYPE_MXFP4, 256u) != 0ul) {
        printf("FALLO: row_bytes acepta cols que no cuadran o un dtype sin kernel\n");
        return -1;
    }
    /* Y la talla que importa: una fila de ffn_up de TinyLlama son 22 superbloques,
     * 3168 B — contra los 22528 B que ocuparía en f32. */
    if (q_row_bytes(GSP_DTYPE_Q4_K, 5632u) != 3168ul) {
        printf("FALLO: la fila Q4_K de 5632 columnas mide %lu B\n",
               q_row_bytes(GSP_DTYPE_Q4_K, 5632u));
        return -1;
    }
    printf("OK: row_bytes de Q4_K/Q8_0 para las tallas reales, y rechazo de cols "
           "que no son bloque entero (fila de 5632 = 3168 B vs 22528 en f32)\n");
    return 0;
}

/* EL AMARRE C↔RUST. La misma aritmética existe dos veces: aquí en C (lo que compila
 * nvcc dentro del kernel SASS) y en Rust (`sosomodel::dequant`, lo que ejecuta la
 * CPU y el dispositivo software). Los números de abajo están duplicados a propósito
 * en `crates/sosomodel/tests/dequant_golden.rs`: tocar la decodificación de un lado
 * pone en rojo a uno de los dos. Sin esto, una divergencia sólo se ve como «la GPU
 * saca otros tokens» y se busca en el silicio, que es donde no está. */
static int check_q4k_decode(void)
{
    static const unsigned char scales[12] = {
        0x41, 0x82, 0xC3, 0x04, 0x45, 0x86, 0xC7, 0x08, 0x9A, 0xBC, 0xDE, 0xF0
    };
    static const unsigned char sc_esperado[8] = { 1, 2, 3, 4, 26, 44, 62, 0 };
    static const unsigned char m_esperado[8] = { 5, 6, 7, 8, 25, 43, 61, 15 };
    unsigned char blk[Q4K_BLOCK_BYTES];
    float x[Q4K_BLOCK_ELEMS];
    unsigned char q80[Q80_BLOCK_BYTES];
    float dot, dot80;
    int i, j;

    /* Misma receta que el test de Rust. */
    for (i = 0; i < Q4K_BLOCK_BYTES; i++) {
        blk[i] = 0;
    }
    blk[0] = 0x00; blk[1] = 0x38;   /* d = 0.5 en f16 */
    blk[2] = 0x00; blk[3] = 0x34;   /* dmin = 0.25 */
    for (i = 0; i < 12; i++) {
        blk[4 + i] = scales[i];
    }
    for (i = 0; i < 128; i++) {
        blk[16 + i] = (unsigned char)((i * 7 + 3) % 256);
    }
    for (i = 0; i < Q4K_BLOCK_ELEMS; i++) {
        x[i] = ((float)(i % 13) - 6.0f) * 0.125f;
    }

    for (j = 0; j < 8; j++) {
        unsigned char sc, m;

        q4k_scale_min(blk + 4, j, &sc, &m);
        if (sc != sc_esperado[j] || m != m_esperado[j]) {
            printf("FALLO: q4k_scale_min(%d) = (%u,%u), esperaba (%u,%u) — el lado "
                   "Rust dice lo segundo (dequant_golden.rs)\n",
                   j, sc, m, sc_esperado[j], m_esperado[j]);
            return -1;
        }
    }

    /* Valor dorado del producto punto, calculado por el Rust y fijado allí también. */
    dot = q4k_dot_block(blk, x);
    if (dot < -15.885f || dot > -15.865f) {
        printf("FALLO: q4k_dot_block = %f, el Rust dice -15.875 (dequant_golden.rs)\n",
               (double)dot);
        return -1;
    }

    /* Y el Q8_0, que es el kernel de humo: escala f32 + 32 int8, sin nibbles. */
    {
        union { unsigned int u; float f; } s;
        s.f = 0.25f;
        q80[0] = (unsigned char)(s.u & 0xffu);
        q80[1] = (unsigned char)((s.u >> 8) & 0xffu);
        q80[2] = (unsigned char)((s.u >> 16) & 0xffu);
        q80[3] = (unsigned char)((s.u >> 24) & 0xffu);
    }
    for (i = 0; i < 32; i++) {
        q80[4 + i] = (unsigned char)(signed char)(i * 9 - 100);
    }
    dot80 = q80_dot_block(q80, x);
    if (dot80 < 172.58f || dot80 > 172.61f) {
        printf("FALLO: q80_dot_block = %f, el Rust dice 172.59375\n", (double)dot80);
        return -1;
    }

    printf("OK: la decodificación Q4_K/Q8_0 en C da los mismos números que el Rust "
           "(escalas de 6 bits, nibbles cruzados y los dos productos punto)\n");
    return 0;
}

/* G5: el troceado por tandas. Es aritmética pura y es la parte que en hardware
 * no dejaría rastro: una tanda mal medida copia filas de otro sitio de la matriz
 * y el resultado sale plausible. Se comprueba que cada tanda quepa en LAS DOS
 * regiones (W e Y), que las columnas de más se rechacen, y que el par
 * stage/read lleve cada fila a su sitio y la devuelva a su sitio. */
static int check_mv_tiling(struct gsp_compute *cp)
{
    /* Las anchuras que importan: 1024/4096 son modelos pequeños, 11008 el FFN de
     * un 7B y 28672 el de un 70B — que es el caso que hay que poder decir que
     * cabe, porque es el objetivo de la fase. */
    static const unsigned cols_probe[] = { 1u, 16u, 512u, 1024u, 4096u, 11008u,
                                           28672u, G5_MAX_COLS };
    unsigned i;

    for (i = 0; i < sizeof(cols_probe) / sizeof(cols_probe[0]); i++) {
        unsigned cols = cols_probe[i];
        unsigned rows = gsp_compute_mv_rows_per_tile(cols);

        if (rows == 0u) {
            printf("FALLO: %u columnas deberían caber y dan 0 filas\n", cols);
            return -1;
        }
        if ((unsigned long)rows * cols * 4ul > G5_MV_W_BYTES) {
            printf("FALLO: cols=%u → %u filas = %lu B, la región W son %u B\n",
                   cols, rows, (unsigned long)rows * cols * 4ul, G5_MV_W_BYTES);
            return -1;
        }
        if ((unsigned long)rows * 4ul > G5_MV_Y_BYTES) {
            printf("FALLO: cols=%u → %u filas = %lu B de salida, la región Y son "
                   "%u B\n", cols, rows, (unsigned long)rows * 4ul, G5_MV_Y_BYTES);
            return -1;
        }
    }
    if (gsp_compute_mv_rows_per_tile(0u) != 0u ||
        gsp_compute_mv_rows_per_tile(G5_MAX_COLS + 1u) != 0u) {
        printf("FALLO: cols=0 o cols>%u tendrían que dar 0 filas\n", G5_MAX_COLS);
        return -1;
    }
    /* Que las regiones no se solapen entre ellas ni se salgan de la reserva: los
     * cuatro offsets están escritos a mano en el header y un solape sería un
     * kernel leyendo su propio vector de salida como si fuera la matriz. */
    if (G5_MV_W_OFF + G5_MV_W_BYTES > G5_MV_X_OFF ||
        G5_MV_X_OFF + G5_MV_X_BYTES > G5_MV_Y_OFF ||
        G5_MV_Y_OFF + G5_MV_Y_BYTES > G5_MV_SIZE) {
        printf("FALLO: las regiones del staging de G5 se solapan\n");
        return -1;
    }

    /* stage + read con una matriz de mentira. La GPU se simula a mano: se escribe
     * en la región Y lo que el kernel habría escrito, y se comprueba que `read`
     * lo deja en las filas correctas de `y`. Lo que esto caza es el `row0`, que es
     * el único índice que el kernel no ve. */
    {
        const unsigned cols = 8u, rows_total = 40u;
        unsigned per_tile = gsp_compute_mv_rows_per_tile(cols);
        float w[40 * 8], x[8], y[40], want[40];
        unsigned r, c, row0;

        for (c = 0; c < cols; c++) {
            x[c] = (float)(c + 1u);
        }
        for (r = 0; r < rows_total; r++) {
            float sum = 0.0f;
            for (c = 0; c < cols; c++) {
                w[r * cols + c] = (float)(r * cols + c);
                sum += w[r * cols + c] * x[c];
            }
            want[r] = sum;
            y[r] = -1.0f;
        }
        if (per_tile > rows_total) {
            per_tile = 7u;   /* fuerza varias tandas con un resto corto */
        }
        for (row0 = 0; row0 < rows_total; row0 += per_tile) {
            unsigned n = (rows_total - row0) < per_tile ? (rows_total - row0)
                                                        : per_tile;
            const float *gw;
            float *gy;

            gsp_compute_mv_stage(cp, w, x, n, cols, row0);
            gw = (const float *)((const unsigned char *)cp->mv.va + G5_MV_W_OFF);
            gy = (float *)((unsigned char *)cp->mv.va + G5_MV_Y_OFF);

            /* La tanda staged tiene que ser la tanda pedida, fila a fila. */
            for (r = 0; r < n; r++) {
                for (c = 0; c < cols; c++) {
                    if (gw[r * cols + c] != w[(row0 + r) * cols + c]) {
                        printf("FALLO: staging fila %u col %u: %f, esperaba %f\n",
                               row0 + r, c, (double)gw[r * cols + c],
                               (double)w[(row0 + r) * cols + c]);
                        return -1;
                    }
                }
            }
            if (*(const uint32_t *)((const unsigned char *)cp->data.va + G4F_SEM_OFF) != 0u) {
                printf("FALLO: el staging no puso el semáforo a cero\n");
                return -1;
            }
            /* Aquí ejecutaría la GPU. */
            for (r = 0; r < n; r++) {
                float sum = 0.0f;
                const float *gx =
                    (const float *)((const unsigned char *)cp->mv.va + G5_MV_X_OFF);

                for (c = 0; c < cols; c++) {
                    sum += gw[r * cols + c] * gx[c];
                }
                gy[r] = sum;
            }
            gsp_compute_mv_read(cp, y, n, row0);
        }
        for (r = 0; r < rows_total; r++) {
            if (y[r] != want[r]) {
                printf("FALLO: y[%u]=%f, esperaba %f (tandas de %u)\n",
                       r, (double)y[r], (double)want[r], per_tile);
                return -1;
            }
        }
        printf("OK: %u filas × %u cols en tandas de %u — staging, row0 y readback\n",
               rows_total, cols, per_tile);
    }
    return 0;
}

/* G6: aritmética del matvec residente (un QMD, warp/fila). */
static int check_g6_resident(void)
{
    unsigned grid;

    if (G6_ROWS_PER_CTA != 8u) {
        printf("FALLO: G6_ROWS_PER_CTA=%u (esperado 8)\n", G6_ROWS_PER_CTA);
        return -1;
    }
    grid = (2816u + G6_ROWS_PER_CTA - 1u) / G6_ROWS_PER_CTA;
    if (grid != 352u) {
        printf("FALLO: grid G6 para 2816 filas = %u (esperado 352)\n", grid);
        return -1;
    }
    if (G6_VA_BASE >= G6_VA_LIMIT ||
        G6_RES_X_BYTES != G5_MV_X_BYTES ||
        G6_MAX_ROWS < 1024u) {
        printf("FALLO: constantes de layout G6 incoherentes\n");
        return -1;
    }
    /* NINGUNA ventana de VAs puede solaparse con otra. Esto no es celo: la de
     * pesos residentes (G6) estuvo tres días encima de la del contexto de GR
     * —las dos empezaban en `GSP_VA_BASE + 0x40000000`— y el síntoma era un
     * `GR_EXCEPTION` sin falta de MMU en el matvec residente, con el volcado de
     * RM señalando al CTXCTL. Nada fallaba al mapear: las VAs estaban mapeadas,
     * a las páginas de otro. Un solapamiento no da error en ninguna parte, y por
     * eso tiene que darlo aquí. */
    {
        const struct { uint64_t base; uint64_t size; const char *que; } vent[] = {
            { G4F_DATA_VA,       G4F_DATA_SIZE,      "data G4f" },
            { G5_MV_VA,          G5_MV_SIZE,         "staging G5" },
            { G6_RES_VA,         G6_RES_SIZE,        "staging residente G6" },
            { G6_BOUNCE_VA,      G6_BOUNCE_BYTES,    "rebote G6" },
            { G6_VA_BASE,        G6_VA_LIMIT - G6_VA_BASE, "pesos residentes G6" },
            { GSP_GRCTX_VA_BASE, GSP_GRCTX_VA_SIZE,  "contexto de GR" },
        };
        unsigned i, j;

        for (i = 0; i < sizeof(vent) / sizeof(vent[0]); i++) {
            for (j = i + 1; j < sizeof(vent) / sizeof(vent[0]); j++) {
                if (vent[i].base < vent[j].base + vent[j].size &&
                    vent[j].base < vent[i].base + vent[i].size) {
                    printf("FALLO: la ventana de %s (0x%llx+0x%llx) pisa la de %s "
                           "(0x%llx+0x%llx)\n",
                           vent[i].que, (unsigned long long)vent[i].base,
                           (unsigned long long)vent[i].size, vent[j].que,
                           (unsigned long long)vent[j].base,
                           (unsigned long long)vent[j].size);
                    return -1;
                }
            }
        }
        printf("OK: las 6 ventanas de VAs del bring-up no se pisan entre sí\n");
    }

    /* El rebote de las subidas: múltiplo de página (la copia multilínea del CE
     * lo exige) y con ventana de VAs PROPIA. */
    if (G6_BOUNCE_BYTES < 4096u || (G6_BOUNCE_BYTES % 4096u) != 0u) {
        printf("FALLO: rebote G6 de %u B no es múltiplo de página\n",
               G6_BOUNCE_BYTES);
        return -1;
    }
    {
        const struct { uint64_t base; uint64_t size; const char *que; } otros[] = {
            { G4F_DATA_VA, G4F_DATA_SIZE, "data G4f" },
            { G5_MV_VA,    G5_MV_SIZE,    "staging G5" },
            { G6_RES_VA,   G6_RES_SIZE,   "staging residente G6" },
        };
        unsigned i;

        for (i = 0; i < sizeof(otros) / sizeof(otros[0]); i++) {
            if (G6_BOUNCE_VA < otros[i].base + otros[i].size &&
                otros[i].base < G6_BOUNCE_VA + G6_BOUNCE_BYTES) {
                printf("FALLO: el rebote G6 (0x%llx+%u) pisa el %s\n",
                       (unsigned long long)G6_BOUNCE_VA, G6_BOUNCE_BYTES,
                       otros[i].que);
                return -1;
            }
        }
    }
    printf("OK: G6 residente — grid 2816→%u, VA 0x%llx, rebote %u KiB en ventana "
           "propia\n", grid, (unsigned long long)G6_VA_BASE,
           G6_BOUNCE_BYTES >> 10);
    return 0;
}

/* Lo que el RM de mentira contesta a CE_GET_FAULT_METHOD_BUFFER_SIZE. El valor
 * concreto da igual —en HW lo dice la tarjeta—; lo que se comprueba es que se
 * pregunte y que lo contestado llegue tal cual al descriptor. */
#define FAKE_MTHDBUF_SIZE  0x1000u

/* Token del doorbell que devuelve el RM de mentira. Se elige con la parte alta
 * distinta de cero para que se vea si el port lo trocea o lo reordena: lo que
 * hay que escribir en el registro es el valor ENTERO tal cual vino, y un token
 * bien copiado pero mal escrito es indistinguible de uno mal copiado si el
 * número es pequeño. */
#define FAKE_DOORBELL_TOKEN  0x00070000u
#define FAKE_DOORBELL_KICK   (FAKE_DOORBELL_TOKEN | NV_VF_DOORBELL_RUNLIST_DOORBELL_ENABLE)
#define FAKE_DOORBELL_KICK_CHID(chid) (FAKE_DOORBELL_KICK | ((chid) & NV_VF_DOORBELL_VECTOR_MASK))

/* Catálogo de clases: que se pida bien y que lo contestado mande de verdad.
 *
 * Lo que hace falta demostrar aquí no es que la petición salga —eso es fácil—
 * sino que la elección **cambia con el catálogo**. Un `pick` que siempre
 * devuelve la primera candidata pasaría cualquier prueba que solo mirase el
 * caso Blackwell, y sería exactamente el bug que trae este cambio: elegir a
 * ciegas creyendo que se está preguntando. Por eso hay dos catálogos, uno de
 * cada familia, y se exige que salgan clases distintas. */
static int check_classlist(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_cmdq q;
    struct gsp_rm rm;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);
    unsigned char *cmdq_base = (unsigned char *)lo->shm.va + lo->cmdq_offset;
    rpc_gsp_rm_alloc ok;
    uint32_t base;
    uint32_t wptr0;
    unsigned i;
    static const uint32_t chan_cand[] = {
        BLACKWELL_CHANNEL_GPFIFO_B, BLACKWELL_CHANNEL_GPFIFO_A,
        HOPPER_CHANNEL_GPFIFO_A, AMPERE_CHANNEL_GPFIFO_B,
        AMPERE_CHANNEL_GPFIFO_A,
    };
    static const uint32_t ce_cand[] = {
        BLACKWELL_DMA_COPY_B, BLACKWELL_DMA_COPY_A, HOPPER_DMA_COPY_A,
        AMPERE_DMA_COPY_B, AMPERE_DMA_COPY_A,
    };
    /* Dos chips de mentira. El de Blackwell lleva las clases que `rm/gb20x.c`
     * dice para GB205; el de Ampere, las de una GA10x. Ninguno de los dos lleva
     * las del otro: si el pick no mira el catálogo, uno de los dos falla. */
    static const uint32_t cat_blackwell[] = {
        0x0000u, 0x0080u, 0x2080u, BLACKWELL_CHANNEL_GPFIFO_B,
        BLACKWELL_DMA_COPY_B, BLACKWELL_COMPUTE_B, 0x902du,
    };
    static const uint32_t cat_ampere[] = {
        0x0000u, 0x0080u, 0x2080u, AMPERE_CHANNEL_GPFIFO_A,
        AMPERE_DMA_COPY_A, AMPERE_COMPUTE_B,
    };
    const struct {
        const char *name;
        const uint32_t *list;
        unsigned nr;
        uint32_t want_chan;
        uint32_t want_ce;
    } chip[2] = {
        { "gb20x",  cat_blackwell,
          (unsigned)(sizeof(cat_blackwell) / sizeof(cat_blackwell[0])),
          BLACKWELL_CHANNEL_GPFIFO_B, BLACKWELL_DMA_COPY_B },
        { "ga10x",  cat_ampere,
          (unsigned)(sizeof(cat_ampere) / sizeof(cat_ampere[0])),
          AMPERE_CHANNEL_GPFIFO_A, AMPERE_DMA_COPY_A },
    };

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: rpc_init (cls)\n"); return -1; }
    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: cmdq_init (cls)\n"); return -1; }

    memset(&ok, 0, sizeof(ok));
    base = *rpc.rptr;
    for (i = 0; i < 3; i++) {
        fake_rpc_post_payload(lo, (base + i) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                              0, (const unsigned char *)&ok, (uint32_t)sizeof(ok));
    }
    msgq->tx.writePtr = (base + 3) % 63;
    if (gsp_rm_init(&q, &rpc, &rm) != 0) {
        printf("FALLO: gsp_rm_init (cls)\n");
        return -1;
    }

    /* Sin catálogo: `supported` no puede decir "no", tiene que decir "no sé". */
    gsp_rm_classes_forget();
    if (gsp_rm_class_supported(BLACKWELL_CHANNEL_GPFIFO_B) != -1) {
        printf("FALLO: sin catálogo, supported() no devuelve -1\n");
        return -1;
    }
    if (gsp_rm_class_pick("canal (sin catálogo)", chan_cand, 5) != chan_cand[0]) {
        printf("FALLO: sin catálogo el pick no cae en la primera candidata\n");
        return -1;
    }
    printf("OK: sin catálogo, 'no se sabe' (-1) y se prueba la primera candidata\n");

    for (i = 0; i < 2; i++) {
        unsigned char *reply = calloc(1, sizeof(rpc_gsp_rm_control) +
                                         sizeof(NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS));
        NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS *cl;
        uint32_t chan_cls;
        uint32_t ce_cls;
        unsigned k;

        if (!reply) { printf("FALLO: sin memoria (cls)\n"); return -1; }
        cl = (NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS *)
                 (reply + sizeof(rpc_gsp_rm_control));
        cl->numClasses = chip[i].nr;
        for (k = 0; k < chip[i].nr; k++) {
            cl->classList[k] = chip[i].list[k];
        }

        base = *rpc.rptr;
        fake_rpc_post_payload(lo, base % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL, 0,
                              reply, (uint32_t)(sizeof(rpc_gsp_rm_control) +
                                  sizeof(NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS)));
        msgq->tx.writePtr = (base + 1) % 63;

        wptr0 = *q.wptr;
        gsp_rm_classes_forget();
        if (gsp_rm_classes_probe(&rm) != 0) {
            printf("FALLO: gsp_rm_classes_probe (%s)\n", chip[i].name);
            free(reply);
            return -1;
        }
        free(reply);

        /* La petición: control de device, mandado entero (gotcha 5). */
        {
            const unsigned char *entry = cmdq_base + 4096 +
                                         (unsigned long)(wptr0 % 63) * 4096;
            const struct gsp_rpc_hdr *hdr =
                (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
            const rpc_gsp_rm_control *c = (const rpc_gsp_rm_control *)(hdr + 1);

            if (c->cmd != NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2 ||
                c->hObject != NVKM_RM_DEVICE ||
                c->paramsSize != sizeof(NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS)) {
                printf("FALLO: GET_CLASSLIST_V2 cmd=0x%x obj=0x%08x params=%u "
                       "(esperaba 0x%x/0x%08x/%zu)\n",
                       c->cmd, c->hObject, c->paramsSize,
                       NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2, NVKM_RM_DEVICE,
                       sizeof(NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS));
                return -1;
            }
        }

        chan_cls = gsp_rm_class_pick("canal", chan_cand, 5);
        ce_cls = gsp_rm_class_pick("CE", ce_cand, 5);
        if (chan_cls != chip[i].want_chan || ce_cls != chip[i].want_ce) {
            printf("FALLO: %s eligió canal=0x%04x CE=0x%04x (esperaba "
                   "0x%04x/0x%04x)\n", chip[i].name, chan_cls, ce_cls,
                   chip[i].want_chan, chip[i].want_ce);
            return -1;
        }
        if (gsp_rm_class_supported(chip[i].want_chan) != 1 ||
            gsp_rm_class_supported(0xdeadu) != 0) {
            printf("FALLO: supported() no distingue del catálogo (%s)\n",
                   chip[i].name);
            return -1;
        }
        printf("OK: catálogo %s (%u clases) → canal 0x%04x, CE 0x%04x\n",
               chip[i].name, chip[i].nr, chan_cls, ce_cls);
    }

    gsp_rm_classes_forget();
    return 0;
}

/* El plan del contexto de GR: aritmética pura sobre lo que contesta RM, y por eso
 * probable sin GPU. Aquí no hay mensajes de error posibles —un búfer del tamaño o
 * la alineación equivocados sale del silicio como "el gráfico lee fuera"—, así que
 * los números se comprueban uno a uno contra `r535_gr_get_ctxbuf_info`. */
static int check_grctx(void)
{
    NV2080_CTRL_INTERNAL_STATIC_GR_GET_CONTEXT_BUFFERS_INFO_PARAMS *info;
    struct gsp_grctx ctx;
    const struct gsp_grctx_buf *b;
    unsigned i;
    int n;

    info = calloc(1, sizeof(*info));
    if (!info) {
        printf("FALLO: sin memoria para la info de contexto\n");
        return -1;
    }
#define ENG0(prop) info->engineContextBuffersInfo[0].engine[(prop)]
#define ENG1(prop) info->engineContextBuffersInfo[1].engine[(prop)]
    ENG0(NV0080_CTX_PROP_GRAPHICS).size = 0x9000u;            /* el principal */
    ENG0(NV0080_CTX_PROP_GRAPHICS_PATCH).size = 0x1000u;
    ENG0(NV0080_CTX_PROP_GRAPHICS_BUNDLE_CB).size = 0x30000u; /* 192 KiB */
    ENG0(NV0080_CTX_PROP_GRAPHICS_PAGEPOOL).size = 0x8000u;   /* 32 KiB */
    ENG0(NV0080_CTX_PROP_GRAPHICS_ATTRIBUTE_CB).size = 0x1800000u; /* 24 MiB */
    ENG0(NV0080_CTX_PROP_GRAPHICS_RTV_CB_GLOBAL).size = 0u;   /* este chip no lo usa */
    ENG0(NV0080_CTX_PROP_GRAPHICS_FECS_EVENT).size = 0x1000u;
    ENG0(NV0080_CTX_PROP_GRAPHICS_PRIV_ACCESS_MAP).size = 0x10000u;
    /* Otro motor con tamaños distintos: si el plan ignorase `engine_idx`, cogería
     * estos y los números de abajo no cuadrarían. */
    ENG1(NV0080_CTX_PROP_GRAPHICS).size = 0x400000u;
    ENG1(NV0080_CTX_PROP_GRAPHICS_PATCH).size = 0x400000u;

    n = gsp_grctx_plan(info, 0, &ctx);
    /* Siete propiedades con tamaño (la de RTV va a cero y se salta) más el
     * duplicado del mapa de acceso privilegiado = 8. */
    if (n != 8) {
        printf("FALLO: el plan da %d búferes, esperaba 8\n", n);
        goto fallo;
    }

    /* MAIN: ALIGN(0x9000, 0x1000) + 64 páginas = 0x49000, que ya pasa de 64 KiB →
     * página 2^16. Sin el margen de las 64 páginas saldría 0x9000 y página 2^12:
     * el mismo búfer con la mitad de sitio y otra alineación. */
    b = &ctx.buf[0];
    if (b->buffer_id != NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_MAIN ||
        b->size != 0x49000u || b->page_shift != 16 || b->align != 0x10000u ||
        !b->init || b->global) {
        printf("FALLO: MAIN size=0x%llx page=2^%u align=0x%llx init=%u global=%u\n",
               (unsigned long long)b->size, b->page_shift,
               (unsigned long long)b->align, b->init, b->global);
        goto fallo;
    }
    /* ATTRIBUTE_CB: la alineación es `order_base_2(size)`, NO la página. 24 MiB
     * redondea a 32 MiB; con la página saldrían 2 MiB y RM no se quejaría. */
    for (i = 0, b = NULL; i < ctx.nr; i++) {
        if (ctx.buf[i].buffer_id == NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_ATTRIBUTE_CB) {
            b = &ctx.buf[i];
        }
    }
    if (!b || b->align != 0x2000000u || b->page_shift != 21 || !b->global || b->init) {
        printf("FALLO: ATTRIBUTE_CB align=0x%llx page=2^%u global=%u init=%u\n",
               b ? (unsigned long long)b->align : 0ull, b ? b->page_shift : 0,
               b ? b->global : 0, b ? b->init : 0);
        goto fallo;
    }
    /* El de RTV, con tamaño 0, no debe estar: reservarle una página "por si acaso"
     * es memoria que RM no espera en ese bufferId. */
    for (i = 0; i < ctx.nr; i++) {
        if (ctx.buf[i].buffer_id == NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_RTV_CB_GLOBAL) {
            printf("FALLO: el plan incluye un búfer de tamaño 0\n");
            goto fallo;
        }
    }
    /* El mapa de acceso privilegiado va dos veces, con el mismo tamaño y el segundo
     * con el bufferId "sin restricciones". Y es el único `ro`. */
    if (ctx.buf[ctx.nr - 2].buffer_id !=
            NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PRIV_ACCESS_MAP ||
        ctx.buf[ctx.nr - 1].buffer_id !=
            NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_UNRESTRICTED_PRIV_ACCESS_MAP ||
        ctx.buf[ctx.nr - 1].size != ctx.buf[ctx.nr - 2].size ||
        !ctx.buf[ctx.nr - 1].ro) {
        printf("FALLO: el PRIV_ACCESS_MAP no se duplica bien (%u/%u)\n",
               ctx.buf[ctx.nr - 2].buffer_id, ctx.buf[ctx.nr - 1].buffer_id);
        goto fallo;
    }
    /* Y que los dos juegos de números no se confundan: la propiedad 0x17 tiene que
     * salir como bufferId 9, no como 0x17. */
    for (i = 0, b = NULL; i < ctx.nr; i++) {
        if (ctx.buf[i].prop_id == NV0080_CTX_PROP_GRAPHICS_FECS_EVENT) {
            b = &ctx.buf[i];
        }
    }
    if (!b || b->buffer_id != NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_FECS_EVENT) {
        printf("FALLO: la propiedad 0x17 no traduce a bufferId 9\n");
        goto fallo;
    }

    {
        unsigned inherit_entries = 0;
        unsigned inherit_allocs = 0;

        for (i = 0; i < (unsigned)ctx.nr; i++) {
            if (ctx.buf[i].buffer_id ==
                    NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_UNRESTRICTED_PRIV_ACCESS_MAP) {
                continue;
            }
            inherit_entries++;
            if (!ctx.buf[i].global) {
                inherit_allocs++;
            }
        }
        if (inherit_entries != (unsigned)ctx.nr - 1u || inherit_allocs != 2u) {
            printf("FALLO: 2º promote hereda %u entradas / %u allocs "
                   "(esperaba %u / 2 MAIN+PATCH)\n", inherit_entries, inherit_allocs,
                   (unsigned)ctx.nr - 1u);
            goto fallo;
        }
        printf("OK: 2º PROMOTE_CTX hereda globales — %u entradas, %u VRAM nuevas "
               "(sin UNRESTRICTED_PRIV)\n", inherit_entries, inherit_allocs);
    }

    /* El otro motor, para demostrar que `engine_idx` se usa. */
    n = gsp_grctx_plan(info, 1, &ctx);
    if (n != 2 || ctx.buf[0].size != 0x400000u + 64u * 0x1000u) {
        printf("FALLO: el plan del motor 1 da %d búferes (MAIN 0x%llx)\n", n,
               n > 0 ? (unsigned long long)ctx.buf[0].size : 0ull);
        goto fallo;
    }
    /* Un motor fuera de rango no se inventa nada. */
    if (gsp_grctx_plan(info, NV2080_CTRL_INTERNAL_GR_MAX_ENGINES, &ctx) != -1) {
        printf("FALLO: el plan acepta un índice de motor fuera de rango\n");
        goto fallo;
    }
    /* GA107 run14: RM devuelve ATTRIBUTE_CB=0x851200 (no múltiplo de 4 KiB).
     * El mapeo debe redondear a 2^page_shift (21 → 0xA00000). */
    {
        uint64_t raw = 0x851200u;
        uint64_t gran = 1ull << 21;
        uint64_t map = (raw + gran - 1ull) & ~(gran - 1ull);

        if (map != 0xA00000u || (map & 0xfffu) != 0u) {
            printf("FALLO: map ATTRIBUTE_CB 0x%llx → 0x%llx\n",
                   (unsigned long long)raw, (unsigned long long)map);
            goto fallo;
        }
    }
#undef ENG0
#undef ENG1
    free(info);
    printf("OK: plan del contexto de GR — 8 búferes, MAIN +64 páginas, "
           "ATTRIBUTE_CB alineado a 32 MiB, tamaño 0 saltado, PRIV_ACCESS_MAP "
           "duplicado, map 0x851200→0xA00000\n");
    return 0;
/* Un solo sitio donde soltar `info`: el `free` + `return -1` estaba repetido ocho
 * veces, y la novena comprobación que se añadiera se lo dejaría. */
fallo:
    free(info);
    return -1;
}

/* PTOP: la topología que publica el chip, que es de donde tienen que salir las
 * direcciones de las runlists en vez de un índice inferido de la tabla de RM
 * (2026-07-28: la inferencia leyó 0xbadf5040 en GB205).
 *
 * La tabla simulada tiene tres motores con runlists distintas y un hueco en medio;
 * un parser que devolviera la primera entrada, o que ignorase la instancia, o que
 * cortase en el hueco, pasaría una prueba de un solo motor y aquí no. */
static int check_ptop(void)
{
    uint32_t runl = 0;
    uint32_t addr = 0;
    uint8_t type = 0;
    uint8_t inst = 0;
    int n;

    /* Primero el caso malo: sin GPU en el bus, PTOP no puede "encontrar" nada. Si
     * esto colase, el volcado del canal se pondría a leer registros a partir de
     * una tabla de ceros y diría cosas sobre un chip que no está. */
    fake_gpu_gone = 1;
    if (gsp_top_probe() != -1 ||
        gsp_top_runlist_of(GSP_TOP_TYPE_CE, 0, &runl, NULL) == 0) {
        printf("FALLO: PTOP se cree una tabla con la GPU fuera del bus\n");
        fake_gpu_gone = 0;
        return -1;
    }
    fake_gpu_gone = 0;

    n = gsp_top_probe();
    if (n != 3) {
        printf("FALLO: PTOP devuelve %d motores, esperaba 3\n", n);
        return -1;
    }
    if (gsp_top_runlist_of(GSP_TOP_TYPE_GR, 0, &runl, &addr) != 0 ||
        runl != FAKE_TOP_GR_RUNL || addr != 0x400000u) {
        printf("FALLO: PTOP GR0 runlist=0x%06x addr=0x%06x\n", runl, addr);
        return -1;
    }
    if (gsp_top_runlist_of(GSP_TOP_TYPE_CE, 0, &runl, &addr) != 0 ||
        runl != FAKE_TOP_CE0_RUNL || addr != 0x104000u) {
        printf("FALLO: PTOP CE0 runlist=0x%06x addr=0x%06x\n", runl, addr);
        return -1;
    }
    /* La que separa "lee la tabla" de "devuelve la primera que encuentra". */
    if (gsp_top_runlist_of(GSP_TOP_TYPE_CE, 1, &runl, &addr) != 0 ||
        runl != FAKE_TOP_CE1_RUNL || addr != 0x105000u) {
        printf("FALLO: PTOP CE1 runlist=0x%06x addr=0x%06x (¿ignora la "
               "instancia?)\n", runl, addr);
        return -1;
    }
    if (gsp_top_runlist_of(GSP_TOP_TYPE_NVDEC, 0, &runl, NULL) == 0) {
        printf("FALLO: PTOP inventa un NVDEC que no está en la tabla\n");
        return -1;
    }

    /* Y la traducción desde los engineType de RM, que es como llega la pregunta. */
    if (gsp_top_type_of_engine(NV2080_ENGINE_TYPE_GR0, &type, &inst) != 0 ||
        type != GSP_TOP_TYPE_GR || inst != 0) {
        printf("FALLO: GR0 de RM no traduce a PTOP (0x%02x/%u)\n", type, inst);
        return -1;
    }
    if (gsp_top_type_of_engine(NV2080_ENGINE_TYPE_COPY0 + 1u, &type, &inst) != 0 ||
        type != GSP_TOP_TYPE_CE || inst != 1) {
        printf("FALLO: COPY1 de RM no traduce a CE1 (0x%02x/%u)\n", type, inst);
        return -1;
    }
    /* Un motor que no sabemos traducir tiene que fallar, no caer en el 0 — que es
     * justo GR y mandaría el volcado a los registros del gráfico. */
    if (gsp_top_type_of_engine(100u, &type, &inst) == 0) {
        printf("FALLO: un engineType desconocido traduce a 0x%02x/%u\n", type, inst);
        return -1;
    }
    {
        uint32_t pick = gsp_top_pick_ce_engine();

        if (pick != NV2080_ENGINE_TYPE_COPY0 + 1u) {
            printf("FALLO: pick_ce_engine=%u (esperaba COPY1=%u, runlist ≠ GR0)\n",
                   pick, NV2080_ENGINE_TYPE_COPY0 + 1u);
            return -1;
        }
    }
    printf("OK: PTOP — 3 motores (GR0, CE0, CE1 con runlists 0x%06x/0x%06x), "
           "hueco saltado, picker COPY1, y sin GPU no inventa tabla\n",
           FAKE_TOP_CE0_RUNL, FAKE_TOP_CE1_RUNL);
    return 0;
}

struct nvfw_bin_hdr_hc {
    uint32_t bin_magic;
    uint32_t bin_ver;
    uint32_t bin_size;
    uint32_t header_offset;
    uint32_t data_offset;
    uint32_t data_size;
};

struct nvfw_hs_header_v2_hc {
    uint32_t sig_prod_offset;
    uint32_t sig_prod_size;
    uint32_t patch_loc;
    uint32_t patch_sig;
    uint32_t meta_data_offset;
    uint32_t meta_data_size;
    uint32_t num_sig;
    uint32_t header_offset;
    uint32_t header_size;
};

static int check_hs_v2_blob(const char *path, unsigned expect_data_off)
{
    FILE *f = fopen(path, "rb");
    uint8_t *data;
    long sz;
    const struct nvfw_bin_hdr_hc *hdr;
    const struct nvfw_hs_header_v2_hc *hshdr;
    unsigned loc;
    unsigned cnt;
    unsigned sig_size;
    unsigned patch_off;

    if (!f)
        return -1;
    fseek(f, 0, SEEK_END);
    sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    data = malloc((size_t)sz);
    if (!data || fread(data, 1, (size_t)sz, f) != (size_t)sz) {
        fclose(f);
        free(data);
        return -1;
    }
    fclose(f);

    hdr = (const struct nvfw_bin_hdr_hc *)data;
    if (hdr->bin_magic != 0x000010deu || hdr->data_offset != expect_data_off) {
        printf("FALLO: %s data_offset=0x%x (esperaba 0x%x)\n", path,
               hdr->data_offset, expect_data_off);
        free(data);
        return -1;
    }
    hshdr = (const struct nvfw_hs_header_v2_hc *)(data + hdr->header_offset);
    loc = *(const unsigned *)(data + hshdr->patch_loc);
    cnt = *(const unsigned *)(data + hshdr->num_sig);
    sig_size = cnt ? hshdr->sig_prod_size / cnt : 0;
    patch_off = loc;
    if (patch_off + sig_size > hdr->data_size) {
        printf("FALLO: %s patch fuera del payload\n", path);
        free(data);
        return -1;
    }
    {
        unsigned i;
        int zero = 1;
        for (i = 0; i < 16 && i < sig_size; i++) {
            if (data[hdr->data_offset + patch_off + i]) {
                zero = 0;
                break;
            }
        }
        if (!zero) {
            printf("FALLO: %s hueco firma no está vacío en contenedor\n", path);
            free(data);
            return -1;
        }
    }
    printf("OK: HS v2 %s payload@0x%x patch=0x%x sigs=%u\n", path,
           hdr->data_offset, patch_off, cnt);
    free(data);
    return 0;
}

static int check_hs_v2_ampere_payload(void)
{
    const char *root = getenv("SOSO_ROOT");
    char path[512];

    if (!root)
        root = ".";
    snprintf(path, sizeof(path), "%s/rootfs/lib/firmware/nvidia/ga102/gsp/"
                               "booter_load-570.144.bin", root);
    if (check_hs_v2_blob(path, 0x378u) != 0)
        return -1;
    snprintf(path, sizeof(path), "%s/rootfs/lib/firmware/nvidia/ga102/acr/"
                               "ucode_ahesasc.bin", root);
    if (check_hs_v2_blob(path, 0x600u) != 0)
        return -1;
    return 0;
}

/* GA107: PTOP sigue reportando 0x087000; el booter Ampere usa 0x840000
 * (ga102_sec2_new). Orden boot: FWSEC-FRTS → booter (sin ACR/AHESASC previo;
 * ver l6-g3-gsp-hostcheck.sh). */
static int check_ampere_sec2_falcon_base(void)
{
    unsigned ptop_base;
    unsigned booter_base = LX_FLCN_SEC2_BASE_LEGACY;

    fake_ptop_active = fake_ptop_ga107;
    fake_ptop_active_words = GA107_PTOP_WORDS;
    if (gsp_top_probe() < 1) {
        printf("FALLO: PTOP GA107 sin motores\n");
        fake_ptop_active = NULL;
        fake_ptop_active_words = 0;
        return -1;
    }
    ptop_base = gsp_top_falcon_base(GSP_TOP_TYPE_SEC2, 0, LX_FLCN_SEC2_BASE_LEGACY);
    fake_ptop_active = NULL;
    fake_ptop_active_words = 0;
    if (ptop_base != 0x087000u) {
        printf("FALLO: PTOP GA107 SEC2 addr=0x%x (esperaba 0x087000)\n",
               ptop_base);
        return -1;
    }
    if (booter_base != LX_FLCN_SEC2_BASE_LEGACY) {
        printf("FALLO: booter Ampere SEC2=0x%x (esperaba 0x840000)\n",
               booter_base);
        return -1;
    }
    if (booter_base == ptop_base) {
        printf("FALLO: booter no debe usar la dirección PTOP de SEC2\n");
        return -1;
    }
    printf("OK: Ampere SEC2 — PTOP=0x087000, booter=0x840000 (override Linux)\n");
    return 0;
}

static int check_pramin_family(uint32_t boot0, uint16_t devid, const char *label)
{
    uint32_t win_reg;

    gsp_nv_family_set(boot0, devid);
    gsp_pramin_invalidate();

    win_reg = (gsp_nv_family_current() == NV_FAM_BLACKWELL) ?
              FAKE_PRAMIN_WINDOW_GB : FAKE_PRAMIN_WINDOW_NV50;

    gsp_mmio_wr32(win_reg, 0x10u);
    if (gsp_mmio_rd32(win_reg) != 0x10u) {
        printf("FALLO: PRAMIN %s ventana readback (escribí 0x10, leí 0x%x)\n",
               label, gsp_mmio_rd32(win_reg));
        return -1;
    }

    gsp_pramin_wr32(0x1234ull, 0xdeadbeefu);
    if (gsp_pramin_rd32(0x1234ull) != 0xdeadbeefu) {
        printf("FALLO: PRAMIN %s rd/wr (escribí deadbeef, leí %08x)\n",
               label, gsp_pramin_rd32(0x1234ull));
        return -1;
    }

    if (gsp_nv_family_current() == NV_FAM_BLACKWELL) {
        gsp_pramin_wr32(0xfffcull, 0xaaaau);
        gsp_pramin_wr32(0x10000ull, 0xbbbau);
        if (gsp_pramin_rd32(0xfffcull) != 0xaaaau ||
            gsp_pramin_rd32(0x10000ull) != 0xbbbau) {
            printf("FALLO: PRAMIN %s cruce 64K (0xfffc=%08x 0x10000=%08x)\n",
                   label, gsp_pramin_rd32(0xfffcull),
                   gsp_pramin_rd32(0x10000ull));
            return -1;
        }
    } else {
        gsp_pramin_wr32(0x000ffffcull, 0xaaaau);
        gsp_pramin_wr32(0x00100000ull, 0xbbbau);
        if (gsp_pramin_rd32(0x000ffffcull) != 0xaaaau ||
            gsp_pramin_rd32(0x00100000ull) != 0xbbbau) {
            printf("FALLO: PRAMIN %s cruce 1MiB (0xffffc=%08x 0x100000=%08x)\n",
                   label, gsp_pramin_rd32(0x000ffffcull),
                   gsp_pramin_rd32(0x00100000ull));
            return -1;
        }
    }

    if (!gsp_pramin_alive()) {
        printf("FALLO: gsp_pramin_alive (%s)\n", label);
        return -1;
    }

    printf("OK: PRAMIN %s ventana BAR0 + rd/wr\n", label);
    return 0;
}

static int check_pramin(void)
{
    if (check_pramin_family(0xb74000a1u, 0x249cu, "Ampere GA107") != 0)
        return -1;
    if (check_pramin_family(0x1b5000a1u, 0x2f18u, "Blackwell GB205") != 0)
        return -1;
    return 0;
}

/* Recorrido de las tablas de BAR1. Aquí no hay tarjeta, así que la cadena se
 * PLANTA en la VRAM falsa y se comprueba que el recorrido la lee entera y que
 * se para donde tiene que pararse. Lo que se valida es la aritmética de niveles
 * (índices, tamaño de entrada, la mitad alta de la PDE doble del PD0) y la
 * decodificación de APERTURE — que es justo lo que no se puede depurar en
 * hardware sin gastar un ciclo de VFIO por error de un bit. */
static int check_bar1_walk(void)
{
    struct gsp_bar1 b;
    struct gsp_bar1_step steps[GSP_BAR1_LEVELS];
    const uint64_t pd3 = 0x10000ull, pd2 = 0x11000ull, pd1 = 0x12000ull;
    const uint64_t pd0 = 0x13000ull, spt = 0x14000ull, page = 0x2a000ull;
    int n;

    gsp_pramin_invalidate();
    (void)gsp_pramin_alive();

    /* PDE: APERTURE (2:1) = 1 (VRAM), PCF = 2, ADDRESS 51:12. El bit 0 NO se
     * pone: ahí `IS_PTE` convertiría el puntero en una traducción final. */
#define FAKE_PDE(addr) (((uint64_t)(addr) & 0x000ffffffffff000ull) | (1ull << 1) | (2ull << 3))
    {
        struct { uint64_t at; uint64_t val; } e[] = {
            { pd3 + 0u,  FAKE_PDE(pd2) },       /* PD3[0] → PD2 */
            { pd2 + 0u,  FAKE_PDE(pd1) },       /* PD2[0] → PD1 */
            { pd1 + 0u,  FAKE_PDE(pd0) },       /* PD1[0] → PD0 */
            /* PD0 son 16 B por entrada: el PDE de 4 KiB va en la mitad ALTA. */
            { pd0 + 8u,  FAKE_PDE(spt) },
            /* Hoja: aquí el bit 0 sí es VALID y VRAM se codifica como 0. */
            { spt + 0u,  ((uint64_t)page & 0x000ffffffffff000ull) | 1ull | (0x10ull << 3) },
        };
        unsigned i;

        for (i = 0; i < sizeof(e) / sizeof(e[0]); i++) {
            gsp_pramin_wr32(e[i].at, (uint32_t)e[i].val);
            gsp_pramin_wr32(e[i].at + 4u, (uint32_t)(e[i].val >> 32));
        }
    }

    if (gsp_bar1_init(&b, 0xf0000000ull, 256ull << 20, pd3) != 0) {
        printf("FALLO: gsp_bar1_init con raíz y apertura buenas\n");
        return -1;
    }
    n = gsp_bar1_walk(&b, 0, steps, GSP_BAR1_LEVELS);
    if (n != (int)GSP_BAR1_LEVELS) {
        printf("FALLO: el recorrido de BAR1 dio %d pasos, esperaba %u\n",
               n, GSP_BAR1_LEVELS);
        return -1;
    }
    if (steps[0].table != pd3 || steps[1].table != pd2 || steps[2].table != pd1 ||
        steps[3].table != pd0 || steps[4].table != spt || steps[4].next != page) {
        printf("FALLO: el recorrido no siguió la cadena plantada "
               "(%llx %llx %llx %llx %llx → %llx)\n",
               (unsigned long long)steps[0].table, (unsigned long long)steps[1].table,
               (unsigned long long)steps[2].table, (unsigned long long)steps[3].table,
               (unsigned long long)steps[4].table, (unsigned long long)steps[4].next);
        return -1;
    }

    /* Una rama sin construir corta el recorrido, y no se sigue leyendo: un PDE a
     * cero apunta a la VRAM 0, que en la tarjeta tiene datos de RM y daría un
     * volcado con pinta de tabla. */
    gsp_pramin_wr32(pd1 + 0u, 0u);
    gsp_pramin_wr32(pd1 + 4u, 0u);
    n = gsp_bar1_walk(&b, 0, steps, GSP_BAR1_LEVELS);
    if (n != 3 || steps[2].aperture != 0u) {
        printf("FALLO: con PD1 inválido el recorrido dio %d pasos (ap=%u)\n",
               n, n >= 3 ? steps[2].aperture : 99u);
        return -1;
    }

    /* Y una tabla en sysmem también corta: PRAMIN sólo llega a VRAM. */
    {
        /* APERTURE = 2 (SYS_COH), no 1: el campo son los bits 2:1 enteros, así
         * que hay que ponerlo, no añadirle un bit al de VRAM. */
        uint64_t sys_pde = ((uint64_t)pd0 & 0x000ffffffffff000ull) |
                           (2ull << 1) | (1ull << 3);

        gsp_pramin_wr32(pd1 + 0u, (uint32_t)sys_pde);
        gsp_pramin_wr32(pd1 + 4u, (uint32_t)(sys_pde >> 32));
        n = gsp_bar1_walk(&b, 0, steps, GSP_BAR1_LEVELS);
        if (n != 3 || steps[2].aperture != 2u) {
            printf("FALLO: con PD1 en sysmem el recorrido dio %d pasos (ap=%u)\n",
                   n, n >= 3 ? steps[2].aperture : 99u);
            return -1;
        }
    }

    /* Raíz a cero: no hay recorrido que valga. */
    if (gsp_bar1_init(&b, 0xf0000000ull, 0, 0) == 0) {
        printf("FALLO: gsp_bar1_init aceptó bar1PdeBase=0\n");
        return -1;
    }
#undef FAKE_PDE

    printf("OK: BAR1 — recorrido PD3→SPT de la cadena de RM, y se para en rama "
           "inválida o en sysmem\n");
    return 0;
}

/* El mapeo de BAR1 escribe PDEs ENCIMA de las tablas de RM. Eso no se puede
 * depurar en la tarjeta: si el índice o el encoding están mal, se pisa un mapeo
 * que RM usa y la GPU se cuelga — a un ciclo de VFIO por intento. Aquí la cadena
 * se planta en la VRAM falsa con la MISMA forma que la real (PD3[0]→PD2[0]→PD1[0]
 * en VRAM, medida el 2026-08-01) y se comprueba entrada por entrada dónde
 * escribe, que se niegue cuando la ventana está ocupada, y que el unmap deshaga. */
static int check_bar1_map(void)
{
    struct gsp_bar1 b;
    const uint64_t pd3 = 0x20000ull, pd2 = 0x21000ull, pd1 = 0x22000ull;
    const uint64_t apertura = 0xf800000000ull;
    const uint64_t tam = 16ull * 1024 * 1024 * 1024;      /* 16 GiB, como la GB205 */
    const uint64_t vram = 0x7000000ull;                   /* la VRAM a exponer */
    uint64_t va, off, e;
    uint32_t i1, i0, is;

    gsp_pramin_invalidate();
    (void)gsp_pramin_alive();

#define PDE_VRAM(addr) (((uint64_t)(addr) & 0x000ffffffffff000ull) | (1ull << 1) | (2ull << 3))
    /* Cadena de RM hasta PD1, igual que en silicio. PD1[31] se queda inválido. */
    gsp_pramin_wr32(pd3, (uint32_t)PDE_VRAM(pd2));
    gsp_pramin_wr32(pd3 + 4u, (uint32_t)(PDE_VRAM(pd2) >> 32));
    gsp_pramin_wr32(pd2, (uint32_t)PDE_VRAM(pd1));
    gsp_pramin_wr32(pd2 + 4u, (uint32_t)(PDE_VRAM(pd1) >> 32));
#undef PDE_VRAM

    memset(&b, 0, sizeof(b));
    if (gsp_bar1_init(&b, apertura, tam, pd3) != 0) {
        printf("FALLO: gsp_bar1_init con la apertura de 16 GiB\n");
        return -1;
    }
    b.window_va = (tam - GSP_BAR1_WINDOW_BYTES) & ~(GSP_BAR1_WINDOW_BYTES - 1ull);
    i1 = (uint32_t)((b.window_va >> 29) & 0x1ffu);
    i0 = (uint32_t)((b.window_va >> 21) & 0xffu);
    is = (uint32_t)((b.window_va >> 12) & 0x1ffu);
    if (i1 != 31u || i0 != 255u || is != 0u) {
        printf("FALLO: la ventana alta cae en PD1[%u] PD0[%u] SPT[%u]; esperaba "
               "31/255/0\n", i1, i0, is);
        return -1;
    }

    va = gsp_bar1_map(&b, vram, 4096);
    if (va != b.window_va) {
        printf("FALLO: gsp_bar1_map devolvió 0x%llx, esperaba 0x%llx\n",
               (unsigned long long)va, (unsigned long long)b.window_va);
        return -1;
    }
    /* El enlace tiene que estar en la tabla de RM (VRAM) y apuntar a NUESTRO PD0
     * en sysmem: aperture 2, no 1. Un 1 aquí mandaría a la MMU a leer tablas a
     * una dirección de VRAM que no existe. */
    e = (uint64_t)gsp_pramin_rd32(pd1 + (uint64_t)i1 * 8u) |
        ((uint64_t)gsp_pramin_rd32(pd1 + (uint64_t)i1 * 8u + 4u) << 32);
    if (((e >> 1) & 3u) != 2u ||
        (e & 0x000ffffffffff000ull) != (b.pd0.phys & 0x000ffffffffff000ull)) {
        printf("FALLO: PD1[%u] = 0x%016llx no apunta a nuestro PD0 (0x%llx) en "
               "sysmem\n", i1, (unsigned long long)e,
               (unsigned long long)b.pd0.phys);
        return -1;
    }
    /* Y la PDE doble del PD0: la mitad ALTA es la de 4 KiB; la baja a cero. */
    {
        const uint64_t *pd0 = (const uint64_t *)b.pd0.va;

        if (pd0[(unsigned long)i0 * 2u] != 0 ||
            ((pd0[(unsigned long)i0 * 2u + 1u] >> 1) & 3u) != 2u) {
            printf("FALLO: PD0[%u] mal: baja=0x%llx alta=0x%llx\n", i0,
                   (unsigned long long)pd0[(unsigned long)i0 * 2u],
                   (unsigned long long)pd0[(unsigned long)i0 * 2u + 1u]);
            return -1;
        }
    }
    /* La hoja: PTE de VRAM (bit 0 VALID, aperture 0) a la física pedida. */
    {
        const uint64_t *spt = (const uint64_t *)b.spt.va;

        if (!(spt[is] & 1ull) || ((spt[is] >> 1) & 3u) != 0u ||
            (spt[is] & 0x000ffffffffff000ull) != vram) {
            printf("FALLO: SPT[%u] = 0x%016llx no es un PTE de VRAM a 0x%llx\n",
                   is, (unsigned long long)spt[is], (unsigned long long)vram);
            return -1;
        }
    }

    /* Con la ventana ya enlazada, un segundo mapeo tiene que NEGARSE: es lo que
     * protege las estructuras de RM de que un segundo llamante las pise. */
    off = gsp_bar1_map(&b, vram + 0x10000ull, 4096);
    if (off != 0) {
        printf("FALLO: gsp_bar1_map pisó una ventana ya válida (devolvió 0x%llx)\n",
               (unsigned long long)off);
        return -1;
    }

    gsp_bar1_unmap(&b);
    e = (uint64_t)gsp_pramin_rd32(pd1 + (uint64_t)i1 * 8u) |
        ((uint64_t)gsp_pramin_rd32(pd1 + (uint64_t)i1 * 8u + 4u) << 32);
    if (e != 0) {
        printf("FALLO: tras unmap PD1[%u] sigue a 0x%016llx\n", i1,
               (unsigned long long)e);
        return -1;
    }

    printf("OK: BAR1 mapeo — ventana 0x%llx en PD1[31]/PD0[255]/SPT[0], PDE a "
           "sysmem, PTE a VRAM, se niega a pisar y el unmap deshace\n",
           (unsigned long long)b.window_va);
    return 0;
}

/* GA107 run13: caminar un PDB que no es bar1PdeBase devolvía
 * `0xbad0fb2fbad0fb2e` y el selftest (más el reintento con bar2Pde) escribía
 * encima. Linux GSP envuelve `rm_bar1_pdb` y no parchea. */
static int check_bar1_refuse_poison(void)
{
    struct gsp_bar1 b;
    struct gsp_bar1_step steps[GSP_BAR1_LEVELS];
    const uint64_t pd3 = 0x30000ull;
    const uint64_t poison = 0xbad0fb2fbad0fb2eull;
    uint64_t va;
    uint32_t lo, hi;
    int n;

    if (!gsp_bar1_pri_poison(poison)) {
        printf("FALLO: 0xbad0fb2fbad0fb2e no se reconoce como veneno PRI\n");
        return -1;
    }
    if (!gsp_bar1_pri_poison(0x00000000badf5040ull)) {
        printf("FALLO: 0xbadf5040 no se reconoce como veneno PRI\n");
        return -1;
    }
    if (gsp_bar1_pri_poison(0x00000003f3c2a000ull)) {
        printf("FALLO: bar1PdeBase 0x3f3c2a000 marcado como veneno PRI\n");
        return -1;
    }

    memset(&b, 0, sizeof(b));
    if (gsp_bar1_init(&b, 0xf0000000ull, 256ull << 20, 0xbad0fb2fbad0f000ull) == 0) {
        printf("FALLO: gsp_bar1_init aceptó un PDB PRI-veneno\n");
        return -1;
    }

    gsp_pramin_invalidate();
    (void)gsp_pramin_alive();
    gsp_pramin_wr32(pd3, (uint32_t)poison);
    gsp_pramin_wr32(pd3 + 4u, (uint32_t)(poison >> 32));
    lo = gsp_pramin_rd32(pd3);
    hi = gsp_pramin_rd32(pd3 + 4u);

    memset(&b, 0, sizeof(b));
    if (gsp_bar1_init(&b, 0xf0000000ull, 256ull << 20, pd3) != 0) {
        printf("FALLO: gsp_bar1_init con PDB bueno y PD3[0] veneno\n");
        return -1;
    }
    n = gsp_bar1_walk(&b, GSP_BAR1_WINDOW_BYTES, steps, GSP_BAR1_LEVELS);
    if (n < 1 || !gsp_bar1_pri_poison(steps[0].entry)) {
        printf("FALLO: walk no se paró en PD3 veneno (n=%d entry=0x%llx)\n",
               n, n >= 1 ? (unsigned long long)steps[0].entry : 0ull);
        return -1;
    }

    b.window_va = GSP_BAR1_WINDOW_BYTES;
    va = gsp_bar1_map(&b, 0x7000000ull, 4096);
    if (va != 0) {
        printf("FALLO: gsp_bar1_map escribió sobre PD3 veneno (va=0x%llx)\n",
               (unsigned long long)va);
        return -1;
    }
    if (gsp_pramin_rd32(pd3) != lo || gsp_pramin_rd32(pd3 + 4u) != hi) {
        printf("FALLO: gsp_bar1_map cambió PD3 veneno (era 0x%08x%08x)\n", hi, lo);
        return -1;
    }

    printf("OK: BAR1 — PDB/entrada PRI-veneno no se escribe\n");
    return 0;
}

static int check_rc_triggered(void)
{
    rpc_rc_triggered_v17_02 msg;

    memset(&msg, 0, sizeof(msg));
    msg.nv2080EngineType = NV2080_ENGINE_TYPE_COPY0;
    msg.chid = 0;
    msg.exceptType = 32;
    msg.scope = 1;
    msg.partitionAttributionId = 0;
    msg.mmuFaultAddrLo = 0x1000u;
    msg.mmuFaultType = 3;

    if (gsp_rpc_rc_triggered_log(&msg, sizeof(msg)) != 0) {
        printf("FALLO: gsp_rpc_rc_triggered_log\n");
        return -1;
    }
    if (gsp_rpc_rc_triggered_log(&msg, 4u) == 0) {
        printf("FALLO: rc parser aceptó payload corto\n");
        return -1;
    }
    if (strcmp(mmu_fault_type_name(0), "PDE") != 0) {
        printf("FALLO: mmu_fault_type_name(0)=%s (esperaba PDE)\n",
               mmu_fault_type_name(0));
        return -1;
    }
    printf("OK: RC_TRIGGERED decodificado (engn=%08x chid=%u type=%u PBDMA_ERROR)\n",
           msg.nv2080EngineType, msg.chid, msg.exceptType);
    return 0;
}

static int check_doorbell_kick_by_family(void)
{
    uint32_t kick_bw = gsp_chan_doorbell_kick(NV_FAM_BLACKWELL, FAKE_DOORBELL_TOKEN);
    uint32_t kick_amp = gsp_chan_doorbell_kick(NV_FAM_AMPERE, FAKE_DOORBELL_TOKEN);

    if (kick_bw != FAKE_DOORBELL_KICK) {
        printf("FALLO: kick gb20x=0x%08x (esperaba 0x%08x)\n", kick_bw,
               FAKE_DOORBELL_KICK);
        return -1;
    }
    if (kick_amp != FAKE_DOORBELL_TOKEN ||
        (kick_amp & NV_VF_DOORBELL_RUNLIST_DOORBELL_ENABLE) != 0) {
        printf("FALLO: kick Ampere=0x%08x (esperaba 0x%08x sin bit 30)\n",
               kick_amp, FAKE_DOORBELL_TOKEN);
        return -1;
    }
    printf("OK: doorbell kick chip-aware (gb20x bit30 ON, Ampere sin bit30)\n");
    return 0;
}

static int check_doorbell_ampere_copy2_resolved(void)
{
    uint32_t token = 0x00000001u;
    uint32_t dbcfg = 0x00010000u;
    uint32_t kick = gsp_chan_doorbell_kick_resolved(NV_FAM_AMPERE, token, dbcfg);

    if (kick != 0x00010001u) {
        printf("FALLO: COPY2 kick=0x%08x (esperaba 0x00010001)\n", kick);
        return -1;
    }
    printf("OK: Ampere COPY2 dbcfg doorbell=1 token runlist=0 → kick 0x00010001\n");
    return 0;
}

/* RTX 3050 Mobile: boot0 muerto no puede clasificarse como Blackwell/FMC. */
static int check_ga107_dead_boot0(void)
{
    enum nv_family fam = gsp_nv_family_of(0xffffffffu, 0x249cu);

    if (fam != NV_FAM_AMPERE) {
        printf("FALLO: 0x249c + boot0 all-ones → familia %d (esperaba Ampere)\n",
               (int)fam);
        return -1;
    }
    if (gsp_nv_ampere_chip_name(0x249cu)[0] != 'g' ||
        strcmp(gsp_nv_ampere_chip_name(0x249cu), "ga107") != 0) {
        printf("FALLO: chip 0x249c=%s (esperaba ga107)\n",
               gsp_nv_ampere_chip_name(0x249cu));
        return -1;
    }
    if (!gsp_nv_boot0_valid(0xffffffffu)) {
        printf("OK: boot0 all-ones no es válido para arch\n");
    }
    printf("OK: 0x249c + boot0 muerto → Ampere/ga107 (no Blackwell/FMC)\n");
    return 0;
}

/* Valor de un método dentro de un pushbuffer ya codificado. Recorre las
 * cabeceras como haría el host (INCR con `count` datos detrás) en vez de asumir
 * un offset fijo: así el banco no se rompe cada vez que se reordena la copia. */
static int pb_method_value(const uint32_t *pb, unsigned dwords, unsigned mthd,
                           uint32_t *out)
{
    unsigned i = 0;

    while (i < dwords) {
        uint32_t hdr = pb[i];
        unsigned count = (hdr >> 16) & 0x1fffu;
        unsigned m = (hdr & 0xfffu) << 2;

        if (count == 0u || i + count >= dwords + 1u) {
            return -1;
        }
        if (m == mthd) {
            *out = pb[i + 1u];
            return 0;
        }
        i += 1u + count;
    }
    return -1;
}

static int check_g4e_chan_ce(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_cmdq q;
    struct gsp_vmm v;
    struct gsp_vram pool;
    struct gsp_static_info vram_si;
    struct gsp_chan chan;
    struct gsp_chan chan_gr;
    struct gsp_ce ce;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);
    unsigned char *cmdq_base = (unsigned char *)lo->shm.va + lo->cmdq_offset;
    rpc_gsp_rm_alloc alloc_ok;
    unsigned char ctrl_ok[sizeof(rpc_gsp_rm_control)];
    unsigned char ctrl_mthdbuf[sizeof(rpc_gsp_rm_control) +
                               sizeof(NV2080_CTRL_CE_GET_FAULT_METHOD_BUFFER_SIZE_PARAMS)];
    unsigned char ctrl_token[sizeof(rpc_gsp_rm_control) +
                             sizeof(NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS)];
    /* Uno por canal: las respuestas se encolan TODAS antes de las llamadas, así que
     * dos canales con tokens distintos necesitan dos búferes vivos a la vez. */
    unsigned char ctrl_token_gr[sizeof(rpc_gsp_rm_control) +
                                sizeof(NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS)];
    unsigned char ctrl_token_golden[sizeof(rpc_gsp_rm_control) +
                                    sizeof(NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS)];
    /* 1664 B de tamaños de búferes de contexto + el wrapper: 1688, que sigue
     * cabiendo en la página de un elemento de cola. */
    unsigned char ctrl_grctx[sizeof(rpc_gsp_rm_control) +
                             sizeof(NV2080_CTRL_INTERNAL_STATIC_GR_GET_CONTEXT_BUFFERS_INFO_PARAMS)];
    /* RPCs del golden oneinit (r535_gr_oneinit) insertados antes del query/promote
     * de usuario: vaspace×2, canal×4, query, promote. */
#define G4E_GOLDEN_RPC_NR        8u
#define G4E_SLOT_GRCTX_QUERY     (15u + G4E_GOLDEN_RPC_NR)
#define G4E_SLOT_USER_PROMOTE    (16u + G4E_GOLDEN_RPC_NR)
#define G4E_SLOT_COMPUTE_ALLOC   (17u + G4E_GOLDEN_RPC_NR)
    uint32_t base, wptr0;
    unsigned pb_off = 0, pb_len = 0;
    unsigned i;
    const uint32_t test_pat = 0x300u;

    printf("sizeof NV_CHANNEL_ALLOC_PARAMS=%zu Nvc56fControl=%zu\n",
           sizeof(NV_CHANNEL_ALLOC_PARAMS), sizeof(Nvc56fControl));

    /* GB205 simulado: el kick del doorbell lleva bit 30 solo en Blackwell. */
    gsp_nv_family_set(0x1b5000a1u, 0x2f18u);

    /* Este escenario corre SIN catálogo a propósito: comprueba la ruta a ciegas,
     * que es la que se toma si GET_CLASSLIST_V2 falla en hardware. Las clases
     * esperadas son entonces las primeras candidatas (las B de Blackwell). */
    gsp_rm_classes_forget();

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: rpc_init (g4e)\n"); return -1; }
    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: cmdq_init (g4e)\n"); return -1; }

    memset(&alloc_ok, 0, sizeof(alloc_ok));
    memset(ctrl_ok, 0, sizeof(ctrl_ok));
    base = *rpc.rptr;
    for (i = 0; i < 4; i++) {
        fake_rpc_post_payload(lo, (base + i) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                              0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    }
    fake_rpc_post_payload(lo, (base + 4) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    /* Lo primero que hace ahora el canal es preguntar el tamaño del method
     * buffer, así que hay que contestarle antes que a su RM_ALLOC. Por eso todos
     * los índices de aquí abajo van uno más que antes. */
    {
        NV2080_CTRL_CE_GET_FAULT_METHOD_BUFFER_SIZE_PARAMS *sz =
            (NV2080_CTRL_CE_GET_FAULT_METHOD_BUFFER_SIZE_PARAMS *)
                (ctrl_mthdbuf + sizeof(rpc_gsp_rm_control));

        memset(ctrl_mthdbuf, 0, sizeof(ctrl_mthdbuf));
        sz->size = FAKE_MTHDBUF_SIZE;
        fake_rpc_post_payload(lo, (base + 5) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                              0, ctrl_mthdbuf, (uint32_t)sizeof(ctrl_mthdbuf));
    }
    /* Canal, sus tres controles de arranque, CE y compute.
     *
     * Reservar el canal no lo arranca: detrás van BIND, GPFIFO_SCHEDULE y
     * GET_WORK_SUBMIT_TOKEN, en ese orden. Los dos primeros no devuelven nada y
     * les basta la cabecera; el tercero SÍ trae payload, y si no se le contesta
     * con un token el canal se declara sin arrancar y no se encola nada. */
    fake_rpc_post_payload(lo, (base + 6) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                          0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    fake_rpc_post_payload(lo, (base + 7) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    fake_rpc_post_payload(lo, (base + 8) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    {
        NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS *tk =
            (NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS *)
                (ctrl_token + sizeof(rpc_gsp_rm_control));

        memset(ctrl_token, 0, sizeof(ctrl_token));
        tk->workSubmitToken = FAKE_DOORBELL_TOKEN | 1u;
        fake_rpc_post_payload(lo, (base + 9) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                              0, ctrl_token, (uint32_t)sizeof(ctrl_token));
    }
    fake_rpc_post_payload(lo, (base + 10) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                          0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    /* Canal GR0: el tamaño del method buffer ya está en cache de fifo
     * (`r535_fifo_ctor`); no hay segundo CE_GET_FAULT_METHOD_BUFFER_SIZE. */
    fake_rpc_post_payload(lo, (base + 11) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                          0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    fake_rpc_post_payload(lo, (base + 12) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    fake_rpc_post_payload(lo, (base + 13) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    {
        /* Tras rsvd_chids=1: COPY0 chid=1, GR0 chid=2 — tokens distintos. */
        NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS *tk =
            (NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS *)
                (ctrl_token_gr + sizeof(rpc_gsp_rm_control));

        memset(ctrl_token_gr, 0, sizeof(ctrl_token_gr));
        tk->workSubmitToken = FAKE_DOORBELL_TOKEN | 2u;
        fake_rpc_post_payload(lo, (base + 14) % 63,
                              NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                              0, ctrl_token_gr, (uint32_t)sizeof(ctrl_token_gr));
    }
    /* Golden oneinit (r535_gr_oneinit): vaspace, canal GR, query y promote. */
    fake_rpc_post_payload(lo, (base + 15) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                          0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    fake_rpc_post_payload(lo, (base + 16) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    fake_rpc_post_payload(lo, (base + 17) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                          0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    fake_rpc_post_payload(lo, (base + 18) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    fake_rpc_post_payload(lo, (base + 19) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    {
        NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS *tk =
            (NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS *)
                (ctrl_token_golden + sizeof(rpc_gsp_rm_control));

        memset(ctrl_token_golden, 0, sizeof(ctrl_token_golden));
        tk->workSubmitToken = FAKE_DOORBELL_TOKEN | 3u;
        fake_rpc_post_payload(lo, (base + 20) % 63,
                              NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                              0, ctrl_token_golden, (uint32_t)sizeof(ctrl_token_golden));
    }
    {
        NV2080_CTRL_INTERNAL_STATIC_GR_GET_CONTEXT_BUFFERS_INFO_PARAMS *gi =
            (NV2080_CTRL_INTERNAL_STATIC_GR_GET_CONTEXT_BUFFERS_INFO_PARAMS *)
                (ctrl_grctx + sizeof(rpc_gsp_rm_control));

        memset(ctrl_grctx, 0, sizeof(ctrl_grctx));
#define GI(prop) gi->engineContextBuffersInfo[0].engine[(prop)].size
        GI(NV0080_CTX_PROP_GRAPHICS) = 0x9000u;
        GI(NV0080_CTX_PROP_GRAPHICS_PATCH) = 0x1000u;
        GI(NV0080_CTX_PROP_GRAPHICS_BUNDLE_CB) = 0x30000u;
        GI(NV0080_CTX_PROP_GRAPHICS_PAGEPOOL) = 0x8000u;
        GI(NV0080_CTX_PROP_GRAPHICS_ATTRIBUTE_CB) = 0x1800000u;
        GI(NV0080_CTX_PROP_GRAPHICS_FECS_EVENT) = 0x1000u;
        GI(NV0080_CTX_PROP_GRAPHICS_PRIV_ACCESS_MAP) = 0x10000u;
#undef GI
        fake_rpc_post_payload(lo, (base + 21) % 63,
                              NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                              0, ctrl_grctx, (uint32_t)sizeof(ctrl_grctx));
    }
    fake_rpc_post_payload(lo, (base + 22) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    /* r535_gr_chan_new: query + promote de usuario antes de RM_ALLOC compute. */
    fake_rpc_post_payload(lo, (base + G4E_SLOT_GRCTX_QUERY) % 63,
                          NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_grctx, (uint32_t)sizeof(ctrl_grctx));
    fake_rpc_post_payload(lo, (base + G4E_SLOT_USER_PROMOTE) % 63,
                          NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    fake_rpc_post_payload(lo, (base + G4E_SLOT_COMPUTE_ALLOC) % 63,
                          NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC,
                          0, (const unsigned char *)&alloc_ok, sizeof(alloc_ok));
    msgq->tx.writePtr = (base + G4E_SLOT_COMPUTE_ALLOC + 1u) % 63;

    wptr0 = *q.wptr;
    if (gsp_vmm_init(&q, &rpc, &v, NULL, 1u) != 0) {
        printf("FALLO: gsp_vmm_init (g4e)\n");
        return -1;
    }
    /* El canal necesita VRAM para su bloque de instancia. Región base 0, que es
     * el caso real de esta tarjeta (ver check_vmm). */
    memset(&vram_si, 0, sizeof(vram_si));
    vram_si.ready = 1;
    vram_si.region_nr = 1;
    vram_si.region[0].base = 0;
    /* 256 MiB: el contexto de GR se lleva el attribute CB con alineación de 32 MiB,
     * y con 1 MiB no cabía ni el hueco de alinearlo. */
    vram_si.region[0].size = 0x10000000ull;
    if (gsp_vram_init(&pool, &vram_si) != 0) {
        printf("FALLO: gsp_vram_init (g4e)\n");
        return -1;
    }
    if (gsp_chan_init(&v.rm, &v, &pool, &chan, v.vaspace, 0u,
                      NV2080_ENGINE_TYPE_COPY0) != 0) {
        printf("FALLO: gsp_chan_init (COPY0)\n");
        return -1;
    }

    {
        /* +6 y no +5: el canal manda ahora un RM_CONTROL (tamaño del method
         * buffer) antes de su RM_ALLOC. */
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 6) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_alloc *a = (const rpc_gsp_rm_alloc *)(hdr + 1);
        const NV_CHANNEL_ALLOC_PARAMS *p =
            (const NV_CHANNEL_ALLOC_PARAMS *)(a + 1);

        if (a->hClass != BLACKWELL_CHANNEL_GPFIFO_B ||
            a->hObject != NVKM_RM_CHAN(0) ||
            a->hParent != NVKM_RM_DEVICE) {
            printf("FALLO: canal cls=0x%x obj=0x%08x padre=0x%08x\n",
                   a->hClass, a->hObject, a->hParent);
            return -1;
        }
        if (a->paramsSize != sizeof(*p)) {
            printf("FALLO: canal paramsSize=%u\n", a->paramsSize);
            return -1;
        }
        if (p->gpFifoEntries != GSP_CHAN_GPFIFO_ENTRIES ||
            p->hVASpace != v.vaspace) {
            printf("FALLO: canal entries=%u vaspace=0x%08x\n",
                   p->gpFifoEntries, p->hVASpace);
            return -1;
        }
        /* Contra el 9 literal de `nvrm/engine.h`, NO contra nuestro #define:
         * comparar con la constante propia daba verde con el 6 (= GR5) que RM
         * rechazaba en hardware. La tabla reserva GR0..GR7 en 1..8 y el primer
         * CE cae en el 9. */
        if (p->engineType != 9u) {
            printf("FALLO: engineType=%u, NV2080_ENGINE_TYPE_COPY0 es 9 "
                   "(el 6 es GR5)\n", p->engineType);
            return -1;
        }
        /* Estas afirmaciones van contra los valores de upstream escritos a
         * mano, NO contra nuestras propias constantes: el RM_ALLOC del canal
         * devolvió 0x3b (INVALID_PARAMETER) en HW el 2026-07-27 con el banco en
         * verde, porque el banco releía los campos con la misma definición del
         * struct que los escribía. Un layout mal pero coherente consigo mismo
         * es invisible desde dentro; los números crudos sí lo delatan. */
        /* El 656 que había aquí era ese mismo error una capa más arriba: se
         * escribió "a mano" pero sumando con NV_MAX_SUBDEVICES=32, y el 32 sale
         * de `NV_MAX_DEVICES` de `nvlimits.h`, no de `NV_MAX_SUBDEVICES`, que es
         * 8. Con 8 la struct mide 368 y `instanceMem` cae en el 144 (2026-07-28).
         *
         * Va también el offset de `instanceMem`, que es el que de verdad delata
         * el desplazamiento: `hUserdMemory` está antes de los dos arrays y su 32
         * no se mueve ni con el valor bueno ni con el malo. */
        if (sizeof(*p) != 368u || a->paramsSize != 368u) {
            printf("FALLO: NV_CHANNEL_ALLOC_PARAMS mide %zu y paramsSize dice %u "
                   "(upstream r570 con NV_MAX_SUBDEVICES=8: 368)\n",
                   sizeof(*p), a->paramsSize);
            return -1;
        }
        if (offsetof(NV_CHANNEL_ALLOC_PARAMS, instanceMem) != 144u) {
            printf("FALLO: instanceMem en el offset %zu (upstream r570: 144)\n",
                   offsetof(NV_CHANNEL_ALLOC_PARAMS, instanceMem));
            return -1;
        }
        if (p->userdMem.addressSpace != 2u || p->mthdbufMem.addressSpace != 1u) {
            printf("FALLO: aperturas userd=%u mthdbuf=%u (upstream: userd=2 VRAM, "
                   "mthdbuf=1 sysmem)\n",
                   p->userdMem.addressSpace, p->mthdbufMem.addressSpace);
            return -1;
        }
        if (!chan.userd_vram || chan.userd.phys == 0) {
            printf("FALLO: USERD no está en VRAM (userd_vram=%d phys=0x%llx)\n",
                   chan.userd_vram, (unsigned long long)chan.userd.phys);
            return -1;
        }
        /* Bloque de instancia y RAMFC: en VRAM (2), mismo base, y el RAMFC son
         * los primeros 0x200 B. Estaban a cero y son obligatorios. */
        if (p->instanceMem.addressSpace != 2u || p->ramfcMem.addressSpace != 2u ||
            p->instanceMem.base == 0 || p->ramfcMem.base != p->instanceMem.base ||
            p->instanceMem.size != 0x1000u || p->ramfcMem.size != 0x200u) {
            printf("FALLO: instanceMem/ramfcMem base=0x%llx/0x%llx size=%llu/%llu aper=%u/%u\n",
                   (unsigned long long)p->instanceMem.base,
                   (unsigned long long)p->ramfcMem.base,
                   (unsigned long long)p->instanceMem.size,
                   (unsigned long long)p->ramfcMem.size,
                   p->instanceMem.addressSpace, p->ramfcMem.addressSpace);
            return -1;
        }
        /* errorNotifierMem a cero y los dos tipos de notificador a NONE (1),
         * que en los bits 3:2 y 5:4 son 0x4|0x10. Cero significaba UNKNOWN. */
        if (p->errorNotifierMem.base != 0 || p->errorNotifierMem.addressSpace != 0) {
            printf("FALLO: errorNotifierMem relleno (upstream no lo toca)\n");
            return -1;
        }
        if ((p->internalFlags & 0x3cu) != 0x14u) {
            printf("FALLO: internalFlags=0x%08x, notificadores no están a NONE\n",
                   p->internalFlags);
            return -1;
        }
        if (p->subDeviceId != 0 || (p->flags & NVOS04_FLAGS_CHANNEL_CLIENT_MAP_FIFO)) {
            printf("FALLO: subDeviceId=%u flags=0x%08x (upstream: 0 y CLIENT_MAP_FIFO a FALSE)\n",
                   p->subDeviceId, p->flags);
            return -1;
        }
        /* Con rsvd_chids=1 el primer canal pide chid=1 → USERD_INDEX=1. */
        if (p->flags != (0x00200020u | NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE(1))) {
            printf("FALLO: flags=0x%08x, esperaba 0x%08x (PRIVILEGED | PAGE_FIXED |"
                   " USERD idx=1, rsvd_chids=1)\n", p->flags,
                   0x00200020u | NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE(1));
            return -1;
        }
        /* Y el privilegio tiene que decir lo mismo en los dos sitios. */
        if ((p->internalFlags & 0x3u) != 1u) {
            printf("FALLO: internalFlags privilegio=%u con PRIVILEGED_CHANNEL puesto\n",
                   p->internalFlags & 0x3u);
            return -1;
        }
        /* La VA del ring, no su dirección física: si esto vuelve a ser `phys`,
         * RM programa el PBDMA para buscar las entradas donde no están. */
        if (p->gpFifoOffset != chan.gpfifo_va || p->gpFifoOffset == chan.gpfifo.phys) {
            printf("FALLO: gpFifoOffset=0x%llx (VA esperada 0x%llx, phys 0x%llx)\n",
                   (unsigned long long)p->gpFifoOffset,
                   (unsigned long long)chan.gpfifo_va,
                   (unsigned long long)chan.gpfifo.phys);
            return -1;
        }
        /* 0x200 = `gv100_chan_userd.size`. La página que lo aloja son 4096, pero
         * eso no es lo que se le declara a RM. */
        if (p->userdMem.size != 0x200u) {
            printf("FALLO: userdMem.size=%llu, el USERD del chip son 0x200\n",
                   (unsigned long long)p->userdMem.size);
            return -1;
        }
        /* Method buffer: búfer propio (no el pushbuffer) y del tamaño que dijo
         * RM por CE_GET_FAULT_METHOD_BUFFER_SIZE. */
        if (p->mthdbufMem.base != chan.mthdbuf.phys ||
            p->mthdbufMem.base == chan.pushbuf.phys ||
            p->mthdbufMem.size != FAKE_MTHDBUF_SIZE) {
            printf("FALLO: mthdbufMem base=0x%llx size=%llu (esperaba 0x%llx/%u, y "
                   "NO el pushbuffer 0x%llx)\n",
                   (unsigned long long)p->mthdbufMem.base,
                   (unsigned long long)p->mthdbufMem.size,
                   (unsigned long long)chan.mthdbuf.phys, FAKE_MTHDBUF_SIZE,
                   (unsigned long long)chan.pushbuf.phys);
            return -1;
        }
    }
    printf("OK: canal contra r535_chan_alloc — flags 0x%08x, GPFIFO por VA, "
           "USERD 0x200, method buffer propio de %u B\n",
           0x00200020u | NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE(1),
           chan.mthdbuf_size);
    printf("OK: BLACKWELL_CHANNEL_GPFIFO_B alloc params (layout r570, inst+ramfc en VRAM)\n");

    /* Los tres pasos de arranque, en orden y sobre el objeto del CANAL. Faltaban
     * enteros: el canal se reservaba y se quedaba fuera de la runlist, y el CE se
     * comía un timeout de 2 s con el semáforo a 0 (HW, 2026-07-28). Que el
     * `hObject` sea el canal y no el subdevice es la mitad del contrato. */
    {
        static const struct { unsigned slot; uint32_t cmd; const char *name; } start[] = {
            { 7, NVA06F_CTRL_CMD_BIND, "BIND" },
            { 8, NVA06F_CTRL_CMD_GPFIFO_SCHEDULE, "GPFIFO_SCHEDULE" },
            { 9, NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN, "GET_WORK_SUBMIT_TOKEN" },
        };

        for (i = 0; i < sizeof(start) / sizeof(start[0]); i++) {
            const unsigned char *entry = cmdq_base + 4096 +
                (unsigned long)((wptr0 + start[i].slot) % 63) * 4096;
            const struct gsp_rpc_hdr *hdr =
                (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
            const rpc_gsp_rm_control *ct = (const rpc_gsp_rm_control *)(hdr + 1);

            if (ct->cmd != start[i].cmd || ct->hObject != NVKM_RM_CHAN(0)) {
                printf("FALLO: arranque paso %u (%s) cmd=0x%08x obj=0x%08x "
                       "(esperaba 0x%08x sobre el canal 0x%08x)\n",
                       i + 1, start[i].name, ct->cmd, ct->hObject,
                       start[i].cmd, NVKM_RM_CHAN(0));
                return -1;
            }
        }
    }
    {
        /* El BIND ata el canal a COPY0, y otra vez contra el 9 literal: es lo que
         * dice `nvrm/engine.h` y lo que el propio alloc declaró arriba. */
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 7) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_control *ct = (const rpc_gsp_rm_control *)(hdr + 1);
        const NVA06F_CTRL_BIND_PARAMS *bp =
            (const NVA06F_CTRL_BIND_PARAMS *)(ct + 1);

        if (ct->paramsSize != 4u || bp->engineType != 9u) {
            printf("FALLO: BIND paramsSize=%u engineType=%u (esperaba 4 y 9)\n",
                   ct->paramsSize, bp->engineType);
            return -1;
        }
    }
    {
        /* DOS NvBool son DOS bytes. Fueron tres —el `bSkipEnable` de la rama
         * main— y RM contestó INVALID_ARGUMENT en hardware: en 570.144 ese campo
         * no existe. El número crudo es el ancla, y va contra el tag. */
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 8) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_control *ct = (const rpc_gsp_rm_control *)(hdr + 1);
        const NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS *sp =
            (const NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS *)(ct + 1);

        if (ct->paramsSize != 2u || sp->bEnable != 1 || sp->bSkipSubmit != 0) {
            printf("FALLO: SCHEDULE paramsSize=%u bEnable=%u bSkipSubmit=%u "
                   "(esperaba 2 y 1/0)\n", ct->paramsSize, sp->bEnable,
                   sp->bSkipSubmit);
            return -1;
        }
    }
    /* Y que el token que contestó RM haya llegado entero al canal. */
    if (!chan.doorbell_ok ||
        chan.doorbell_token != (FAKE_DOORBELL_TOKEN | 1u) ||
        chan.doorbell_kick != FAKE_DOORBELL_KICK_CHID(1)) {
        printf("FALLO: token del doorbell ok=%d RPC=0x%08x kick=0x%08x "
               "(esperaba RPC=0x%08x kick=0x%08x)\n",
               chan.doorbell_ok, chan.doorbell_token, chan.doorbell_kick,
               FAKE_DOORBELL_TOKEN | 1u, FAKE_DOORBELL_KICK_CHID(1));
        return -1;
    }
    printf("OK: canal arrancado — BIND(9) + SCHEDULE(bEnable=1, 3 B) + token "
           "RPC=0x%08x kick=0x%08x, los tres sobre el canal\n",
           chan.doorbell_token, chan.doorbell_kick);

    if (gsp_pramin_rd32(chan.userd.phys + FAKE_USERD_OFF_GPPUT) != 0 ||
        chan.gpput != 0) {
        printf("FALLO: USERD/GPPut no arrancan en cero\n");
        return -1;
    }
    if (fake_doorbell_writes != 0) {
        printf("FALLO: %u doorbell(s) antes de encolar nada\n",
               fake_doorbell_writes);
        return -1;
    }
    /* El arranque del canal tiene que haber comprobado que el aperture de
     * usermode contesta: es lo único que se puede LEER del sitio donde después se
     * escribe el doorbell a ciegas. */
    if (fake_usermode_reads < 2u) {
        printf("FALLO: el canal arrancó sin sondear el reloj de usermode "
               "(%u lecturas de 0x%06x)\n", fake_usermode_reads,
               FAKE_USERMODE_TIME);
        return -1;
    }
    if (g_usermode_verdict != 1) {
        printf("FALLO: el aperture de usermode contesta y el veredicto es %d\n",
               g_usermode_verdict);
        return -1;
    }
    printf("OK: aperture de usermode sondeado antes del primer submit "
           "(0x%06x, %u lecturas)\n", FAKE_USERMODE_TIME, fake_usermode_reads);

    /* Y el camino malo, que es el que va a importar en hardware: con el aperture
     * mudo la sonda tiene que DECIRLO. Un veredicto que sólo sabe dar buenas
     * noticias no distingue un doorbell escrito en el vacío de un canal que no
     * arranca, que es exactamente el par que vino a separar. */
    fake_usermode_dead = 1;
    g_usermode_verdict = 0;
    chan_probe_usermode();
    fake_usermode_dead = 0;
    if (g_usermode_verdict != -1) {
        printf("FALLO: con el usermode a 0xbadf1000 el veredicto es %d\n",
               g_usermode_verdict);
        return -1;
    }
    g_usermode_verdict = 1;
    printf("OK: un aperture de usermode mudo se detecta (no se da por bueno)\n");

    if (gsp_ce_init(&v.rm, &chan, &ce) != 0) {
        printf("FALLO: gsp_ce_init\n");
        return -1;
    }

    {
        /* +10, no +7: entre el alloc del canal y este van los tres controles de
         * arranque. */
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 10) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_alloc *a = (const rpc_gsp_rm_alloc *)(hdr + 1);

        const NVC0B5_ALLOCATION_PARAMETERS *ce_args =
            (const NVC0B5_ALLOCATION_PARAMETERS *)(a + 1);

        if (a->hClass != BLACKWELL_DMA_COPY_B || a->hObject != NVKM_RM_CE0 ||
            a->hParent != NVKM_RM_CHAN(0)) {
            printf("FALLO: CE cls=0x%x obj=0x%08x padre=0x%08x\n",
                   a->hClass, a->hObject, a->hParent);
            return -1;
        }
        if (a->paramsSize != sizeof(*ce_args)) {
            printf("FALLO: CE paramsSize=%u (esperaba %zu)\n",
                   a->paramsSize, sizeof(*ce_args));
            return -1;
        }
        if (ce_args->version != 1u || ce_args->engineType != chan.engine) {
            printf("FALLO: CE alloc version=%u engineType=%u (esperaba 1 y %u)\n",
                   ce_args->version, ce_args->engineType, chan.engine);
            return -1;
        }
    }
    printf("OK: BLACKWELL_DMA_COPY_B colgado del canal (NVC0B5 engineType=%u)\n",
           chan.engine);

    if (gsp_ce_encode_copy(&ce, GSP_CHAN_VA_BASE + 8192ull,
                           GSP_CHAN_VA_BASE + 12288ull, 4096,
                           &pb_off, &pb_len) != 0 || pb_len < 32) {
        printf("FALLO: gsp_ce_encode_copy\n");
        return -1;
    }
    {
        const uint32_t *pb = (const uint32_t *)((const unsigned char *)chan.pushbuf.va + pb_off);
        unsigned found_launch = 0;

        /* Lo PRIMERO del pushbuffer: SET_OBJECT con la **clase** del CE por el
         * subcanal 4 (COPY_ENGINE), en paridad con nouveau/UVM. El handle de RM
         * entero por subcanal 0 dejaba clase 0x0000 atada a GR.
         *
         * INCR_OPCODE (31:29) | count=1 (28:16) | subc (15:13) | mthd>>2 (11:0). */
        if (pb_len < 8u ||
            pb[0] != ((NVC56F_DMA_INCR_OPCODE_VALUE << 29) | (1u << 16) |
                      (4u << 13) | ((NVC56F_SET_OBJECT >> 2) & 0xfffu)) ||
            pb[1] != ce.cls) {
            printf("FALLO: el pushbuffer no empieza por SET_OBJECT(cls=0x%04x "
                   "subc=4): %08x %08x\n", ce.cls, pb[0],
                   pb_len >= 8u ? pb[1] : 0u);
            return -1;
        }
        printf("OK: SET_OBJECT lleva la clase 0x%04x por subcanal 4\n", ce.cls);

        for (i = 0; i < pb_len / 4; i++) {
            if (pb[i] == (NVC6B5_LAUNCH_DMA_DATA_TRANSFER_TYPE_NON_PIPELINED |
                          NVC6B5_LAUNCH_DMA_FLUSH_ENABLE_TRUE |
                          NVC6B5_LAUNCH_DMA_SRC_TYPE_VIRTUAL |
                          NVC6B5_LAUNCH_DMA_DST_TYPE_VIRTUAL |
                          NVC6B5_LAUNCH_DMA_SRC_MEMORY_LAYOUT_PITCH |
                          NVC6B5_LAUNCH_DMA_DST_MEMORY_LAYOUT_PITCH |
                          NVC6B5_LAUNCH_DMA_SEMAPHORE_TYPE_RELEASE_ONE_WORD)) {
                found_launch = 1;
                break;
            }
        }
        if (!found_launch) {
            printf("FALLO: pushbuffer sin LAUNCH_DMA esperado\n");
            return -1;
        }
    }
    printf("OK: pushbuffer CE (%u B) con LAUNCH_DMA\n", pb_len);

    if (gsp_chan_submit(&chan, pb_off, pb_len) != 0) {
        printf("FALLO: gsp_chan_submit\n");
        return -1;
    }
    if (chan.gpput != 1 ||
        gsp_pramin_rd32(chan.userd.phys + FAKE_USERD_OFF_GPPUT) != 1) {
        printf("FALLO: GPPut=%u gpput=%u\n",
               gsp_pramin_rd32(chan.userd.phys + FAKE_USERD_OFF_GPPUT),
               chan.gpput);
        return -1;
    }
    /* Y el kick, que es la razón de todo este cambio: publicar GPPut en el USERD
     * no despierta a nadie en Volta+. Un submit sin doorbell es exactamente el
     * fallo que había —trabajo encolado, semáforo a 0, dos segundos de espera— y
     * desde dentro se ve idéntico a un submit correcto, así que hay que
     * comprobarlo aquí: una escritura, en el registro de usermode, con el token
     * ENTERO que devolvió RM. */
    if (fake_doorbell_writes != 1 ||
        fake_doorbell_last != FAKE_DOORBELL_KICK_CHID(1)) {
        printf("FALLO: doorbell escrituras=%u último=0x%08x (esperaba 1 y 0x%08x "
               "en 0x%06x)\n", fake_doorbell_writes, fake_doorbell_last,
               FAKE_DOORBELL_KICK_CHID(1), NV_VFN_DOORBELL);
        return -1;
    }
    {
        const uint32_t *ring = (const uint32_t *)chan.gpfifo.va;
        uint64_t want = chan.pushbuf_va + pb_off;
        uint64_t got;

        if ((ring[0] & 1u) != NVC56F_GP_ENTRY0_FETCH_UNCONDITIONAL) {
            printf("FALLO: GPFIFO entry0 fetch\n");
            return -1;
        }
        if (((ring[1] >> 10) & 0x1fffffu) != ((pb_len + 3u) / 4u)) {
            printf("FALLO: GPFIFO length en words\n");
            return -1;
        }
        /* LA DIRECCIÓN. Este banco comprobaba el bit de fetch y la longitud y
         * nada más, así que dio verde a un encoder que mandaba `addr >> 2` y a
         * una VA de 41 bits truncada a 40 — el host iba a buscar el pushbuffer a
         * otro sitio y el CE se quedaba sin señalizar (HW, 2026-07-28). Se
         * reconstruye la dirección DESDE la entrada, como haría el host, en vez de
         * releer los campos con la misma fórmula que los escribió. */
        got = ((uint64_t)(ring[1] & 0xffu) << 32) | (uint64_t)(ring[0] & 0xfffffffcu);
        if (got != want) {
            printf("FALLO: GPFIFO apunta a 0x%llx, el pushbuffer está en 0x%llx\n",
                   (unsigned long long)got, (unsigned long long)want);
            return -1;
        }
        /* Y que la VA quepa de verdad en el campo: si el mapa vuelve a subir por
         * encima de 2^40, la comparación de arriba seguiría cuadrando —los dos
         * lados truncarían igual— y esto es lo que lo caza. */
        if (want > GSP_GPFIFO_VA_MAX) {
            printf("FALLO: pushbuffer en 0x%llx, por encima del techo 0x%llx del "
                   "GPFIFO\n", (unsigned long long)want,
                   (unsigned long long)GSP_GPFIFO_VA_MAX);
            return -1;
        }
    }
    printf("OK: GPFIFO entry + USERD GPPut + doorbell 0x%08x en 0x%06x\n",
           fake_doorbell_last, NV_VFN_DOORBELL);

    /* Copia de más de una página: la que sube los pesos. Tiene que salir como
     * la de upstream (`nve0_bo_move_copy`) —N líneas de página con el pitch a
     * página y MULTI_LINE— y no como una línea gigante, que es un encoding que
     * no ha visto silicio. Sin esto, el cambio de "un LAUNCH_DMA por búfer" se
     * comprobaría por primera vez en la tarjeta, a un ciclo de VFIO por intento. */
    {
        unsigned mpb_off = 0, mpb_len = 0;
        const uint32_t *pb;
        uint32_t pitch_in = 0, pitch_out = 0, line_len = 0, lines = 0, launch = 0;
        const uint32_t size = 3u * 4096u;

        if (gsp_ce_encode_copy(&ce, GSP_CHAN_VA_BASE + 8192ull,
                               GSP_CHAN_VA_BASE + 16384ull, size,
                               &mpb_off, &mpb_len) != 0) {
            printf("FALLO: gsp_ce_encode_copy multilínea\n");
            return -1;
        }
        pb = (const uint32_t *)((const unsigned char *)chan.pushbuf.va + mpb_off);
        if (pb_method_value(pb, mpb_len / 4u, NVC6B5_PITCH_IN, &pitch_in) != 0 ||
            pb_method_value(pb, mpb_len / 4u, NVC6B5_PITCH_OUT, &pitch_out) != 0 ||
            pb_method_value(pb, mpb_len / 4u, NVC6B5_LINE_LENGTH_IN, &line_len) != 0 ||
            pb_method_value(pb, mpb_len / 4u, NVC6B5_LINE_COUNT, &lines) != 0 ||
            pb_method_value(pb, mpb_len / 4u, NVC6B5_LAUNCH_DMA, &launch) != 0) {
            printf("FALLO: al pushbuffer multilínea le falta algún método\n");
            return -1;
        }
        if (pitch_in != GSP_CE_LINE_BYTES || pitch_out != GSP_CE_LINE_BYTES ||
            line_len != GSP_CE_LINE_BYTES || lines != size / GSP_CE_LINE_BYTES ||
            !(launch & NVC6B5_LAUNCH_DMA_MULTI_LINE_ENABLE_TRUE)) {
            printf("FALLO: multilínea pitch=%u/%u len=%u count=%u launch=0x%08x\n",
                   pitch_in, pitch_out, line_len, lines, launch);
            return -1;
        }
        /* Rabo < página: boa0b5 usa LINE_LENGTH=PAGE_SIZE, LINE_COUNT=1
         * (stage_sass rellena el rebote). Saxpy 512 B fue el fallo run14. */
        if (gsp_ce_encode_copy(&ce, GSP_CHAN_VA_BASE + 8192ull,
                               GSP_CHAN_VA_BASE + 16384ull, 512u,
                               &mpb_off, &mpb_len) != 0) {
            printf("FALLO: gsp_ce_encode_copy saxpy 512 B\n");
            return -1;
        }
        pb = (const uint32_t *)((const unsigned char *)chan.pushbuf.va + mpb_off);
        if (pb_method_value(pb, mpb_len / 4u, NVC6B5_LINE_COUNT, &lines) != 0 ||
            pb_method_value(pb, mpb_len / 4u, NVC6B5_LINE_LENGTH_IN, &line_len) != 0 ||
            pb_method_value(pb, mpb_len / 4u, NVC6B5_LAUNCH_DMA, &launch) != 0 ||
            lines != 1u || line_len != GSP_CE_LINE_BYTES ||
            (launch & NVC6B5_LAUNCH_DMA_MULTI_LINE_ENABLE_TRUE)) {
            printf("FALLO: saxpy 512 B no salió PAGE×1 (count=%u len=%u "
                   "launch=0x%08x)\n", lines, line_len, launch);
            return -1;
        }
        printf("OK: copia de %u B = %u líneas de página con MULTI_LINE; "
               "512 B = PAGE×1 (boa0b5)\n", size, size / GSP_CE_LINE_BYTES);
    }

    /* --- G6: subida a un offset del búfer y subida por DMA sin copia de CPU ---
     *
     * Las dos nacieron del mismo fallo (2026-08-17): el kernel copiaba el tensor
     * ENTERO a un temporal de su heap —44 MiB en TinyLlama— porque esta capa sólo
     * aceptaba la VA base del slot. Con `offset` puede subir a trozos, y con la
     * variante DMA no copia nada: mapea las páginas del proceso en `G6_SRC_VA` y
     * el CE lee de ahí. Nada de esto se puede probar en la placa Ampere (no llega
     * a haber pool), así que el encoding y los rechazos se juzgan aquí. */
    {
        struct gsp_buf buf;
        struct gsp_dma_buf rebote;
        struct gsp_dma_buf origen;
        static unsigned char datos[3u * 4096u];
        uint64_t va, phys[3], phys_leida = 0, pte = 0;
        unsigned antes, i;
        const uint32_t *pb;
        uint32_t hi = 0, lo = 0, lines = 0, launch = 0;
        uint64_t dst, src;

        /* El semáforo del CE no lo firma nadie en el host: se deja por encima de
         * cualquier payload futuro para que la espera pase y lo que se juzgue sea
         * el pushbuffer, que es lo que este banco sí puede leer. */
        *(volatile uint32_t *)chan.notifier.va = 0x40000000u;

        if (gsp_dma_alloc(&rebote, 8u * 4096u, "rebote G6") != 0 ||
            gsp_vmm_map(&v, G6_BOUNCE_VA, rebote.phys, 8u * 4096u,
                        GSP_VMM_SYSMEM) != 0) {
            printf("FALLO: rebote G6\n");
            return -1;
        }
        if (gsp_buf_init(&buf, &pool, &v, &ce, G6_BOUNCE_VA, rebote.va,
                         8u * 4096u) != 0) {
            printf("FALLO: gsp_buf_init\n");
            return -1;
        }
        printf("OK: G6 pool con CE y sin canal GR\n");
        va = gsp_buf_alloc(&buf, 3u * 4096u);
        if (!va) {
            printf("FALLO: gsp_buf_alloc\n");
            return -1;
        }

        /* Con offset, el destino del CE es va+offset y no la base del slot. */
        memset(datos, 0xab, sizeof(datos));
        antes = chan.pb_pos;
        if (gsp_buf_upload_at(&buf, va, 4096u, datos, 4096u) != 0) {
            printf("FALLO: gsp_buf_upload_at\n");
            return -1;
        }
        pb = (const uint32_t *)((const unsigned char *)chan.pushbuf.va + antes);
        if (pb_method_value(pb, (chan.pb_pos - antes) / 4u, NVC6B5_OFFSET_OUT_UPPER, &hi) != 0 ||
            pb_method_value(pb, (chan.pb_pos - antes) / 4u, NVC6B5_OFFSET_OUT_LOWER, &lo) != 0) {
            printf("FALLO: al pushbuffer de la subida le falta OFFSET_OUT\n");
            return -1;
        }
        dst = ((uint64_t)hi << 32) | lo;
        if (dst != va + 4096ull) {
            printf("FALLO: subida con offset escribe en 0x%llx (esperaba 0x%llx)\n",
                   (unsigned long long)dst, (unsigned long long)(va + 4096ull));
            return -1;
        }
        printf("OK: gsp_buf_upload_at copia a va+offset (0x%llx)\n",
               (unsigned long long)dst);

        /* Y lo que NO puede hacer: salirse del slot (pisaría el tensor de al lado
         * y el síntoma saldría capas después, en otro peso) ni empezar a mitad de
         * página (la copia dejaría de ser la multilínea probada). */
        if (gsp_buf_upload_at(&buf, va, 4096u, datos, 3u * 4096u) == 0 ||
            gsp_buf_upload_at(&buf, va, 100u, datos, 4096u) == 0) {
            printf("FALLO: la subida con offset acepta pasarse del búfer o sin alinear\n");
            return -1;
        }
        printf("OK: subida con offset rechaza pasarse del búfer y el offset sin alinear\n");

        /* DMA: el origen son páginas ajenas, se mapean en G6_SRC_VA y el CE lee
         * de ahí. Un solo LAUNCH_DMA para todo el lote. */
        if (gsp_dma_alloc(&origen, 3u * 4096u, "origen G6") != 0) {
            printf("FALLO: origen G6\n");
            return -1;
        }
        for (i = 0; i < 3u; i++) {
            phys[i] = origen.phys + (uint64_t)i * 4096ull;
        }
        antes = chan.pb_pos;
        if (gsp_buf_upload_dma(&buf, va, 0, phys, 3u, 0u, 3u * 4096u) != 0) {
            printf("FALLO: gsp_buf_upload_dma\n");
            return -1;
        }
        for (i = 0; i < 3u; i++) {
            if (gsp_vmm_translate(&v, G6_SRC_VA + (uint64_t)i * 4096ull,
                                  &phys_leida, &pte) != 0 ||
                phys_leida != phys[i]) {
                printf("FALLO: la ventana del origen traduce la página %u a 0x%llx "
                       "(esperaba 0x%llx)\n", i, (unsigned long long)phys_leida,
                       (unsigned long long)phys[i]);
                return -1;
            }
        }
        pb = (const uint32_t *)((const unsigned char *)chan.pushbuf.va + antes);
        if (pb_method_value(pb, (chan.pb_pos - antes) / 4u, NVC6B5_OFFSET_IN_UPPER, &hi) != 0 ||
            pb_method_value(pb, (chan.pb_pos - antes) / 4u, NVC6B5_OFFSET_IN_LOWER, &lo) != 0) {
            printf("FALLO: al pushbuffer del DMA le falta OFFSET_IN\n");
            return -1;
        }
        src = ((uint64_t)hi << 32) | lo;
        if (pb_method_value(pb, (chan.pb_pos - antes) / 4u, NVC6B5_LINE_COUNT, &lines) != 0 ||
            pb_method_value(pb, (chan.pb_pos - antes) / 4u, NVC6B5_LAUNCH_DMA, &launch) != 0) {
            printf("FALLO: al pushbuffer del DMA le faltan métodos\n");
            return -1;
        }
        if (src != G6_SRC_VA || lines != 3u ||
            !(launch & NVC6B5_LAUNCH_DMA_MULTI_LINE_ENABLE_TRUE)) {
            printf("FALLO: DMA src=0x%llx lines=%u launch=0x%08x\n",
                   (unsigned long long)src, lines, launch);
            return -1;
        }
        printf("OK: subida por DMA — origen mapeado en G6_SRC_VA y UN LAUNCH_DMA "
               "de %u líneas, sin copia de CPU\n", lines);

        /* Y sus rechazos, todos antes de tocar el CE: si la lista de páginas no
         * cuadra con el tamaño, el mapeo y la copia dirían cosas distintas. */
        if (gsp_buf_upload_dma(&buf, va, 0, phys, 2u, 0u, 3u * 4096u) == 0 ||
            gsp_buf_upload_dma(&buf, va, 0, phys, 3u, 0u, G6_SRC_MAX + 4096ull) == 0 ||
            gsp_buf_upload_dma(&buf, va, 100u, phys, 3u, 0u, 3u * 4096u) == 0) {
            printf("FALLO: el DMA acepta una lista que no cuadra, pasarse de "
                   "ventana o un offset sin alinear\n");
            return -1;
        }
        {
            uint64_t torcida[3];

            for (i = 0; i < 3u; i++) {
                torcida[i] = phys[i] + 8ull;
            }
            if (gsp_buf_upload_dma(&buf, va, 0, torcida, 3u, 0u, 3u * 4096u) == 0) {
                printf("FALLO: el DMA acepta físicas sin alinear a página\n");
                return -1;
            }
        }
        printf("OK: el DMA rechaza lista descuadrada, ventana pasada, offset y "
               "físicas sin alinear\n");

        /* EL CASO DE VERDAD: el payload de un shard `.som` empieza en el byte 64
         * del fichero, así que el puntero de un tensor mapeado llega aquí en +64.
         * Mientras esto exigió alineación de página, el camino sin copias no se
         * ejecutó ni una vez con pesos de un modelo y nadie lo notó (el rebote da
         * el mismo resultado, sólo cuesta una copia entera por la CPU). Aquí se
         * fija que el CE lee de G6_SRC_VA+64 y que las páginas cubren 64+size. */
        antes = chan.pb_pos;
        if (gsp_buf_upload_dma(&buf, va, 0, phys, 3u, 64u, 2u * 4096u) != 0) {
            printf("FALLO: el DMA rechaza un origen en +64 (el caso del shard)\n");
            return -1;
        }
        pb = (const uint32_t *)((const unsigned char *)chan.pushbuf.va + antes);
        if (pb_method_value(pb, (chan.pb_pos - antes) / 4u, NVC6B5_OFFSET_IN_UPPER, &hi) != 0 ||
            pb_method_value(pb, (chan.pb_pos - antes) / 4u, NVC6B5_OFFSET_IN_LOWER, &lo) != 0 ||
            pb_method_value(pb, (chan.pb_pos - antes) / 4u, NVC6B5_LINE_COUNT, &lines) != 0) {
            printf("FALLO: al pushbuffer del DMA desalineado le faltan métodos\n");
            return -1;
        }
        src = ((uint64_t)hi << 32) | lo;
        if (src != G6_SRC_VA + 64ull || lines != 2u) {
            printf("FALLO: DMA desalineado src=0x%llx (esperaba 0x%llx) lines=%u\n",
                   (unsigned long long)src, (unsigned long long)(G6_SRC_VA + 64ull),
                   lines);
            return -1;
        }
        printf("OK: subida por DMA desde un origen en +64 — el CE lee de "
               "G6_SRC_VA+64 (el caso del payload de un shard)\n");

        if (gsp_buf_upload_dma(&buf, va, 0, phys, 3u, 4096u, 2u * 4096u) == 0 ||
            gsp_buf_upload_dma(&buf, va, 0, phys, 3u, 64u, 3u * 4096u) == 0 ||
            gsp_buf_upload_dma(&buf, va, 0, phys, 3u, 64u, G6_SRC_MAX) == 0) {
            printf("FALLO: el DMA acepta src_off de una página entera, una lista "
                   "que no cubre src_off+size, o pasarse de ventana con src_off\n");
            return -1;
        }
        printf("OK: el DMA rechaza src_off >= 4096, lista corta para src_off+size "
               "y ventana pasada por el src_off\n");
        gsp_buf_fini(&buf);
    }

    /* --- G4f/G5: canal de GR0 + compute + QMD inline --- */
    if (gsp_chan_init(&v.rm, &v, &pool, &chan_gr, v.vaspace, 1u,
                      NV2080_ENGINE_TYPE_GR0) != 0) {
        printf("FALLO: gsp_chan_init (GR0)\n");
        return -1;
    }
    {
        /* El canal de GR0 va en el índice 11 de las peticiones (el del CE en el 6
         * y su alloc de CE en el 10; sin segundo GET_FAULT_METHOD_BUFFER_SIZE). Lo
         * que hay que demostrar aquí es que el SEGUNDO canal pide de verdad otro
         * motor y no una copia del primero: RM contesta INVALID_CLASS al objeto
         * de compute sobre un canal de COPY0. */
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 11) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_gsp_rm_alloc *a = (const rpc_gsp_rm_alloc *)(hdr + 1);
        const NV_CHANNEL_ALLOC_PARAMS *p = (const NV_CHANNEL_ALLOC_PARAMS *)(a + 1);

        if (a->hObject != NVKM_RM_CHAN(1) || a->hParent != NVKM_RM_DEVICE) {
            printf("FALLO: canal GR0 obj=0x%08x padre=0x%08x\n",
                   a->hObject, a->hParent);
            return -1;
        }
        /* Contra el 1 literal de `nvrm/engine.h`, como el 9 de COPY0. */
        if (p->engineType != 1u) {
            printf("FALLO: engineType=%u, NV2080_ENGINE_TYPE_GR0 es 1\n",
                   p->engineType);
            return -1;
        }
        /* Y el slot de USERD, que es por donde se pide el chid: el segundo canal
         * tiene que pedir OTRO. Con los índices clavados a 0 —como estaban— los dos
         * canales pedían el mismo teniéndolo declarado fijo, y el segundo se llevó
         * un NO_MEMORY en hardware. El valor esperado es el del chid 1: índice 1
         * (bit 8), página 0, PAGE_FIXED y PRIVILEGED. */
        if (p->flags != (0x00200020u | NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE(2))) {
            printf("FALLO: flags del canal GR0 = 0x%08x, esperaba 0x%08x "
                   "(índice de USERD 2, chid=2 tras rsvd_chids=1)\n",
                   p->flags,
                   0x00200020u | NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE(2));
            return -1;
        }
        if (chan_gr.doorbell_token != (FAKE_DOORBELL_TOKEN | 2u) ||
            chan_gr.doorbell_token == chan.doorbell_token) {
            printf("FALLO: token del canal GR0 = 0x%08x (el del CE es 0x%08x)\n",
                   chan_gr.doorbell_token, chan.doorbell_token);
            return -1;
        }
        printf("OK: el segundo canal pide otro slot de USERD (flags 0x%08x → "
               "índice %u, página %u) y recibe otro token (0x%08x ≠ 0x%08x)\n",
               p->flags, (p->flags >> 8) & 0x7u, (p->flags >> 12) & 0x1ffu,
               chan_gr.doorbell_token, chan.doorbell_token);
        /* Dos canales, dos ventanas de VAs. Compartir el pushbuffer o el GPFIFO
         * sería un canal escribiendo métodos dentro del ring del otro. */
        if (chan_gr.pushbuf_va == chan.pushbuf_va ||
            chan_gr.gpfifo_va == chan.gpfifo_va ||
            chan_gr.userd_va == chan.userd_va ||
            chan_gr.inst_addr == chan.inst_addr) {
            printf("FALLO: los dos canales comparten búferes (pb 0x%llx/0x%llx "
                   "inst 0x%llx/0x%llx)\n",
                   (unsigned long long)chan.pushbuf_va,
                   (unsigned long long)chan_gr.pushbuf_va,
                   (unsigned long long)chan.inst_addr,
                   (unsigned long long)chan_gr.inst_addr);
            return -1;
        }
        {
            /* idx=0 mapea gpfifo+PB+notifier (104 KiB). Con stride 64 KiB el GR0
             * pisaba el notifier del CE y el semáforo se quedaba en 10/11. */
            uint64_t end0 = chan.gpfifo_va + (uint64_t)GSP_CHAN_VA_USED;

            if (chan_gr.gpfifo_va < end0) {
                printf("FALLO: solape VA CE idx=0 [0x%llx..0x%llx) con GR0 "
                       "idx=1 gpfifo=0x%llx (stride=0x%llx used=0x%x)\n",
                       (unsigned long long)chan.gpfifo_va,
                       (unsigned long long)end0,
                       (unsigned long long)chan_gr.gpfifo_va,
                       (unsigned long long)GSP_CHAN_VA_STRIDE,
                       (unsigned)GSP_CHAN_VA_USED);
                return -1;
            }
            if (GSP_CHAN_VA_STRIDE < (uint64_t)GSP_CHAN_VA_USED) {
                printf("FALLO: GSP_CHAN_VA_STRIDE=0x%llx < used=0x%x\n",
                       (unsigned long long)GSP_CHAN_VA_STRIDE,
                       (unsigned)GSP_CHAN_VA_USED);
                return -1;
            }
            if ((chan_gr.gpfifo_va - chan.gpfifo_va) != GSP_CHAN_VA_STRIDE) {
                printf("FALLO: delta gpfifo idx=1 = 0x%llx (esperaba stride "
                       "0x%llx)\n",
                       (unsigned long long)(chan_gr.gpfifo_va - chan.gpfifo_va),
                       (unsigned long long)GSP_CHAN_VA_STRIDE);
                return -1;
            }
            printf("OK: ventanas VA idx=0/1 disjuntas (stride 0x%llx, "
                   "used=0x%x)\n", (unsigned long long)GSP_CHAN_VA_STRIDE,
                   (unsigned)GSP_CHAN_VA_USED);
        }
        if (chan_gr.pushbuf_va > GSP_GPFIFO_VA_MAX) {
            printf("FALLO: el pushbuffer del canal GR0 (0x%llx) no cabe en una "
                   "entrada de GPFIFO\n", (unsigned long long)chan_gr.pushbuf_va);
            return -1;
        }
        printf("OK: segundo canal en GR0 (motor %u, VAs +0x%llx)\n",
               p->engineType,
               (unsigned long long)(chan_gr.gpfifo_va - chan.gpfifo_va));
        if (chan_gr.mthdbuf_size != FAKE_MTHDBUF_SIZE ||
            chan_gr.mthdbuf_size != chan.mthdbuf_size) {
            printf("FALLO: GR0 mthdbuf=%u (COPY0 %u, cache fifo %u)\n",
                   chan_gr.mthdbuf_size, chan.mthdbuf_size, FAKE_MTHDBUF_SIZE);
            return -1;
        }
        {
            unsigned n = 0;
            uint32_t i, end = *q.wptr;

            for (i = wptr0; i != end; i = (i + 1u) % 63u) {
                const unsigned char *e = cmdq_base + 4096 +
                                         (unsigned long)(i % 63u) * 4096;
                const struct gsp_rpc_hdr *h =
                    (const struct gsp_rpc_hdr *)(e + sizeof(struct gsp_msg_elem));
                const rpc_gsp_rm_control *ct = (const rpc_gsp_rm_control *)(h + 1);

                if (h->function == NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL &&
                    ct->cmd == NV2080_CTRL_CMD_CE_GET_FAULT_METHOD_BUFFER_SIZE)
                    n++;
            }
            if (n != 1u) {
                printf("FALLO: GET_FAULT_METHOD_BUFFER_SIZE se mandó %u veces "
                       "(Linux fifo: una)\n", n);
                return -1;
            }
        }
        printf("OK: method buffer cacheado — una query fifo, GR0 reutiliza %u B\n",
               chan_gr.mthdbuf_size);
    }
    {
        struct gsp_compute cp;
        struct gsp_grctx ctx;
        GspQmdV05 qmd;
        unsigned qmd_off = 0, qmd_len = 0;
        const unsigned char *entry;
        const struct gsp_rpc_hdr *hdr;
        const rpc_gsp_rm_alloc *a;
        const rpc_gsp_rm_control *c;
        const NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS *p;

        if (check_sass_sets() != 0)
            return -1;
        if (check_family_caps() != 0)
            return -1;

        /* r535_gr_oneinit: golden antes del promote de usuario (canal GR0). */
        {
            struct gsp_grctx golden_ctx;
            struct gsp_vmm golden_vmm;
            struct gsp_chan golden_chan;

            if (gsp_vmm_init_on_rm(&golden_vmm, &v.rm, &pool, NVKM_RM_VASPACE_GOLDEN) != 0) {
                printf("FALLO: gsp_vmm_init_on_rm golden\n");
                return -1;
            }
            if (gsp_chan_init(&v.rm, &golden_vmm, &pool, &golden_chan,
                              golden_vmm.vaspace, 2u, NV2080_ENGINE_TYPE_GR0) != 0) {
                printf("FALLO: gsp_chan_init golden\n");
                gsp_vmm_fini_vaspace(&golden_vmm);
                return -1;
            }
            if (gsp_grctx_query(&v.rm, 0u, &golden_ctx) < 0) {
                printf("FALLO: gsp_grctx_query golden\n");
                gsp_chan_fini(&golden_chan);
                gsp_vmm_fini_vaspace(&golden_vmm);
                return -1;
            }
            if (gsp_grctx_promote(&v.rm, &golden_vmm, &pool, &golden_chan,
                                  &golden_ctx, 1) != 0) {
                printf("FALLO: gsp_grctx_promote golden\n");
                gsp_chan_fini(&golden_chan);
                gsp_vmm_fini_vaspace(&golden_vmm);
                return -1;
            }
            gsp_grctx_golden_publish(&golden_ctx);
            /* No hacer fini del canal/vaspace golden: UNSET_PAGE_DIRECTORY y FREE
             * desplazarían el wptr respecto a los fakes preencolados (+23..+25). */
            (void)golden_chan;
            (void)golden_vmm;
        }

        /* r535_gr_chan_new: promote_ctx de usuario antes de RM_ALLOC compute. */
        if (gsp_grctx_query(&v.rm, 0u, &ctx) < 0) {
            printf("FALLO: gsp_grctx_query\n");
            return -1;
        }
        if (gsp_grctx_promote(&v.rm, &v, &pool, &chan_gr, &ctx, 0) != 0) {
            printf("FALLO: gsp_grctx_promote usuario\n");
            return -1;
        }
        entry = cmdq_base + 4096 +
                (unsigned long)((wptr0 + G4E_SLOT_USER_PROMOTE) % 63) * 4096;
        hdr = (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        c = (const rpc_gsp_rm_control *)(hdr + 1);
        p = (const NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS *)(c + 1);
        if (c->cmd != NV2080_CTRL_CMD_GPU_PROMOTE_CTX ||
            c->hObject != v.rm.subdevice ||
            c->paramsSize != sizeof(*p) ||
            p->engineType != 1u || p->hChanClient != v.rm.client ||
            p->hObject != chan_gr.handle) {
            printf("FALLO: PROMOTE_CTX antes de compute (cmd=0x%08x obj=0x%08x)\n",
                   c->cmd, p->hObject);
            return -1;
        }
        printf("OK: PROMOTE_CTX usuario en índice %u antes de RM_ALLOC compute\n",
               G4E_SLOT_USER_PROMOTE);

        /* RM_ALLOC compute tras promote (r535_gr_chan_new). */
        entry = cmdq_base + 4096 +
                (unsigned long)((wptr0 + G4E_SLOT_COMPUTE_ALLOC) % 63) * 4096;
        hdr = (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        a = (const rpc_gsp_rm_alloc *)(hdr + 1);

        if (gsp_compute_init(&v.rm, &chan_gr, &cp) != 0) {
            printf("FALLO: gsp_compute_init\n");
            return -1;
        }
        if (a->hClass != BLACKWELL_COMPUTE_B || a->hObject != NVKM_RM_COMPUTE0 ||
            a->hParent != NVKM_RM_CHAN(1)) {
            printf("FALLO: compute cls=0x%x obj=0x%08x padre=0x%08x (tiene que "
                   "colgar del canal de GR0, 0x%08x)\n",
                   a->hClass, a->hObject, a->hParent, NVKM_RM_CHAN(1));
            return -1;
        }
        printf("OK: BLACKWELL_COMPUTE_B colgado del canal de GR0\n");

        gsp_compute_fill_qmd(&cp, &cp.saxpy, &qmd, 4);
        if (check_qmd_fields(&cp, &cp.saxpy, 4, &qmd) != 0)
            return -1;
        if (check_compute_params(&cp) != 0)
            return -1;
        /* Y el mismo QMD para matvec: otro programa, otro regcount, otra malla. */
        gsp_compute_fill_qmd(&cp, &cp.matvec, &qmd, 17);
        if (check_qmd_fields(&cp, &cp.matvec, 17, &qmd) != 0)
            return -1;
        if (check_mvq_params(&cp) != 0)
            return -1;
        if (check_mvq_layout() != 0)
            return -1;
        if (check_q4k_decode() != 0)
            return -1;
        if (check_mv_params(&cp) != 0)
            return -1;
        if (!cp.mv_mapped) {
            printf("FALLO: el staging de matvec no quedó mapeado\n");
            return -1;
        }
        if (check_mv_tiling(&cp) != 0)
            return -1;
        if (check_g6_resident() != 0)
            return -1;
        if (!cp.res_mapped) {
            printf("FALLO: el staging G6 no quedó mapeado\n");
            return -1;
        }
        /* El pushbuffer del canal de GR0, que es el que usa el compute. Mirar el
         * del CE aquí daba "pushbuffer sin QMD" con el encoder perfectamente
         * bien: los métodos estaban, pero en el otro canal. */
        chan_gr.pb_pos = 0;
        /* 6 dwords: SET_OBJECT(2) + WFI immd(1) + SEND_PCAS_A(2) + PCAS2 immd(1).
         * Un IMMD lleva el dato en la cabecera (bits 28:16) y NO tiene dword de
         * datos: los 32 B de antes eran el bug de 2026-07-29 (el dato suelto se
         * leía como la siguiente cabecera y el PBDMA levantaba PBDMA_ERROR). */
        if (gsp_compute_encode_qmd(&cp, &qmd, &qmd_off, &qmd_len) != 0 ||
            qmd_len != 24) {
            printf("FALLO: gsp_compute_encode_qmd (len=%u, esperaba 24)\n", qmd_len);
            return -1;
        }
        {
            const uint32_t *pb =
                (const uint32_t *)((const unsigned char *)chan_gr.pushbuf.va + qmd_off);
            const uint32_t *stored =
                (const uint32_t *)((const unsigned char *)cp.data.va + G4F_QMD_OFF);
            uint64_t qmd_va = cp.data_va + G4F_QMD_OFF;
            uint32_t want_pcas = (uint32_t)(qmd_va >> 8);
            unsigned j;

            if (memcmp(stored, qmd.words, sizeof(qmd.words)) != 0) {
                printf("FALLO: QMD no copiado a sysmem en +0x%x\n", G4F_QMD_OFF);
                return -1;
            }
            printf("OK: QMD v05 copiado a sysmem 0x%llx (+0x%x)\n",
                   (unsigned long long)qmd_va, G4F_QMD_OFF);

            /* SET_OBJECT con la clase de compute por subcanal 1. */
            if (pb[0] != ((NVC56F_DMA_INCR_OPCODE_VALUE << 29) | (1u << 16) |
                          (1u << 13) | ((NVCEC0_SET_OBJECT >> 2) & 0xfffu)) ||
                pb[1] != cp.cls) {
                printf("FALLO: el QMD no empieza por SET_OBJECT(cls=0x%04x "
                       "subc=1): %08x %08x\n", cp.cls, pb[0], pb[1]);
                return -1;
            }
            /* WFI immd con dato 0: bits 28:16 a cero y SIN dword detrás. */
            if (pb[2] != ((NVC56F_DMA_SEC_OP_IMMD_DATA_METHOD << 29) |
                          ((NVC86F_WFI >> 2) & 0xfffu))) {
                printf("FALLO: pushbuffer sin WFI del canal: %08x\n", pb[2]);
                return -1;
            }
            if (pb[3] != ((NVC56F_DMA_INCR_OPCODE_VALUE << 29) | (1u << 16) |
                          (1u << 13) | ((NVCEC0_SEND_PCAS_A >> 2) & 0xfffu)) ||
                pb[4] != want_pcas) {
                printf("FALLO: SEND_PCAS_A=0x%08x (esperaba 0x%08x va>>8)\n",
                       pb[4], want_pcas);
                return -1;
            }
            if (pb[5] != ((NVC56F_DMA_SEC_OP_IMMD_DATA_METHOD << 29) |
                          (NVCEC0_SEND_SIGNALING_PCAS2_B_PCAS_ACTION_INVALIDATE_COPY_SCHEDULE << 16) |
                          (1u << 13) |
                          ((NVCEC0_SEND_SIGNALING_PCAS2_B >> 2) & 0xfffu))) {
                printf("FALLO: SEND_SIGNALING_PCAS2_B: hdr=%08x\n", pb[5]);
                return -1;
            }
            for (j = 6; j < qmd_len / 4; j++) {
                if (pb[j] != 0u) {
                    printf("FALLO: pushbuffer compute tiene palabra extra pb[%u]=%08x\n",
                           j, pb[j]);
                    return -1;
                }
            }
            printf("OK: SET_OBJECT compute cls=0x%04x subcanal 1\n", cp.cls);
        }
        printf("OK: pushbuffer compute SEND_PCAS (%u B)\n", qmd_len);

        /* Payload PROMOTE_CTX (índice 16): layout y entradas del plan. */
        {
            const NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY *e;
            unsigned nmapped = 0, nonmapped = 0, j;

            entry = cmdq_base + 4096 +
                    (unsigned long)((wptr0 + G4E_SLOT_USER_PROMOTE) % 63) * 4096;
            hdr = (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
            c = (const rpc_gsp_rm_control *)(hdr + 1);
            p = (const NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS *)(c + 1);
            {
                unsigned want_entries = 0;

                for (j = 0; j < ctx.nr; j++) {
                    if (ctx.buf[j].buffer_id ==
                            NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_UNRESTRICTED_PRIV_ACCESS_MAP) {
                        continue;
                    }
                    want_entries++;
                }

                if (c->cmd != NV2080_CTRL_CMD_GPU_PROMOTE_CTX ||
                    c->hObject != v.rm.subdevice ||
                    c->paramsSize != sizeof(*p)) {
                    printf("FALLO: PROMOTE_CTX cmd=0x%08x obj=0x%08x params=%u (esperaba "
                           "0x%08x/0x%08x/%u)\n", c->cmd, c->hObject, c->paramsSize,
                           NV2080_CTRL_CMD_GPU_PROMOTE_CTX, v.rm.subdevice,
                           (unsigned)sizeof(*p));
                    return -1;
                }
                if (p->engineType != 1u || p->hChanClient != v.rm.client ||
                    p->hObject != chan_gr.handle) {
                    printf("FALLO: PROMOTE_CTX engineType=%u hChanClient=0x%08x "
                           "hObject=0x%08x\n", p->engineType, p->hChanClient, p->hObject);
                    return -1;
                }
                if (p->hClient != 0u || p->ChID != 0u || p->hVirtMemory != 0u ||
                    p->virtAddress != 0u || p->size != 0u) {
                    printf("FALLO: PROMOTE_CTX trae campos que upstream deja a cero\n");
                    return -1;
                }
                if (p->entryCount != want_entries) {
                    printf("FALLO: entryCount=%u y el plan de usuario tiene %u entradas "
                           "(plan %u, sin UNRESTRICTED)\n",
                           p->entryCount, want_entries, ctx.nr);
                    return -1;
                }
            }

            e = &p->promoteEntry[0];
            if (e->bufferId != NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_MAIN ||
                !e->bInitialize || e->gpuPhysAddr == 0u || e->size != 0x49000u ||
                e->physAttr != NV2080_CTRL_GPU_PROMOTE_CTX_PHYS_ATTR_DEFAULT ||
                e->gpuVirtAddr == 0u) {
                printf("FALLO: entrada MAIN pa=0x%llx va=0x%llx sz=0x%llx attr=%u "
                       "init=%u\n", (unsigned long long)e->gpuPhysAddr,
                       (unsigned long long)e->gpuVirtAddr,
                       (unsigned long long)e->size, e->physAttr, e->bInitialize);
                return -1;
            }

            for (j = 0; j < p->entryCount; j++) {
                e = &p->promoteEntry[j];

                if (e->bNonmapped) {
                    nonmapped++;
                    if (e->gpuVirtAddr != 0u) {
                        printf("FALLO: la entrada %u dice bNonmapped y trae VA 0x%llx\n",
                               j, (unsigned long long)e->gpuVirtAddr);
                        return -1;
                    }
                    continue;
                }
                nmapped++;
                if (e->gpuVirtAddr == 0u) {
                    printf("FALLO: la entrada %u (id %u) va mapeada y sin VA\n", j,
                           e->bufferId);
                    return -1;
                }
                /* Y la alineación de la VA, que es la prueba de que el mapeo respeta lo
                 * que dijo el plan y no sólo la página: el attribute CB de 24 MiB pide
                 * 32 MiB, y con la página saldría alineado a 2 MiB — que también
                 * "parece" bien. */
                if (e->bufferId == NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_ATTRIBUTE_CB &&
                    (e->gpuVirtAddr & 0x1ffffffull) != 0u) {
                    printf("FALLO: el ATTRIBUTE_CB está en 0x%llx, no alineado a "
                           "32 MiB\n", (unsigned long long)e->gpuVirtAddr);
                    return -1;
                }
                if (e->bufferId ==
                        NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_UNRESTRICTED_PRIV_ACCESS_MAP) {
                    printf("FALLO: UNRESTRICTED_PRIV_ACCESS_MAP no debe ir en promote "
                           "de canal usuario\n");
                    return -1;
                }
                if (e->bufferId ==
                        NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_PRIV_ACCESS_MAP) {
                    uint64_t pam_pa = 0, pam_pte = 0;

                    if (gsp_vmm_translate(&v, e->gpuVirtAddr, &pam_pa, &pam_pte) != 0) {
                        printf("FALLO: PRIV_ACCESS_MAP sin traducción en VMM usuario\n");
                        return -1;
                    }
                }
            }
            if (nonmapped != 0u || nmapped != p->entryCount) {
                printf("FALLO: %u entradas sin mapear y %u mapeadas de %u (entryCount=%u)\n",
                       nonmapped, nmapped, p->entryCount, p->entryCount);
                return -1;
            }
            printf("OK: PROMOTE_CTX usuario — %u entradas (%u B), MAIN con física, "
                   "PRIV_ACCESS_MAP mapeado, ATTRIBUTE_CB alineado a 32 MiB\n",
                   p->entryCount, c->paramsSize);
        }

        gsp_compute_fini(&cp);
    }

    /* Teardown G4e/G4f antes del vmm_fini de check_vmm (este test es autónomo). */
    base = *rpc.rptr;
    for (i = 0; i < 4; i++) {
        fake_rpc_post_payload(lo, (base + i) % 63, NV_VGPU_MSG_FUNCTION_FREE,
                              0, NULL, 0);
    }
    fake_rpc_post_payload(lo, (base + 4) % 63, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL,
                          0, ctrl_ok, (uint32_t)sizeof(ctrl_ok));
    for (i = 0; i < 5; i++) {
        fake_rpc_post_payload(lo, (base + 5 + i) % 63, NV_VGPU_MSG_FUNCTION_FREE,
                              0, NULL, 0);
    }
    msgq->tx.writePtr = (base + 10) % 63;

    /* Orden inverso al de creación: el canal de GR0 después de su compute (que ya
     * se soltó arriba) y antes del CE. */
    gsp_chan_fini(&chan_gr);
    gsp_ce_fini(&ce);
    gsp_chan_fini(&chan);
    gsp_vmm_fini(&v);
    (void)test_pat;
    printf("OK: fini compute → canal GR0 → CE → canal COPY0 → vaspace\n");
    return 0;
}

static int check_fini(const struct gsp_libos *lo)
{
    struct gsp_rpc rpc;
    struct gsp_cmdq q;
    struct gsp_rm rm;
    struct gsp_msgq_headers *msgq =
        (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->msgq_offset);
    unsigned char *cmdq_base = (unsigned char *)lo->shm.va + lo->cmdq_offset;
    /* Puntero no-NULL que nunca se desreferencia: el stub de clear_master lo ignora. */
    struct lx_pci_dev *fake_pdev = (struct lx_pci_dev *)(void *)&rpc;
    uint32_t base, wptr0;
    unsigned i;
    /* Orden inverso al de la reserva: el hijo antes que el padre. */
    const uint32_t want_free[3] = { NVKM_RM_SUBDEVICE, NVKM_RM_DEVICE, NVKM_RM_CLIENT(0) };

    printf("sizeof rpc_unloading_guest_driver=%zu\n",
           sizeof(rpc_unloading_guest_driver_v1F_07));

    if (gsp_rpc_init(lo, &rpc) != 0) { printf("FALLO: rpc_init (fini)\n"); return -1; }
    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: cmdq_init (fini)\n"); return -1; }
    /* Tests anteriores dejan la cmdq casi llena; fini necesita hueco libre. */
    {
        struct gsp_msgq_headers *cmdq =
            (struct gsp_msgq_headers *)((unsigned char *)lo->shm.va + lo->cmdq_offset);

        cmdq->tx.writePtr = 0;
        cmdq->rx.readPtr = 0;
        msgq->tx.writePtr = 0;
        msgq->rx.readPtr = 0;
    }

    memset(&rm, 0, sizeof(rm));
    rm.q = &q;
    rm.rpc = &rpc;
    rm.client = NVKM_RM_CLIENT(0);
    rm.device = NVKM_RM_DEVICE;
    rm.subdevice = NVKM_RM_SUBDEVICE;
    rm.ready = 1;

    /* Cuatro respuestas OK: tres FREE y el unload, en el orden en que se piden. */
    base = *rpc.rptr;
    for (i = 0; i < 3; i++) {
        fake_rpc_post_payload(lo, (base + i) % 63, NV_VGPU_MSG_FUNCTION_FREE, 0, NULL, 0);
    }
    fake_rpc_post_payload(lo, (base + 3) % 63,
                          NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER, 0, NULL, 0);
    msgq->tx.writePtr = (base + 4) % 63;

    fake_unload_pending = 1;
    fsp_mbox_reads = 0;
    pci_master_cleared = 0;
    wptr0 = *q.wptr;

    if (gsp_fini(&rm, &q, &rpc, fake_pdev) != 0) {
        printf("FALLO: gsp_fini con todo respondiendo bien\n");
        return -1;
    }
    fake_unload_pending = 0;

    /* Las tres peticiones de FREE. */
    for (i = 0; i < 3; i++) {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + i) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const uint32_t *p = (const uint32_t *)(hdr + 1);

        if (hdr->function != NV_VGPU_MSG_FUNCTION_FREE) {
            printf("FALLO: free %u con function=%u (esperaba %u; 27 era "
                   "DMA_FILL_PTE_MEM)\n", i, hdr->function,
                   NV_VGPU_MSG_FUNCTION_FREE);
            return -1;
        }
        if (hdr->length != sizeof(*hdr) + 16u) {
            printf("FALLO: free %u length=%u\n", i, hdr->length);
            return -1;
        }
        if (p[0] != NVKM_RM_CLIENT(0) || p[1] != 0 || p[2] != want_free[i] || p[3] != 0) {
            printf("FALLO: free %u params {0x%08x,0x%08x,0x%08x,0x%08x}, "
                   "esperaba objeto 0x%08x\n", i, p[0], p[1], p[2], p[3], want_free[i]);
            return -1;
        }
    }
    printf("OK: FREE (fn=%u) de subdevice, device y cliente, en ese orden\n",
           NV_VGPU_MSG_FUNCTION_FREE);

    /* Y el aviso de descarga: 8 B a cero, que es el unload que no es suspensión. */
    {
        const unsigned char *entry = cmdq_base + 4096 +
                                     (unsigned long)((wptr0 + 3) % 63) * 4096;
        const struct gsp_rpc_hdr *hdr =
            (const struct gsp_rpc_hdr *)(entry + sizeof(struct gsp_msg_elem));
        const rpc_unloading_guest_driver_v1F_07 *u =
            (const rpc_unloading_guest_driver_v1F_07 *)(hdr + 1);

        if (hdr->function != NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER) {
            printf("FALLO: unload con function=%u (esperaba %u)\n",
                   hdr->function, NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER);
            return -1;
        }
        if (hdr->length != sizeof(*hdr) + sizeof(*u)) {
            printf("FALLO: unload length=%u (esperaba %zu)\n",
                   hdr->length, sizeof(*hdr) + sizeof(*u));
            return -1;
        }
        if (u->bInPMTransition != 0 || u->bGc6Entering != 0 || u->newLevel != 0) {
            printf("FALLO: unload pm=%u gc6=%u level=%u (los tres a cero si no "
                   "es suspensión)\n", u->bInPMTransition, u->bGc6Entering, u->newLevel);
            return -1;
        }
    }
    printf("OK: UNLOADING_GUEST_DRIVER (fn=%u) con %zu B a cero\n",
           NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER,
           sizeof(rpc_unloading_guest_driver_v1F_07));

    if (pci_master_cleared != 1) {
        printf("FALLO: bus master quitado %u veces\n", pci_master_cleared);
        return -1;
    }
    if (rm.ready) {
        printf("FALLO: los objetos de RM siguen marcados como vivos\n");
        return -1;
    }
    printf("OK: bus master quitado y objetos de RM marcados como muertos\n");

    /* El caso que importa para el host: aunque no haya nada vivo con lo que
     * negociar, el DMA se corta igual. Es exactamente el escenario del cuelgue
     * del 2026-07-25, donde el apagado ordenado no era posible. */
    {
        struct gsp_rm dead;
        struct gsp_cmdq dead_q;
        struct gsp_rpc dead_rpc;

        memset(&dead, 0, sizeof(dead));
        memset(&dead_q, 0, sizeof(dead_q));
        memset(&dead_rpc, 0, sizeof(dead_rpc));
        pci_master_cleared = 0;
        if (gsp_fini(&dead, &dead_q, &dead_rpc, fake_pdev) == 0) {
            printf("FALLO: un apagado sin RPC vivo se dio por bueno\n");
            return -1;
        }
        if (pci_master_cleared != 1) {
            printf("FALLO: sin RPC vivo NO se quitó el bus master — "
                   "es justo cuando más falta hace\n");
            return -1;
        }
        printf("OK: sin RPC vivo el apagado falla pero corta el DMA igual\n");
    }

    /* Y sin pci_dev no hay forma de cortarlo: tiene que decirlo, no fingir. */
    if (gsp_fini(&rm, &q, &rpc, NULL) == 0) {
        printf("FALLO: un apagado sin pci_dev se dio por bueno\n");
        return -1;
    }
    printf("OK: sin pci_dev el apagado se declara fallido\n");
    return 0;
}

static int check_cmdq(const struct gsp_libos *lo)
{
    struct gsp_cmdq q;
    struct gsp_sysinfo si;
    const struct gsp_msg_elem *elem;
    const struct gsp_rpc_hdr *rpc;
    unsigned char *cmdq_base = (unsigned char *)lo->shm.va + lo->cmdq_offset;

    /* GspSystemInfo es el contrato con el firmware: si el tamaño baila, RM lee
     * los campos desplazados. Con el layout de r570 y alineación natural salen
     * 928 B; se imprime para que un cambio de header se note aquí. */
    printf("sizeof(GspSystemInfo) = %zu\n", sizeof(GspSystemInfo));
    if (sizeof(GspSystemInfo) != 928) { printf("FALLO: tamaño de GspSystemInfo\n"); return -1; }

    if (gsp_cmdq_init(lo, &q) != 0) { printf("FALLO: gsp_cmdq_init\n"); return -1; }
    if (q.cnt != 63) { printf("FALLO: cnt=%u\n", q.cnt); return -1; }
    if (*q.wptr != 0) { printf("FALLO: wptr inicial\n"); return -1; }

    memset(&si, 0, sizeof(si));
    si.bar0_phys = 0xf0000000ull;
    si.bar1_phys = 0xe0000000ull;
    si.bar3_phys = 0xd0000000ull;
    si.bdf = 0x100;
    si.vendor_id = 0x10de;
    si.device_id = 0x2f18;
    si.revision_id = 0xa1;

    if (gsp_cmdq_set_system_info(&q, &si) != 0) { printf("FALLO: set_system_info\n"); return -1; }

    /* El elemento va detrás de la primera página (la cabecera de la cola). */
    elem = (const struct gsp_msg_elem *)(cmdq_base + 4096);
    rpc = (const struct gsp_rpc_hdr *)(elem + 1);
    if (rpc->header_version != 0x03000000u) { printf("FALLO: header_version\n"); return -1; }
    if (rpc->signature != 0x43505256u) { printf("FALLO: signature 0x%08x\n", rpc->signature); return -1; }
    if (rpc->function != 72u) { printf("FALLO: function %u\n", rpc->function); return -1; }
    if (rpc->length != sizeof(struct gsp_rpc_hdr) + sizeof(GspSystemInfo)) {
        printf("FALLO: length %u\n", rpc->length);
        return -1;
    }
    /* El payload tiene que haber llegado entero. */
    {
        const GspSystemInfo *info = (const GspSystemInfo *)(rpc + 1);
        if (info->gpuPhysAddr != si.bar0_phys || info->gpuPhysFbAddr != si.bar1_phys ||
            info->PCIDeviceID != (((uint32_t)0x2f18 << 16) | 0x10de)) {
            printf("FALLO: el payload no cuadra\n");
            return -1;
        }
    }
    /* Checksum: XOR de todo el elemento en u64, plegado; con el campo ya puesto
     * el resultado tiene que dar cero. */
    {
        uint32_t len = (48u + rpc->length + 4095u) & ~4095u;
        const uint64_t *p = (const uint64_t *)elem;
        uint64_t csum = 0;
        for (uint32_t i = 0; i < len / 8u; i++) csum ^= p[i];
        if (((uint32_t)(csum >> 32) ^ (uint32_t)csum) != 0) {
            printf("FALLO: checksum no cierra\n");
            return -1;
        }
    }
    if (elem->elem_count != 1 || elem->sequence != 0) {
        printf("FALLO: elem_count=%u sequence=%u\n", elem->elem_count, elem->sequence);
        return -1;
    }
    if (*q.wptr != 1) { printf("FALLO: wptr=%u tras un RPC\n", *q.wptr); return -1; }
    printf("OK: SET_SYSTEM_INFO encolado (1 página, checksum cierra)\n");

    if (gsp_cmdq_set_registry(&q) != 0) { printf("FALLO: set_registry\n"); return -1; }
    {
        const struct gsp_msg_elem *e2 = (const struct gsp_msg_elem *)(cmdq_base + 4096 + 4096);
        const struct gsp_rpc_hdr *r2 = (const struct gsp_rpc_hdr *)(e2 + 1);
        const PACKED_REGISTRY_TABLE *t = (const PACKED_REGISTRY_TABLE *)(r2 + 1);
        const PACKED_REGISTRY_ENTRY *en = (const PACKED_REGISTRY_ENTRY *)(t + 1);
        const char *name0 = (const char *)t + en[0].nameOffset;

        if (r2->function != 73u) { printf("FALLO: fn registry %u\n", r2->function); return -1; }
        if (t->numEntries != 3) { printf("FALLO: numEntries %u\n", t->numEntries); return -1; }
        if (en[0].type != 1 || en[0].data != 1) { printf("FALLO: entrada 0\n"); return -1; }
        if (strcmp(name0, "RMSecBusResetEnable")) {
            printf("FALLO: nombre 0 = %s\n", name0);
            return -1;
        }
        if (e2->sequence != 1) { printf("FALLO: sequence no avanza\n"); return -1; }
        printf("OK: SET_REGISTRY encolado (3 claves, nombres en su offset)\n");
    }
    if (*q.wptr != 2) { printf("FALLO: wptr=%u tras dos RPCs\n", *q.wptr); return -1; }
    return 0;
}

/* GSP_RUN_CPU_SEQUENCER: buffer grande y CORE_RESET (falcon reset, no RISC-V). */
static int check_cpu_seq(void)
{
    unsigned char buf[8192];
    uint32_t *w = (uint32_t *)buf;

    memset(buf, 0, sizeof(buf));
    w[0] = 16354u;
    w[1] = 0u;
    if (gsp_cpu_seq_run(buf, 6296u) != 0) {
        printf("FALLO: cpu_seq payload 6296 B (cmdIndex=0)\n");
        return -1;
    }

    memset(buf, 0, sizeof(buf));
    w[0] = 100u;
    w[1] = 1u;
    w[10] = 6u; /* GSP_SEQ_BUF_OPCODE_CORE_RESET */
    if (gsp_cpu_seq_run(buf, 44u) != 0) {
        printf("FALLO: cpu_seq CORE_RESET\n");
        return -1;
    }
    printf("OK: cpu_seq payload 6296 B y CORE_RESET (falcon reset)\n");
    return 0;
}

/* Paso 6: el paquete COT, contra un FSP simulado. */
static int check_cot(const struct gsp_wpr *wpr)
{
    struct gsp_libos lo;
    struct fmc_image img;
    struct fmc_staged staged;
    const struct gsp_fw_blob *fmc = gsp_fw_get(GSP_FW_FMC);

    if (gsp_libos_prepare(wpr, &lo) != 0 || !fmc ||
        fmc_lx_parse(fmc->data, fmc->len, &img) != 0 ||
        fmc_lx_stage(&img, &staged) != 0) {
        printf("FALLO: preparando el payload del COT\n");
        return -1;
    }

    fake_fsp_reset();
    if (fsp_lx_boot_gsp_fmc(&staged, &lo, wpr) != 0) {
        printf("FALLO: fsp_lx_boot_gsp_fmc\n");
        return -1;
    }
    /* 868 B = 2 DWORD de cabeceras + 860 de payload COT. */
    if (fsp_sent_dwords != 868u / 4u) {
        printf("FALLO: se enviaron %u DWORDs, esperados %u\n", fsp_sent_dwords, 868u / 4u);
        return -1;
    }
    if (fsp_sent[0] != 0xc0000000u) { printf("FALLO: MCTP 0x%08x\n", fsp_sent[0]); return -1; }
    if (fsp_sent[1] != 0x1410de7eu) { printf("FALLO: NVDM 0x%08x\n", fsp_sent[1]); return -1; }

    const unsigned char *cot = (const unsigned char *)&fsp_sent[2];
    uint16_t version, size;
    uint64_t fmc_off, args_off, frts_off;
    uint32_t frts_size;
    memcpy(&version, cot + 0, 2);
    memcpy(&size, cot + 2, 2);
    memcpy(&fmc_off, cot + 4, 8);
    memcpy(&frts_off, cot + 24, 8);
    memcpy(&frts_size, cot + 32, 4);
    memcpy(&args_off, cot + 852, 8);

    if (version != 2 || size != 860) {
        printf("FALLO: COT v%u size %u (esperado v2/860)\n", version, size);
        return -1;
    }
    if (fmc_off != staged.img.phys) { printf("FALLO: gspFmcSysmemOffset\n"); return -1; }
    if (args_off != lo.boot_params.phys) { printf("FALLO: gspBootArgsSysmemOffset\n"); return -1; }
    if (frts_off != wpr->rsvd_size || frts_size != 0x100000u) {
        printf("FALLO: FRTS %llu/%u\n", (unsigned long long)frts_off, frts_size);
        return -1;
    }
    /* La cadena de firma va en su sitio y solo ocupa lo suyo: el resto de los
     * campos de 384 B (que gh100 sí llena) tiene que seguir a cero. */
    if (memcmp(cot + 36, staged.hash.va, 48) || memcmp(cot + 84, staged.pkey.va, 97) ||
        memcmp(cot + 468, staged.sig.va, 96)) {
        printf("FALLO: la cadena de firma no está en su offset\n");
        return -1;
    }
    for (unsigned i = 84 + 97; i < 468; i++) {
        if (cot[i]) { printf("FALLO: relleno de publicKey no nulo en %u\n", i); return -1; }
    }
    /* frtsSysmemOffset/Size se quedan a cero: la FRTS va en VRAM. */
    for (unsigned i = 12; i < 24; i++) {
        if (cot[i]) { printf("FALLO: frtsSysmem no nulo\n"); return -1; }
    }
    printf("OK: COT 868 B, cabeceras y firma en su sitio, FSP simulado lo acepta\n");

    /* Y que un rechazo del FSP se detecte en vez de darse por bueno. */
    fake_fsp_reset();
    fsp_reply_error = 0x1234u;
    if (fsp_lx_boot_gsp_fmc(&staged, &lo, wpr) == 0) {
        printf("FALLO: un COT rechazado se dio por bueno\n");
        return -1;
    }
    fsp_reply_error = 0;
    printf("OK: el rechazo del FSP se detecta\n");

    /* Y el cuelgue del 2026-07-27: la tarjeta se cae del bus entre el poll de
     * fsp_wait_reply (que ve la cola con datos) y el de fsp_recv (que ya lee
     * all-ones). Como 0xffffffff == 0xffffffff cumple `head == tail`, aquello se
     * anunciaba por serie como "respuesta del FSP de tamaño raro (0)" —un FSP
     * que contesta mal— cuando lo que había pasado era que la GPU estaba muerta,
     * y el host se llevó por delante un panic al cerrar QEMU. Que devuelva error
     * no basta: antes también lo hacía. Lo que se comprueba es que ahora lo
     * diagnostica como caída, y la señal es que intenta la recuperación por
     * espacio de configuración, cosa que el camino de "tamaño raro" nunca hacía. */
    fake_fsp_reset();
    fake_die_after_mtail = 1;
    if (fsp_lx_boot_gsp_fmc(&staged, &lo, wpr) == 0) {
        printf("FALLO: el COT se dio por bueno con la GPU fuera del bus\n");
        return -1;
    }
    if (fake_recover_calls == 0) {
        printf("FALLO: all-ones tomado por cola vacía, no por GPU caída del bus\n");
        return -1;
    }
    fake_fsp_reset();
    printf("OK: GPU caída del bus en la respuesta al COT — se dice, no se disfraza de "
           "'tamaño raro'\n");

    /* Y el fallo del ciclo de HW del 2026-07-28, que es OTRO: el FSP acepta el
     * COT, el FMC arranca y **el enlace se resetea**. El driver se rindió a los
     * 3 ms cantando "se cayó del bus"; el monitor del root port enseñó el enlace
     * volviendo a 32 GT/s 207 ms después (32 → 2.5 → 32), o sea que la tarjeta no
     * estaba muerta, estaba reentrenando. Aquí se simula igual: muere dentro del
     * bucle del FMC, el espacio de configuración sigue contestando (vfio emula el
     * id) y vuelve a los 300 ms. Tiene que arrancar. */
    fake_fsp_reset();
    fake_die_after_mbox0 = 2;
    fake_revive_after_mdelays = 300;
    fake_recover_ret = 0;
    if (fsp_lx_boot_gsp_fmc(&staged, &lo, wpr) != 0) {
        printf("FALLO: un reset del enlace de 300 ms se trató como muerte de la GPU\n");
        return -1;
    }
    if (fake_gpu_gone) {
        printf("FALLO: la simulación no llegó a revivir la GPU (prueba inválida)\n");
        return -1;
    }
    fake_fsp_reset();
    printf("OK: reset del enlace a mitad del FMC — se espera y se arranca, no se "
           "declara muerta a los 3 ms\n");

    /* La cara B, para que la espera no se coma el diagnóstico verdadero: si NO
     * vuelve, sigue siendo un fallo, y acotado (el plazo es finito). */
    fake_fsp_reset();
    fake_die_after_mbox0 = 2;
    fake_revive_after_mdelays = 0;   /* no vuelve nunca */
    fake_recover_ret = 0;
    if (fsp_lx_boot_gsp_fmc(&staged, &lo, wpr) == 0) {
        printf("FALLO: la GPU no volvió y el arranque se dio por bueno\n");
        return -1;
    }
    fake_fsp_reset();
    printf("OK: si no vuelve dentro del plazo, sigue siendo caída del bus\n");

    if (check_rpc(&lo) != 0)
        return -1;
    if (check_cmdq(&lo) != 0)
        return -1;
    if (check_rpc_sync(&lo) != 0)
        return -1;
    if (check_rm_objects(&lo) != 0)
        return -1;
    if (check_classlist(&lo) != 0)
        return -1;
    if (check_vmm_ampere(&lo) != 0)
        return -1;
    if (check_vmm(&lo) != 0)
        return -1;
    if (check_ptop() != 0)
        return -1;
    if (check_hs_v2_ampere_payload() != 0)
        return -1;
    if (check_ampere_sec2_falcon_base() != 0)
        return -1;
    if (check_grctx() != 0)
        return -1;
    if (check_pramin() != 0)
        return -1;
    if (check_bar1_walk() != 0)
        return -1;
    if (check_bar1_map() != 0)
        return -1;
    if (check_bar1_refuse_poison() != 0)
        return -1;
    if (check_rc_triggered() != 0)
        return -1;
    if (check_doorbell_kick_by_family() != 0)
        return -1;
    if (check_doorbell_ampere_copy2_resolved() != 0)
        return -1;
    if (check_ga107_dead_boot0() != 0)
        return -1;
    if (check_cpu_seq() != 0)
        return -1;
    if (check_g4e_chan_ce(&lo) != 0)
        return -1;
    if (check_fini(&lo) != 0)
        return -1;

    fmc_lx_stage_release(&staged);
    gsp_libos_release(&lo);
    return 0;
}

int main(int argc, char **argv)
{
    if (argc < 4) {
        fprintf(stderr, "uso: %s <gsp-…bin> <bootloader-…bin> <fmc-…bin>\n", argv[0]);
        return 2;
    }
    if (load_blob(GSP_FW_UCODE, argv[1]) || load_blob(GSP_FW_BOOTLOADER, argv[2]) ||
        load_blob(GSP_FW_FMC, argv[3]))
        return 2;

    struct gsp_rm_fw rm;
    struct gsp_wpr wpr;
    if (check_radix3(&rm) != 0)
        return 1;
    if (check_wpr(&rm, &wpr) != 0)
        return 1;
    if (check_wpr_ampere(&rm) != 0)
        return 1;
    if (check_libos(&wpr) != 0)
        return 1;
    if (check_cot(&wpr) != 0)
        return 1;
    gsp_wpr_release(&wpr);
    gsp_rm_release(&rm);
    return 0;
}
