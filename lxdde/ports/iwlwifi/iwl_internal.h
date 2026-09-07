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
#define CSR_RESET                    (CSR_BASE + 0x020)
#define CSR_GP_CNTRL                 (CSR_BASE + 0x024)
#define CSR_HW_REV                   (CSR_BASE + 0x028)
#define CSR_GPIO_IN                  (CSR_BASE + 0x018)
#define CSR_CTXT_INFO_BOOT_CTRL      0x0
#define CSR_CTXT_INFO_ADDR           0x118
#define CSR_CTXT_INFO_BA             0x40
#define CSR_IML_DATA_ADDR            0x120
#define CSR_IML_SIZE_ADDR            0x128
#define HBUS_TARG_WRPTR              0x460
#define RFH_Q0_FRBDCB_WIDX_TRG       0x1C80
#define IWL_PCI_AX200                0x2723u
#define IWL_MVM_DQA_CMD_QUEUE        9
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
#define CSR_GP_CNTRL_REG_FLAG_INIT_DONE   (1u << 2)
#define CSR_GP_CNTRL_REG_FLAG_MAC_ACCESS_REQ (1u << 3)
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
#define IWL_MTR_SIZE               256
#define IWL_MCR_SIZE               256
#define IWL_RX_QUEUE_SIZE          256
#define IWL_TX_QUEUE_DATA          8

#define UCODE_ALIVE_NTFY           0x1
#define INIT_COMPLETE_NOTIF        0x4
#define SCAN_COMPLETE_UMAC         0xFC

#define SCAN_GROUP                 0x6
#define DATA_PATH_GROUP            0x5
#define MAC_CONF_GROUP             0x3
#define SYSTEM_GROUP               0x2

#define SCAN_REQ_UMAC              0x0
#define ADD_STA                    0x18
#define TX_CMD                     0x1

#define IWL_PRPH_SCRATCH_RB_SIZE_4K (1u << 16)

struct iwl_ax211_bss {
    char ssid[IWL_AX211_SSID_MAX + 1];
    uint8_t bssid[6];
    int8_t rssi;
    uint8_t channel;
    uint8_t open;
};

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

struct iwl_ax211_priv;

int iwl_fw_parse_tlv(struct iwl_ax211_priv *iwl, const uint8_t *fw, unsigned long fw_len);
int iwl_trans_gen2_start(struct iwl_ax211_priv *iwl);
int iwl_trans_gen3_start(struct iwl_ax211_priv *iwl);
void iwl_trans_poll(struct iwl_ax211_priv *iwl);
int iwl_mvm_scan(struct iwl_ax211_priv *iwl);
int iwl_mvm_connect_open(struct iwl_ax211_priv *iwl, const char *ssid);
int iwl_mvm_connect_wpa2(struct iwl_ax211_priv *iwl, const char *ssid, const uint8_t psk[32]);
int iwl_mvm_install_key(struct iwl_ax211_priv *iwl, const uint8_t key[16], int key_idx);
int iwl_mvm_tx_8023(struct iwl_ax211_priv *iwl, const uint8_t *buf, int len);
int iwl_mvm_rx_8023(struct iwl_ax211_priv *iwl, uint8_t *buf, int buflen);

#endif
