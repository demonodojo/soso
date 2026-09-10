#ifndef IWL_INTERNAL_H
#define IWL_INTERNAL_H

#include <stdint.h>
#include <stddef.h>

#define IWL_AX211_SSID_MAX 32
#define IWL_AX211_MAX_SCAN 32

/* CSR (Gen2) */
#define CSR_BASE                     0x000
#define CSR_HW_IF_CONFIG_REG         (CSR_BASE + 0x000)
#define CSR_INT                      (CSR_BASE + 0x008)
#define CSR_INT_MASK                 (CSR_BASE + 0x00c)
#define CSR_INT_COALESCING           (CSR_BASE + 0x004)
#define CSR_RESET                    (CSR_BASE + 0x020)
#define CSR_GP_CNTRL                 (CSR_BASE + 0x024)
#define CSR_HW_REV                   (CSR_BASE + 0x028)
#define CSR_GPIO_IN                  (CSR_BASE + 0x018)
#define CSR_GIO_REG                  (CSR_BASE + 0x03C)
#define CSR_UCODE_DRV_GP1_CLR        (CSR_BASE + 0x05c)
#define CSR_MAC_SHADOW_REG_CTRL      (CSR_BASE + 0x0A8)
/* Linux cfg/22000.c: mac_addr_from_csr=0x380; OTP/STRAP son +0/+4/+8/+0xc. */
#define CSR_MAC_ADDR_FROM_CSR_22000  0x380
#define CSR_MAC_ADDR0_OTP(base)      ((uint32_t)(base) + 0x00u)
#define CSR_MAC_ADDR1_OTP(base)      ((uint32_t)(base) + 0x04u)
#define CSR_MAC_ADDR0_STRAP(base)    ((uint32_t)(base) + 0x08u)
#define CSR_MAC_ADDR1_STRAP(base)    ((uint32_t)(base) + 0x0cu)
#define CSR_LTR_LONG_VAL_AD          (CSR_BASE + 0x0D4)
#define CSR_GIO_CHICKEN_BITS         (CSR_BASE + 0x100)
#define CSR_DBG_HPET_MEM_REG         (CSR_BASE + 0x240)
#define CSR_CTXT_INFO_BOOT_CTRL      0x0
#define CSR_CTXT_INFO_ADDR           0x118
#define CSR_CTXT_INFO_BA             0x40
#define CSR_IML_DATA_ADDR            0x120
#define CSR_IML_SIZE_ADDR            0x128
#define HBUS_TARG_WRPTR              0x460
/* iwl-csr.h: HBUS_BASE=0x400 → WADDR +0x044, RADDR +0x048, WDAT +0x04c, RDAT +0x050. */
#define HBUS_TARG_PRPH_WADDR         0x444
#define HBUS_TARG_PRPH_RADDR         0x448
#define HBUS_TARG_PRPH_WDATA         0x44c
#define HBUS_TARG_PRPH_RDAT          0x450
/* iwl-prph.h: familia 22000 — no confundir con offsets legacy (0xd03c). */
#define UREG_CPU_INIT_RUN            0xa05c44
#define HPM_DEBUG                    0xa03440
#define PREG_PRPH_WPROT_22000        0xa04d00
/* Linux `iwl_trans_pcie_prph_msk` + `iwl_so_trans_cfg.umac_prph_offset`. */
#define IWL_PRPH_MSK_GEN2            0x000fffffu
#define IWL_PRPH_MSK_GEN3            0x00ffffffu
#define IWL_UMAC_PRPH_OFFSET         0x300000u
#define RFH_Q0_FRBDCB_WIDX_TRG       0x1C80
#define IWL_PCI_AX200                0x2723u
#define IWL_MVM_DQA_CMD_QUEUE        0
#define DQA_ENABLE_CMD               0x0
#define IWL_MVM_HCMD_TIMEOUT_MS      2000
#define IWL_GEN2_RX_N                32
#define IWL_GEN2_RX_SZ               4096
#define IWL_TFH_TFD_SIZE             256
#define IWL_TFH_NUM_TBS              25
#define IWL_CTXT_INFO_TFD_FORMAT_LONG 0x0100u
#define IWL_CTXT_INFO_RB_CB_SIZE_32   0x0050u
#define IWL_CTXT_INFO_RB_SIZE_4K      0x0800u
#define TFD_QUEUE_CB_SIZE_32          2

#define CSR_RESET_REG_FLAG_SW_RESET       (1u << 7)
#define CSR_GP_CNTRL_REG_FLAG_MAC_CLOCK_READY (1u << 0)
#define CSR_GP_CNTRL_REG_FLAG_GOING_TO_SLEEP  (1u << 4)
#define CSR_GP_CNTRL_REG_FLAG_INIT_DONE   (1u << 2)
#define CSR_GP_CNTRL_REG_FLAG_MAC_ACCESS_REQ (1u << 3)
#define CSR_GP_CNTRL_REG_VAL_MAC_ACCESS_EN    (1u << 0)
#define CSR_UCODE_SW_BIT_RFKILL               (1u << 1)
#define CSR_UCODE_DRV_GP1_BIT_CMD_BLOCKED     (1u << 2)
#define IWL_HOST_INT_TIMEOUT_DEF              0x40u
#define PERSISTENCE_BIT                       (1u << 12)
#define PREG_WFPM_ACCESS                      (1u << 12)
#define IWL_PRPH_HW_TIMEOUT                   0x5a5a5a5au
#define CSR_HW_IF_CONFIG_REG_BIT_HAP_WAKE_L1A (1u << 19)
#define CSR_HW_IF_CONFIG_REG_BIT_NIC_READY    (1u << 22)
#define CSR_GIO_REG_VAL_L0S_DISABLED          (1u << 1)
#define CSR_GIO_CHICKEN_BITS_REG_BIT_L1A_NO_L0S_RX (0x00800000u)
#define CSR_DBG_HPET_MEM_REG_VAL              (0xFFFF0000u)
#define CSR_INT_BIT_ALIVE                     (1u << 0)
#define CSR_INT_BIT_RF_KILL                   (1u << 7)
#define CSR_INT_BIT_SW_ERR                    (1u << 25)
#define CSR_INT_BIT_FH_RX                     (1u << 31)
/* iwl-csr.h: bit 27 = estado del switch RF-kill (1 = radio ON). El 9 es SYS_CONFIG. */
#define CSR_GP_CNTRL_REG_FLAG_HW_RF_KILL_SW   (1u << 27)
#define APMG_CLK_EN_REG                       0x3004u
#define APMG_CLK_VAL_DMA_CLK_RQT              0x200u
#define CSR_LTR_LONG_VAL_AD_NO_SNOOP_REQ      0x80000000u
#define CSR_LTR_LONG_VAL_AD_NO_SNOOP_SCALE    0x1c000000u
#define CSR_LTR_LONG_VAL_AD_NO_SNOOP_VAL      0x03ff0000u
#define CSR_LTR_LONG_VAL_AD_SNOOP_REQ         0x00008000u
#define CSR_LTR_LONG_VAL_AD_SNOOP_SCALE       0x00001c00u
#define CSR_LTR_LONG_VAL_AD_SNOOP_VAL         0x000003ffu
#define CSR_LTR_LONG_VAL_AD_SCALE_USEC        2u
#define CSR_AUTO_FUNC_BOOT_ENA            (1u << 1)
#define CSR_AUTO_FUNC_INIT                (1u << 7)
#define IWL_PRPH_INFO_ALLOC               4096u
#define RX_QUEUE_CB_SIZE_32               5
#define IWL_PRPH_SCRATCH_MTR_MODE         (1u << 17)
#define IWL_PRPH_MTR_FORMAT_256B          0xC0000u

#define IWL_TLV_UCODE_MAGIC        0x0a4c5749u
#define IWL_UCODE_TLV_INST         1
#define IWL_UCODE_TLV_DATA         2
#define IWL_UCODE_TLV_INIT         3
#define IWL_UCODE_TLV_INIT_DATA    4
#define IWL_UCODE_TLV_BOOT         5
#define IWL_UCODE_TLV_SEC_RT       19
#define IWL_UCODE_TLV_SEC_INIT     20
#define IWL_UCODE_TLV_SEC_WOWLAN   21
#define IWL_UCODE_TLV_PAGING       32
#define IWL_UCODE_TLV_IML          52
#define IWL_UCODE_TLV_HW_TYPE      58
#define IWL_UCODE_TLV_PNVM_SKU     64

#define IWL_FW_RT_MAX              128
#define IWL_CPU1_CPU2_SEPARATOR    0xFFFFCCCCu
#define IWL_PAGING_SEPARATOR       0xAAAABBBBu

#define IWL_MAX_DRAM_ENTRY         64
#define IWL_CMD_QUEUE_SIZE         32

/* Estado de propiedad de un slot de la cola de comandos (R4). */
#define IWL_SLOT_FREE              0
#define IWL_SLOT_SYNC              1
#define IWL_SLOT_ASYNC             2
/* Envenenado: expiró la espera y el FW aún puede escribir su DMA. */
#define IWL_SLOT_POISON            3
/* Respondido: liberable, pero solo cuando le toque por orden de cola. */
#define IWL_SLOT_DONE              4
#define IWL_CMD_SLOT_SIZE          4096
#define IWL_MTR_SIZE               256
#define IWL_MCR_SIZE               IWL_CMD_SLOT_SIZE
#define IWL_RX_QUEUE_SIZE          256
#define IWL_TX_QUEUE_DATA          8

#define UCODE_ALIVE_NTFY           0x1
#define INIT_COMPLETE_NOTIF        0x4
#define REPLY_RX_PHY_CMD           0xc0
#define REPLY_RX_MPDU_CMD          0xc1
#define SCAN_CFG_CMD               0xc
#define SCAN_REQ_UMAC              0xd
#define SCAN_COMPLETE_UMAC         0x0f
#define OFFLOAD_MATCH_INFO_NOTIF   0xfc

static inline uint32_t iwl_cpu_to_le32(uint32_t v)
{
    return v;
}

static inline uint16_t iwl_cpu_to_le16(uint16_t v)
{
    return v;
}

#define LEGACY_GROUP               0x0
#define LONG_GROUP                 0x1
#define SCAN_GROUP                 0x6
#define DATA_PATH_GROUP            0x5
#define MAC_CONF_GROUP             0x3
#define SYSTEM_GROUP               0x2
#define REGULATORY_AND_NVM_GROUP   0xc

#define INIT_EXTENDED_CFG_CMD      0x03
#define NVM_ACCESS_CMD             0x88 /* LEGACY_GROUP — no usar en init unificado */
#define NVM_ACCESS_COMPLETE        0x00
#define NVM_GET_INFO               0x02
#define PHY_CONFIGURATION_CMD      0x0b
#define TX_ANT_CONFIGURATION_CMD   0x98 /* LEGACY_GROUP */
#define MCC_UPDATE_CMD             0xc8 /* LEGACY_GROUP */

#define FW_PHY_CFG_TX_CHAIN_POS    16
#define FW_PHY_CFG_TX_CHAIN        (0xfu << FW_PHY_CFG_TX_CHAIN_POS)
#define FW_PHY_CFG_RX_CHAIN_POS    20
#define FW_PHY_CFG_RX_CHAIN        (0xfu << FW_PHY_CFG_RX_CHAIN_POS)

#define IWL_NVM_READ               0u
#define IWL_NVM_WRITE              1u
#define NVM_ACCESS_TARGET_CACHE    0u
#define READ_NVM_CHUNK_SUCCEED     0u
#define IWL_NVM_SECTION_TYPE_HW    1u /* NVM_SECTION_TYPE_SW en Linux */
#define NVM_MAC_ADDR_OFFSET        0x64u
#define NVM_CHANNEL_VALID          (1u << 0)
#define NVM_CHANNEL_IBSS           (1u << 1)
#define NVM_CHANNEL_ACTIVE         (1u << 3)
#define NVM_CHANNEL_RADAR          (1u << 4)
#define NVM_CHANNEL_INDOOR_ONLY    (1u << 5)
#define NVM_CHANNEL_GO_CONCURRENT  (1u << 6)

/* Origen de la lista de canales de scan (iwl_mvm_collect_scan_channels). */
#define IWL_CHAN_SRC_NONE          0 /* sin perfil: no se puede escanear */
#define IWL_CHAN_SRC_FALLBACK      1 /* NVM ausente: lista mínima conocida */
#define IWL_CHAN_SRC_NVM           2 /* perfil NVM filtrado */
#define IWL_CHAN_SRC_MCC           3 /* perfil regulatorio MCC aplicado */
#define IWL_CHAN_SRC_EMPTY         4 /* perfil válido, cero canales usables */

#define IWL_UCODE_TLV_PHY_SKU                  23
#define IWL_UCODE_TLV_ENABLED_CAPABILITIES       30
#define IWL_UCODE_TLV_N_SCAN                     31
#define IWL_UCODE_TLV_CMD_VERSIONS               48

/* Linux `enum iwl_ucode_tlv_capa` — bit 12 = DQA_SUPPORT (file.h). */
#define IWL_UCODE_TLV_CAPA_DQA_SUPPORT           12
#define IWL_FW_CAPA_SETS                         4

struct iwl_ucode_capa {
    uint32_t api_index;
    uint32_t api_capa;
} __attribute__((packed));

#define IWL_INIT_NVM               1

#define QUEUE_TO_SEQ(q)            (((uint16_t)(q) & 0x1fu) << 8)
#define INDEX_TO_SEQ(i)            ((uint16_t)(i) & 0xffu)
#define SEQ_RX_FRAME               0x8000u
#define SEQ_TO_QUEUE(s)            (((s) >> 8) & 0x1fu)
#define SEQ_TO_INDEX(s)            ((s) & 0xffu)

#define SCAN_MAX_NUM_CHANS_V3      67
#define SCAN_TWO_LMACS             2
#define IWL_MAX_SCHED_SCAN_PLANS   10
#define PROBE_OPTION_MAX           20
#define SCAN_SHORT_SSID_MAX_SIZE   20
#define SCAN_BSSID_MAX_SIZE        32
#define IWL_RX_DESC_SIZE_V1        48u

#define ADD_STA                    0x18
#define TX_CMD                     0x1

#define IWL_PRPH_SCRATCH_RB_SIZE_4K (1u << 16)

/* Un BSS del scan. `open` sale de las capacidades del beacon y del RSN IE, no
 * de una suposición: antes se marcaba abierto TODO lo que se veía (R7).
 *
 * El espejo en Rust es `LxWifiBss` (kernel/src/lxdde/wifi.rs) y se copia con
 * memcpy, así que los dos han de tener el mismo layout. */
struct iwl_ax211_bss {
    char ssid[IWL_AX211_SSID_MAX + 1];
    uint8_t bssid[6];
    int8_t rssi;
    uint8_t channel;
    uint8_t open;
    /* RSN IE (id 48) presente. */
    uint8_t rsn;
    /* AKM 00-0F-AC:2 (PSK) o :6 (PSK-SHA256) entre los ofrecidos. */
    uint8_t akm_psk;
    /* Cifrado por pares y de grupo = CCMP-128 (00-0F-AC:4). */
    uint8_t ccmp;
    uint8_t band24;
};

/* El espejo en Rust se copia con memcpy: si esto cambia de tamaño hay que
 * cambiar `LxWifiBss` a la vez, y así el compilador lo recuerda. */
typedef char iwl_ax211_bss_size_check[sizeof(struct iwl_ax211_bss) == 46 ? 1 : -1];

/* Elementos y bits de 802.11 que hacen falta para clasificar un BSS. */
#define WLAN_EID_SSID              0
#define WLAN_EID_DS_PARAMS         3
#define WLAN_EID_RSN               48
#define WLAN_CAPABILITY_PRIVACY    (1u << 4)
#define WLAN_AKM_PSK               2u
#define WLAN_AKM_PSK_SHA256        6u
#define WLAN_CIPHER_CCMP128        4u

struct iwl_ucode_tlv {
    uint32_t type;
    uint32_t length;
    uint8_t data[];
} __attribute__((packed));

struct iwl_tlv_ucode_header {
    uint32_t zero;
    uint32_t magic;
    uint8_t human[64];
    uint32_t ver;
    uint32_t build;
    uint64_t ignore;
    uint8_t data[];
} __attribute__((packed));

struct iwl_fw_section {
    const uint8_t *data;
    uint32_t len;
};

struct iwl_fw_rt_section {
    const uint8_t *data;
    uint32_t len;
};

struct iwl_fw_image {
    struct iwl_fw_section inst;
    struct iwl_fw_section data;
    struct iwl_fw_section init;
    struct iwl_fw_section init_data;
    struct iwl_fw_section boot;
    struct iwl_fw_rt_section rt[IWL_FW_RT_MAX];
    int rt_n;
};

struct iwl_prph_scratch_version {
    uint16_t mac_id;
    uint16_t version;
    uint16_t size;
    uint16_t reserved;
} __attribute__((packed));

struct iwl_prph_scratch_control {
    uint32_t control_flags;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_prph_scratch_pnvm_cfg {
    uint64_t pnvm_base_addr;
    uint32_t pnvm_size;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_prph_scratch_hwm_cfg {
    uint64_t hwm_base_addr;
    uint32_t hwm_size;
    uint32_t debug_token_config;
} __attribute__((packed));

struct iwl_prph_scratch_rbd_cfg {
    uint64_t free_rbd_addr;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_prph_scratch_uefi_cfg {
    uint64_t base_addr;
    uint32_t size;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_prph_scratch_step_cfg {
    uint32_t mbx_addr_0;
    uint32_t mbx_addr_1;
} __attribute__((packed));

struct iwl_prph_scratch_ctrl_cfg {
    struct iwl_prph_scratch_version version;
    struct iwl_prph_scratch_control control;
    struct iwl_prph_scratch_pnvm_cfg pnvm_cfg;
    struct iwl_prph_scratch_hwm_cfg hwm_cfg;
    struct iwl_prph_scratch_rbd_cfg rbd_cfg;
    struct iwl_prph_scratch_uefi_cfg reduce_power_cfg;
    struct iwl_prph_scratch_step_cfg step_cfg;
} __attribute__((packed));

struct iwl_context_info_dram {
    uint64_t umac_img[IWL_MAX_DRAM_ENTRY];
    uint64_t lmac_img[IWL_MAX_DRAM_ENTRY];
    uint64_t virtual_img[IWL_MAX_DRAM_ENTRY];
} __attribute__((packed));

struct iwl_prph_scratch {
    struct iwl_prph_scratch_ctrl_cfg ctrl_cfg;
    uint32_t reserved[10];
    struct iwl_context_info_dram dram;
} __attribute__((packed));

struct iwl_prph_info {
    uint32_t boot_stage_mirror;
    uint32_t ipc_status_mirror;
    uint32_t sleep_notif;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_context_info_gen3 {
    uint16_t version;
    uint16_t size;
    uint32_t config;
    uint64_t prph_info_base_addr;
    uint64_t cr_head_idx_arr_base_addr;
    uint64_t tr_tail_idx_arr_base_addr;
    uint64_t cr_tail_idx_arr_base_addr;
    uint64_t tr_head_idx_arr_base_addr;
    uint16_t cr_idx_arr_size;
    uint16_t tr_idx_arr_size;
    uint64_t mtr_base_addr;
    uint64_t mcr_base_addr;
    uint16_t mtr_size;
    uint16_t mcr_size;
    uint16_t mtr_doorbell_vec;
    uint16_t mcr_doorbell_vec;
    uint16_t mtr_msi_vec;
    uint16_t mcr_msi_vec;
    uint8_t mtr_opt_header_size;
    uint8_t mtr_opt_footer_size;
    uint8_t mcr_opt_header_size;
    uint8_t mcr_opt_footer_size;
    uint16_t msg_rings_ctrl_flags;
    uint16_t prph_info_msi_vec;
    uint64_t prph_scratch_base_addr;
    uint32_t prph_scratch_size;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_tfh_tfd {
    uint64_t addr;
    uint16_t num_bytes;
    uint8_t reserved[6];
} __attribute__((packed));

struct iwl_tfh_tb_long {
    uint16_t tb_len;
    uint64_t addr;
} __attribute__((packed));

struct iwl_tfh_tfd_long {
    uint16_t num_tbs;
    struct iwl_tfh_tb_long tbs[IWL_TFH_NUM_TBS];
    uint32_t pad;
} __attribute__((packed));

struct iwl_context_info_version {
    uint16_t mac_id;
    uint16_t version;
    uint16_t size;
    uint16_t reserved;
} __attribute__((packed));

struct iwl_context_info_control {
    uint32_t control_flags;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_context_info_rbd_cfg {
    uint64_t free_rbd_addr;
    uint64_t used_rbd_addr;
    uint64_t status_wr_ptr;
} __attribute__((packed));

struct iwl_context_info_hcmd_cfg {
    uint64_t cmd_queue_addr;
    uint8_t cmd_queue_size;
    uint8_t reserved[7];
} __attribute__((packed));

struct iwl_context_info_dump_cfg {
    uint64_t core_dump_addr;
    uint32_t core_dump_size;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_context_info_early_dbg_cfg {
    uint64_t early_debug_addr;
    uint32_t early_debug_size;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_context_info_pnvm_cfg {
    uint64_t platform_nvm_addr;
    uint32_t platform_nvm_size;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_context_info {
    struct iwl_context_info_version version;
    struct iwl_context_info_control control;
    uint64_t reserved0;
    struct iwl_context_info_rbd_cfg rbd_cfg;
    struct iwl_context_info_hcmd_cfg hcmd_cfg;
    uint32_t reserved1[4];
    struct iwl_context_info_dump_cfg dump_cfg;
    struct iwl_context_info_early_dbg_cfg edbg_cfg;
    struct iwl_context_info_pnvm_cfg pnvm_cfg;
    uint32_t reserved2[16];
    struct iwl_context_info_dram dram;
    uint32_t reserved3[16];
} __attribute__((packed));

struct iwl_rx_packet {
    uint16_t len_n_flags;
    uint8_t group_id;
    uint8_t id;
    uint8_t data[];
} __attribute__((packed));

struct iwl_cmd_header {
    uint8_t cmd;
    uint8_t group_id;
    uint16_t sequence;
    uint8_t reserved;
    uint8_t length;
} __attribute__((packed));

struct iwl_cmd_header_wide {
    uint8_t cmd;
    uint8_t group_id;
    uint16_t sequence;
    uint16_t length;
    uint8_t reserved;
    uint8_t version;
} __attribute__((packed));

struct iwl_scan_config {
    uint8_t enable_cam_mode;
    uint8_t enable_promiscouos_mode;
    uint8_t bcast_sta_id;
    uint8_t reserved;
    uint32_t tx_chains;
    uint32_t rx_chains;
} __attribute__((packed));

struct iwl_scan_umac_schedule {
    uint16_t interval;
    uint8_t iter_count;
    uint8_t reserved;
} __attribute__((packed));

struct iwl_scan_probe_segment {
    uint16_t offset;
    uint16_t len;
} __attribute__((packed));

struct iwl_scan_probe_req_v1 {
    struct iwl_scan_probe_segment mac_header;
    struct iwl_scan_probe_segment band_data[2];
    struct iwl_scan_probe_segment common_data;
    uint8_t buf[512];
} __attribute__((packed));

struct iwl_ssid_ie {
    uint8_t id;
    uint8_t len;
    uint8_t ssid[32];
} __attribute__((packed));

struct iwl_scan_umac_chan_param {
    uint8_t flags;
    uint8_t count;
    uint16_t reserved;
} __attribute__((packed));

struct iwl_scan_channel_cfg_umac {
    uint32_t flags;
    union {
        struct {
            uint8_t channel_num;
            uint8_t band;
            uint8_t iter_count;
            uint8_t iter_interval;
        } v2;
    };
} __attribute__((packed));

struct iwl_scan_req_umac_tail_v1 {
    struct iwl_scan_umac_schedule schedule[2];
    uint16_t delay;
    uint16_t reserved;
    struct iwl_scan_probe_req_v1 preq;
    struct iwl_ssid_ie direct_scan[20];
} __attribute__((packed));

struct iwl_umac_scan_complete {
    uint32_t uid;
    uint8_t last_schedule;
    uint8_t last_iter;
    uint8_t status;
    uint8_t ebs_status;
    uint32_t time_from_last_iter;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_rx_mpdu_res_start {
    uint16_t byte_count;
    uint16_t assist;
} __attribute__((packed));

struct iwl_init_extended_cfg_cmd {
    uint32_t init_flags;
} __attribute__((packed));

/* fw/api/nvm-reg.h NVM_ACCESS_CMD_API_S_VER_2 */
struct iwl_nvm_access_cmd {
    uint8_t op_code;
    uint8_t target;
    uint16_t type;
    uint16_t offset;
    uint16_t length;
} __attribute__((packed));

struct iwl_nvm_access_resp {
    uint16_t offset;
    uint16_t length;
    uint16_t type;
    uint16_t status;
    uint8_t data[];
} __attribute__((packed));

struct iwl_nvm_access_complete_cmd {
    uint32_t reserved;
} __attribute__((packed));

struct iwl_nvm_get_info {
    uint32_t reserved;
} __attribute__((packed));

struct iwl_nvm_get_info_general {
    uint32_t flags;
    uint16_t nvm_version;
    uint8_t board_type;
    uint8_t n_hw_addrs;
} __attribute__((packed));

struct iwl_nvm_get_info_sku {
    uint32_t mac_sku_flags;
} __attribute__((packed));

struct iwl_nvm_get_info_phy {
    uint32_t tx_chains;
    uint32_t rx_chains;
} __attribute__((packed));

#define IWL_NUM_CHANNELS_V1        51
#define IWL_NUM_CHANNELS           110

struct iwl_nvm_get_info_regulatory_v1 {
    uint32_t lar_enabled;
    uint16_t channel_profile[IWL_NUM_CHANNELS_V1];
    uint16_t reserved;
} __attribute__((packed));

struct iwl_nvm_get_info_regulatory {
    uint32_t lar_enabled;
    uint32_t n_channels;
    uint32_t channel_profile[IWL_NUM_CHANNELS];
} __attribute__((packed));

struct iwl_nvm_get_info_rsp_v3 {
    struct iwl_nvm_get_info_general general;
    struct iwl_nvm_get_info_sku mac_sku;
    struct iwl_nvm_get_info_phy phy_sku;
    struct iwl_nvm_get_info_regulatory_v1 regulatory;
} __attribute__((packed));

struct iwl_nvm_get_info_rsp {
    struct iwl_nvm_get_info_general general;
    struct iwl_nvm_get_info_sku mac_sku;
    struct iwl_nvm_get_info_phy phy_sku;
    struct iwl_nvm_get_info_regulatory regulatory;
} __attribute__((packed));

struct iwl_tx_ant_cfg_cmd {
    uint32_t valid;
} __attribute__((packed));

struct iwl_dqa_enable_cmd {
    uint32_t cmd_queue;
} __attribute__((packed));

#define PHY_CONTEXT_CMD            0x08

struct iwl_mcc_update_cmd {
    uint16_t mcc;
    uint8_t source_id;
    uint8_t reserved;
    uint32_t key;
    uint8_t reserved2[20];
} __attribute__((packed));

/* Layouts completos de `nvm-reg.h`: la lista de canales es variable y su
 * tamaño es parte de la validación (payload == struct + 4*n_channels). */
struct iwl_mcc_update_resp_v8 {
    uint32_t status;
    uint16_t mcc;
    uint8_t padding[2];
    uint32_t cap;
    uint16_t time;
    uint16_t geo_info;
    uint8_t source_id;
    uint8_t reserved[3];
    uint32_t n_channels;
    /* uint32_t channels[n_channels]; */
} __attribute__((packed));

struct iwl_mcc_update_resp_v4 {
    uint32_t status;
    uint16_t mcc;
    uint16_t cap;
    uint16_t time;
    uint16_t geo_info;
    uint8_t source_id;
    uint8_t reserved[3];
    uint32_t n_channels;
} __attribute__((packed));

struct iwl_mcc_update_resp_v3 {
    uint32_t status;
    uint16_t mcc;
    uint8_t cap;
    uint8_t source_id;
    uint16_t time;
    uint16_t geo_info;
    uint32_t n_channels;
} __attribute__((packed));

#define MCC_SOURCE_OLD_FW       0
#define MCC_SOURCE_GET_CURRENT  0x10
#define MCC_RESP_NEW_CHAN_PROFILE 0
#define MCC_RESP_SAME_CHAN_PROFILE 1
#define MCC_RESP_INVALID          2
#define MCC_RESP_NVM_DISABLED     3
#define MCC_RESP_ILLEGAL          4
#define MCC_RESP_LOW_PRIORITY     5

struct iwl_scan_config_v2 {
    uint8_t enable_cam_mode;
    uint8_t enable_promiscouos_mode;
    uint8_t bcast_sta_id;
    uint8_t reserved;
    uint32_t tx_chains;
    uint32_t rx_chains;
    uint32_t reserved2;
    uint32_t flags;
    uint32_t reserved3[2];
} __attribute__((packed));

struct iwl_calib_ctrl {
    uint32_t ucode_control;
    uint32_t flow_trigger;
    uint32_t flow_block;
} __attribute__((packed));

struct iwl_phy_cfg_cmd_v1 {
    uint32_t phy_cfg;
    struct iwl_calib_ctrl calib_control;
} __attribute__((packed));

/* `struct iwl_fw_cmd_version` (fw/api/cmdhdr.h): el cuarto byte es la versión
 * de *notificación*, no relleno. MCC_UPDATE elige layout de respuesta con él. */
struct iwl_fw_cmd_version {
    uint8_t cmd;
    uint8_t group;
    uint8_t version;
    uint8_t notif_version;
} __attribute__((packed));

struct iwl_scan_general_params_v11 {
    uint16_t flags;
    uint8_t reserved;
    uint8_t scan_start_mac_or_link_id;
    uint8_t active_dwell[SCAN_TWO_LMACS];
    uint8_t adwell_default_2g;
    uint8_t adwell_default_5g;
    uint8_t adwell_default_social_chn;
    uint8_t flags2;
    uint16_t adwell_max_budget;
    uint32_t max_out_of_time[SCAN_TWO_LMACS];
    uint32_t suspend_time[SCAN_TWO_LMACS];
    uint32_t scan_priority;
    uint8_t passive_dwell[SCAN_TWO_LMACS];
    uint8_t num_of_fragments[SCAN_TWO_LMACS];
} __attribute__((packed));

struct iwl_scan_channel_params_v7 {
    uint8_t flags;
    uint8_t count;
    uint8_t n_aps_override[2];
    struct iwl_scan_channel_cfg_umac channel_config[SCAN_MAX_NUM_CHANS_V3];
} __attribute__((packed));

struct iwl_scan_periodic_parms_v1 {
    struct iwl_scan_umac_schedule schedule[IWL_MAX_SCHED_SCAN_PLANS];
    uint16_t delay;
    uint16_t reserved;
} __attribute__((packed));

struct iwl_scan_probe_req {
    struct iwl_scan_probe_segment mac_header;
    struct iwl_scan_probe_segment band_data[2];
    struct iwl_scan_probe_segment common_data;
    uint8_t buf[512];
} __attribute__((packed));

struct iwl_scan_probe_params_v4 {
    struct iwl_scan_probe_req preq;
    uint8_t short_ssid_num;
    uint8_t bssid_num;
    uint16_t reserved;
    struct iwl_ssid_ie direct_scan[PROBE_OPTION_MAX];
    uint32_t short_ssid[SCAN_SHORT_SSID_MAX_SIZE];
    uint8_t bssid_array[SCAN_BSSID_MAX_SIZE][6];
} __attribute__((packed));

struct iwl_scan_req_params_v17 {
    struct iwl_scan_general_params_v11 general_params;
    struct iwl_scan_channel_params_v7 channel_params;
    struct iwl_scan_periodic_parms_v1 periodic_params;
    struct iwl_scan_probe_params_v4 probe_params;
} __attribute__((packed));

struct iwl_scan_req_umac_v17 {
    uint32_t uid;
    uint32_t ooc_priority;
    struct iwl_scan_req_params_v17 scan_params;
} __attribute__((packed));

/* Tamaño fijo: channel_config[67] no se recorta; count solo enumera válidos. */
static inline unsigned iwl_scan_req_umac_v17_size(unsigned n_channels)
{
    (void)n_channels;
    return (unsigned)sizeof(struct iwl_scan_req_umac_v17);
}

/* Flags por canal (`enum iwl_uhb_chan_cfg_flags`, fw/api/scan.h): los bits
 * bajos son el mapa de SSID directos, NO «pasivo». */
#define IWL_UHB_CHAN_CFG_FLAG_FORCE_PASSIVE (1u << 26)
/* v17: la banda viaja en flags[31:30]; `v2.band` pasa a ser `v5.psd_20`. */
#define IWL_CHAN_CFG_FLAGS_BAND_POS 30

#define IWL_SCAN_REQ_UMAC_SIZE_V6 44u
#define IWL_UMAC_SCAN_GEN_FLAGS_PASS_ALL  (1u << 2)
#define IWL_UMAC_SCAN_GEN_FLAGS_PASSIVE   (1u << 3)
#define IWL_UMAC_SCAN_GEN_FLAGS_ITER_COMPLETE (1u << 5)
#define IWL_UMAC_SCAN_GEN_FLAGS_V2_PERIODIC (1u << 0)
#define IWL_UMAC_SCAN_GEN_FLAGS_V2_PASS_ALL (1u << 1)
#define IWL_UMAC_SCAN_GEN_FLAGS_V2_NTFY_ITER_COMPLETE (1u << 2)
#define IWL_UMAC_SCAN_GEN_FLAGS_V2_MATCH (1u << 5)
#define IWL_UMAC_SCAN_GEN_FLAGS_V2_FORCE_PASSIVE (1u << 11)
#define IWL_SCAN_OFFLOAD_COMPLETED 1u
#define IWL_SCAN_OFFLOAD_ABORTED   2u
#define IWL_SCAN_END_NONE          0
#define IWL_SCAN_END_NORMAL        1
#define IWL_SCAN_END_ABORTED       2
#define IWL_SCAN_END_TIMEOUT       3

/* Resultado de iwl_mvm_scan: se distinguen para el usuario final. */
#define IWL_SCAN_RC_OK             0
#define IWL_SCAN_RC_ABORTED       (-1)
#define IWL_SCAN_RC_TIMEOUT       (-2)
#define IWL_SCAN_RC_NO_CHANNELS   (-3)
#define IWL_SCAN_RC_NO_REGDOM     (-4)
#define IWL_CMD_FAILED_MSK         0x40u

struct iwl_ax211_priv;

int iwl_fw_parse_tlv(struct iwl_ax211_priv *iwl, const uint8_t *fw, unsigned long fw_len);
int iwl_trans_gen2_start(struct iwl_ax211_priv *iwl);
int iwl_trans_gen3_start(struct iwl_ax211_priv *iwl);
void iwl_trans_poll(struct iwl_ax211_priv *iwl);
int iwl_trans_send_cmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                       const void *payload, uint16_t pay_len);
int iwl_trans_send_cmd_async(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                             const void *payload, uint16_t pay_len);
int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t pay_len, int wait_ms);
/* Slots libres en la cola de comandos (backpressure cuando llega a 0). */
unsigned iwl_trans_cmd_space(struct iwl_ax211_priv *iwl);
/* 1 si un timeout dejó la cola bloqueada y hace falta reiniciar transporte. */
int iwl_trans_needs_recover(struct iwl_ax211_priv *iwl);
/* Libera la cola y marca MVM abajo: el llamador debe repetir init/up. */
int iwl_trans_recover(struct iwl_ax211_priv *iwl);
int iwl_mvm_run_init(struct iwl_ax211_priv *iwl);
int iwl_mvm_up_minimal(struct iwl_ax211_priv *iwl);
uint8_t iwl_mvm_scan_rx_ant(struct iwl_ax211_priv *iwl);
int iwl_mvm_nvm_read_mac(struct iwl_ax211_priv *iwl);
int iwl_mvm_nvm_get_info_mac(struct iwl_ax211_priv *iwl);
int iwl_mvm_send_tx_ant_cfg(struct iwl_ax211_priv *iwl);
int iwl_mvm_send_scan_cfg(struct iwl_ax211_priv *iwl);
int iwl_mvm_init_mcc(struct iwl_ax211_priv *iwl);
/* Aplica una respuesta MCC ya recibida en cmd_resp[]; separada para poder
 * validarla en el banco host sin transporte. */
int iwl_mvm_apply_mcc_resp(struct iwl_ax211_priv *iwl, int notif_ver,
                           const uint8_t *resp, unsigned len);
/* Selección de canales de scan: devuelve el número de entradas rellenadas y
 * escribe en `origen` de dónde salen (NVM/MCC, fallback o vacío). */
unsigned iwl_mvm_collect_scan_channels(struct iwl_ax211_priv *iwl,
                                       uint8_t *ch, uint8_t *band,
                                       uint8_t *passive, unsigned max,
                                       int *origen);
int iwl_fw_notif_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd);
uint8_t iwl_mvm_valid_tx_ant(struct iwl_ax211_priv *iwl);
uint8_t iwl_mvm_valid_rx_ant(struct iwl_ax211_priv *iwl);
void iwl_mvm_fill_probe_req(struct iwl_ax211_priv *iwl, struct iwl_scan_probe_params_v4 *probe);
int iwl_mvm_assoc_prepare(struct iwl_ax211_priv *iwl, const char *ssid,
                          const uint8_t *bssid);
int iwl_mvm_scan(struct iwl_ax211_priv *iwl);
int iwl_mvm_scan_umac_supported(uint8_t ver);
uint16_t iwl_mvm_build_scan_req(struct iwl_ax211_priv *iwl, uint8_t *buf, unsigned cap);
void iwl_mvm_on_scan_complete(struct iwl_ax211_priv *iwl, uint32_t uid, uint8_t status);
uint32_t iwl_mac_addr_from_csr(uint16_t device_id);
int iwl_mac_valid_unicast(const uint8_t mac[6]);
void iwl_mac_from_csr(struct iwl_ax211_priv *iwl, uint8_t mac[6]);
void iwl_trans_rx_packet(struct iwl_ax211_priv *iwl, const uint8_t *buf, unsigned len);
int iwl_fw_cmd_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd);
int iwl_fw_has_capa(const struct iwl_ax211_priv *iwl, unsigned capa_bit);
void iwl_mvm_rx_scan_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len);
/* Clasifica un beacon/probe response: SSID, canal, capacidades y RSN.
 * Devuelve la longitud del SSID o -1 si la trama no sirve. */
int iwl_mvm_parse_bss(const uint8_t *frame, int len, struct iwl_ax211_bss *out);
/* El BSS con ese SSID exacto y mejor RSSI, o NULL. */
const struct iwl_ax211_bss *iwl_mvm_pick_bss(struct iwl_ax211_priv *iwl,
                                             const char *ssid);
int iwl_mvm_connect_open(struct iwl_ax211_priv *iwl, const char *ssid);
int iwl_mvm_connect_wpa2(struct iwl_ax211_priv *iwl, const char *ssid, const uint8_t psk[32]);
int iwl_mvm_install_key(struct iwl_ax211_priv *iwl, const uint8_t key[16], int key_idx);
int iwl_mvm_tx_8023(struct iwl_ax211_priv *iwl, const uint8_t *buf, int len);
int iwl_mvm_rx_8023(struct iwl_ax211_priv *iwl, uint8_t *buf, int buflen);

#endif
