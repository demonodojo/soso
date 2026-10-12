/* ath11k / WCN6855 (Steam Deck OLED) — transporte MHI sobre PCIe.
 *
 * Referencias en el árbol Linux pinneado (`lxdde/linux/`):
 *   drivers/bus/mhi/common.h            — registros MHI/BHI/BHIe y TRE
 *   drivers/bus/mhi/host/pm.c           — mhi_async_power_up, ready transition
 *   drivers/bus/mhi/host/init.c         — mhi_init_mmio (registros de contexto)
 *   drivers/bus/mhi/host/boot.c         — descarga de firmware por BHI/BHIe
 *   drivers/net/wireless/ath/ath11k/mhi.c   — canales y eventos del chip
 *   drivers/net/wireless/ath/ath11k/pci.c   — IDs PCI y ventana de registros
 *
 * Este port NO compila el driver de Linux: lo reimplementa acotado, como el
 * port iwlwifi. Todo el acceso MMIO pasa por `ath11k_read32`/`ath11k_write32`,
 * que en el kernel son el BAR y en el hostcheck un modelo del dispositivo; ésa
 * es la costura que permite probar el bring-up sin la Deck delante.
 */
#ifndef ATH11K_INTERNAL_H
#define ATH11K_INTERNAL_H

#include <stddef.h>
#include <stdint.h>

struct lx_pci_dev;

/* ---- IDs PCI (ath11k/pci.c:27) ---- */
#define QCOM_VENDOR_ID       0x17cb
#define QCA6390_DEVICE_ID    0x1101
#define WCN6855_DEVICE_ID    0x1103
#define QCN9074_DEVICE_ID    0x1104

/* ---- Registros MHI (bus/mhi/common.h:13) ---- */
#define MHIREGLEN            0x00
#define MHIVER               0x08
#define MHICFG               0x10
#define CHDBOFF              0x18
#define ERDBOFF              0x20
#define BHIOFF               0x28
#define BHIEOFF              0x2c
#define DEBUGOFF             0x30
#define MHICTRL              0x38
#define MHISTATUS            0x48
#define CCABAP_LOWER         0x58
#define CCABAP_HIGHER        0x5c
#define ECABAP_LOWER         0x60
#define ECABAP_HIGHER        0x64
#define CRCBAP_LOWER         0x68
#define CRCBAP_HIGHER        0x6c
#define CRDB_LOWER           0x70
#define CRDB_HIGHER          0x74
#define MHICTRLBASE_LOWER    0x80
#define MHICTRLBASE_HIGHER   0x84
#define MHICTRLLIMIT_LOWER   0x88
#define MHICTRLLIMIT_HIGHER  0x8c
#define MHIDATABASE_LOWER    0x98
#define MHIDATABASE_HIGHER   0x9c
#define MHIDATALIMIT_LOWER   0xa0
#define MHIDATALIMIT_HIGHER  0xa4

/* Campos (common.h:107) */
#define MHICTRL_RESET_MASK        (1u << 1)
#define MHICTRL_MHISTATE_SHIFT    8
#define MHICTRL_MHISTATE_MASK     0x0000ff00u
#define MHISTATUS_MHISTATE_SHIFT  8
#define MHISTATUS_MHISTATE_MASK   0x0000ff00u
#define MHISTATUS_SYSERR_MASK     (1u << 2)
#define MHISTATUS_READY_MASK      (1u << 0)

#define MHICFG_NER_SHIFT     16
#define MHICFG_NER_MASK      0x00ff0000u
#define MHICFG_NHWER_SHIFT   24
#define MHICFG_NHWER_MASK    0xff000000u

/* ---- Registros BHI, relativos a la ventana que anuncia BHIOFF ---- */
#define BHI_IMGADDR_LOW      0x08
#define BHI_IMGADDR_HIGH     0x0c
#define BHI_IMGSIZE          0x10
#define BHI_IMGTXDB          0x18
#define BHI_INTVEC           0x20
#define BHI_EXECENV          0x28
#define BHI_STATUS           0x2c
#define BHI_ERRCODE          0x30

#define BHI_STATUS_SHIFT     30
#define BHI_STATUS_MASK      0xc0000000u
#define BHI_STATUS_RESET     0x00
#define BHI_STATUS_SUCCESS   0x02
#define BHI_STATUS_ERROR     0x03

/* ---- Registros BHIe, relativos a BHIEOFF ---- */
#define BHIE_TXVECADDR_LOW_OFFS   0x2c
#define BHIE_TXVECADDR_HIGH_OFFS  0x30
#define BHIE_TXVECSIZE_OFFS       0x34
#define BHIE_TXVECDB_OFFS         0x3c
#define BHIE_TXVECSTATUS_OFFS     0x44

#define BHIE_TXVECSTATUS_STATUS_SHIFT      30
#define BHIE_TXVECSTATUS_STATUS_MASK       0xc0000000u
#define BHIE_TXVECSTATUS_STATUS_RESET      0x00
#define BHIE_TXVECSTATUS_STATUS_XFER_COMPL 0x02
#define BHIE_TXVECSTATUS_STATUS_ERROR      0x03
#define BHIE_TXVECSTATUS_SEQNUM_MASK       0x3fffffffu

/* ---- Entornos de ejecución (include/linux/mhi.h:123) ---- */
enum mhi_ee {
    MHI_EE_PBL = 0,
    MHI_EE_SBL = 1,
    MHI_EE_AMSS = 2,
    MHI_EE_RDDM = 3,
    MHI_EE_WFW = 4,
    MHI_EE_PTHRU = 5,
    MHI_EE_EDL = 6,
    MHI_EE_FP = 7,
    MHI_EE_MAX,
};

/* PBL y sus variantes: el único punto legítimo para encender (pm.c,
 * MHI_POWER_UP_CAPABLE / MHI_IN_PBL). */
#define MHI_IN_PBL(ee) \
    ((ee) == MHI_EE_PBL || (ee) == MHI_EE_PTHRU || (ee) == MHI_EE_EDL)
#define MHI_POWER_UP_CAPABLE(ee) (MHI_IN_PBL(ee) || (ee) == MHI_EE_AMSS)

/* ---- Estados MHI ---- */
enum mhi_state {
    MHI_STATE_RESET = 0x0,
    MHI_STATE_READY = 0x1,
    MHI_STATE_M0 = 0x2,
    MHI_STATE_M1 = 0x3,
    MHI_STATE_M2 = 0x4,
    MHI_STATE_M3 = 0x5,
    MHI_STATE_M3_FAST = 0x6,
    MHI_STATE_BHI = 0x7,
    MHI_STATE_SYS_ERR = 0xff,
};

/* ---- Contextos en memoria (common.h:255) ----
 * Los tres son de 24 bytes con los u64 alineados a 4: el dispositivo los lee
 * tal cual, así que el empaquetado forma parte del contrato.
 */
struct mhi_event_ctxt {
    uint32_t intmod;
    uint32_t ertype;
    uint32_t msivec;
    uint64_t rbase;
    uint64_t rlen;
    uint64_t rp;
    uint64_t wp;
} __attribute__((packed, aligned(4)));

struct mhi_chan_ctxt {
    uint32_t chcfg;
    uint32_t chtype;
    uint32_t erindex;
    uint64_t rbase;
    uint64_t rlen;
    uint64_t rp;
    uint64_t wp;
} __attribute__((packed, aligned(4)));

struct mhi_cmd_ctxt {
    uint32_t reserved0;
    uint32_t reserved1;
    uint32_t reserved2;
    uint64_t rbase;
    uint64_t rlen;
    uint64_t rp;
    uint64_t wp;
} __attribute__((packed, aligned(4)));

/* Un TRE: puntero + dos dwords (common.h, «Transfer descriptor macros»). */
struct mhi_tre {
    uint64_t ptr;
    uint32_t dword0;
    uint32_t dword1;
} __attribute__((packed, aligned(4)));

#define CHAN_CTX_CHSTATE_MASK   0x000000ffu
#define CHAN_CTX_BRSTMODE_SHIFT 8
#define CHAN_CTX_POLLCFG_SHIFT  10

enum mhi_ch_state {
    MHI_CH_STATE_DISABLED = 0,
    MHI_CH_STATE_ENABLED = 1,
    MHI_CH_STATE_RUNNING = 2,
    MHI_CH_STATE_SUSPENDED = 3,
    MHI_CH_STATE_STOP = 4,
    MHI_CH_STATE_ERROR = 5,
};

/* Doorbell sin ráfagas: es lo que usan todos los canales de ath11k
 * (`MHI_DB_BRST_DISABLE` en ath11k/mhi.c). */
#define MHI_DB_BRST_DISABLE 2

/* ---- Vector de descarga BHIe (boot.c: struct bhi_vec_entry) ---- */
struct bhi_vec_entry {
    uint64_t dma_addr;
    uint64_t size;
} __attribute__((packed));

/* ---- Configuración del chip ----
 * Canales de WCN6855 (ath11k/mhi.c, tabla qca6390 que WCN6855 comparte):
 * LOOPBACK 0/1 e IPCR 20/21. QMI viaja por IPCR.
 */
#define ATH11K_MHI_CH_LOOPBACK_OUT 0
#define ATH11K_MHI_CH_LOOPBACK_IN  1
#define ATH11K_MHI_CH_IPCR_OUT     20
#define ATH11K_MHI_CH_IPCR_IN      21
#define ATH11K_MHI_NUM_CHANNELS    4
/* Dos anillos de evento: 0 de control, 1 de datos (ath11k_mhi_events_*). */
#define ATH11K_MHI_NUM_EVENT_RINGS 2
#define ATH11K_MHI_MAX_CHANNELS    128
/* Elementos por anillo: los canales de ath11k declaran 32. */
#define ATH11K_MHI_CH_ELEMENTS     32
#define ATH11K_MHI_EV_ELEMENTS     32
#define ATH11K_MHI_CMD_ELEMENTS    32

struct ath11k_mhi_chan_cfg {
    uint8_t num;
    const char *name;
    uint8_t event_ring;
    /* 1 = host→dispositivo (DMA_TO_DEVICE), 0 = dispositivo→host. */
    uint8_t to_device;
};

/* Estado del transporte. */
struct ath11k_base {
    struct lx_pci_dev *pdev;
    volatile uint32_t *mmio;
    uint32_t mmio_len;
    uint16_t device_id;

    /* Ventanas anunciadas por el dispositivo. */
    uint32_t bhi_off;
    uint32_t bhie_off;
    uint32_t chdb_off;
    uint32_t erdb_off;

    enum mhi_ee ee;
    enum mhi_state state;

    /* Contextos DMA. */
    struct mhi_chan_ctxt *chan_ctxt;
    uint64_t chan_ctxt_dma;
    struct mhi_event_ctxt *er_ctxt;
    uint64_t er_ctxt_dma;
    struct mhi_cmd_ctxt *cmd_ctxt;
    uint64_t cmd_ctxt_dma;

    /* Anillos. */
    struct mhi_tre *cmd_ring;
    uint64_t cmd_ring_dma;
    struct mhi_tre *ev_ring[ATH11K_MHI_NUM_EVENT_RINGS];
    uint64_t ev_ring_dma[ATH11K_MHI_NUM_EVENT_RINGS];

    /* Imagen de firmware en vuelo (BHIe). */
    struct bhi_vec_entry *fw_vec;
    uint64_t fw_vec_dma;
    uint32_t fw_vec_entries;
    void *fw_buf;
    uint64_t fw_buf_dma;
    unsigned long fw_len;

    /* Última fase alcanzada, para el informe de hardware. */
    const char *phase;
};

/* ---- MMIO: la costura que el hostcheck sustituye ---- */
uint32_t ath11k_read32(struct ath11k_base *ab, uint32_t off);
void ath11k_write32(struct ath11k_base *ab, uint32_t off, uint32_t val);

/* ---- Transporte MHI ---- */
const char *ath11k_ee_str(enum mhi_ee ee);
const char *ath11k_state_str(enum mhi_state st);
enum mhi_ee ath11k_mhi_get_ee(struct ath11k_base *ab);
enum mhi_state ath11k_mhi_get_state(struct ath11k_base *ab);
int ath11k_mhi_poll_field(struct ath11k_base *ab, uint32_t off, uint32_t mask,
                          uint32_t shift, uint32_t val, unsigned int intentos);
int ath11k_mhi_reset(struct ath11k_base *ab);
int ath11k_mhi_wait_ready(struct ath11k_base *ab);
int ath11k_mhi_alloc_ctxt(struct ath11k_base *ab);
void ath11k_mhi_free_ctxt(struct ath11k_base *ab);
int ath11k_mhi_init_mmio(struct ath11k_base *ab);
int ath11k_mhi_fw_download(struct ath11k_base *ab, const void *fw,
                           unsigned long len);
int ath11k_mhi_set_state(struct ath11k_base *ab, enum mhi_state st);
int ath11k_mhi_power_up(struct ath11k_base *ab, const void *fw,
                        unsigned long fw_len);

extern const struct ath11k_mhi_chan_cfg ath11k_mhi_channels[ATH11K_MHI_NUM_CHANNELS];

#endif /* ATH11K_INTERNAL_H */
