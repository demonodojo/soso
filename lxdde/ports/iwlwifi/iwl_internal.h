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
#define CSR_HW_RF_ID                 (CSR_BASE + 0x09c)
#define CSR_HW_REV_TYPE(_val)        (((_val) & 0x000fff0u) >> 4)
#define CSR_HW_RFID_TYPE(_val)       (((_val) & 0x0fff000u) >> 12)
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
#define HBUS_TARG_MEM_RADDR          0x40c
#define HBUS_TARG_MEM_WADDR          0x410
#define HBUS_TARG_MEM_WDAT           0x418
#define HBUS_TARG_MEM_RDAT           0x41c
/* iwl-csr.h: HBUS_BASE=0x400 → WADDR +0x044, RADDR +0x048, WDAT +0x04c, RDAT +0x050. */
#define HBUS_TARG_PRPH_WADDR         0x444
#define HBUS_TARG_PRPH_RADDR         0x448
#define HBUS_TARG_PRPH_WDATA         0x44c
#define HBUS_TARG_PRPH_RDAT          0x450
/* iwl-prph.h: familia 22000 — no confundir con offsets legacy (0xd03c). */
#define UREG_CPU_INIT_RUN            0xa05c44
/* Linux iwl-prph.h: doorbell UMAC; BIT(20) = carga PNVM (AX211/gen3). */
#define UREG_DOORBELL_TO_ISR6        0xa05c04
#define UREG_DOORBELL_TO_ISR6_PNVM   (1u << 20)
#define HPM_DEBUG                    0xa03440
#define PREG_PRPH_WPROT_22000        0xa04d00
/* Linux `iwl_trans_pcie_prph_msk` + `iwl_so_trans_cfg.umac_prph_offset`. */
#define IWL_PRPH_MSK_GEN2            0x000fffffu
#define IWL_PRPH_MSK_GEN3            0x00ffffffu
#define IWL_UMAC_PRPH_OFFSET         0x300000u
#define RFH_Q0_FRBDCB_WIDX_TRG       0x1C80
#define IWL_PCI_AX200                0x2723u
#define IWL_PCI_8265                 0x24fdu

/* Familias que el port distingue. No es el enum completo de Linux:
 * 8000 no usa context-info gen2/gen3. */
enum iwl_device_family {
    IWL_DEVICE_FAMILY_UNKNOWN = 0,
    IWL_DEVICE_FAMILY_8000,
    IWL_DEVICE_FAMILY_22000,
    IWL_DEVICE_FAMILY_AX210,
};
#define IWL_MVM_DQA_CMD_QUEUE        0
#define DQA_ENABLE_CMD               0x0
#define IWL_MVM_HCMD_TIMEOUT_MS      2000
#define IWL_GEN2_RX_N                32
#define IWL_GEN2_RX_SZ               4096

/* Linux pcie/rx.c: `r = closed_rb_num & 0xFFF; r &= (queue_size - 1)`.
 * Sin el wrap, closed=32 y rx_read∈0..31 recorren el anillo entero. */
static inline uint16_t iwl_closed_rb_idx(const volatile uint16_t *rb_stts,
                                         unsigned queue_n)
{
    uint16_t hw = rb_stts[0] & 0x0fffu;

    return (uint16_t)(hw & (queue_n - 1u));
}

/* Contrato DMA del anillo RX, distinto por generación (pcie/internal.h).
 *
 *  - 22000 / AX200 (`!gen3`): BD libre = `__le64 (addr | vid)`; el descriptor
 *    completado es un `__le32` con el VID en los 12 bits bajos.
 *  - AX210 / AX211 (`gen3`): BD libre = `struct iwl_rx_transfer_desc` de 16 B
 *    con el `rbid` **fuera** de la dirección; el completado mide 32 B y lleva
 *    el `rbid` en el offset 4.
 *
 * El VID va de 1 a N y designa el buffer `VID - 1`: el 0 no es un buffer, es
 * «ranura vacía». Usar en gen3 el formato de gen2 hacía que el firmware
 * escribiera direcciones y el consumidor leyera índices de otro sitio, que es
 * exactamente el síntoma de `timeout ALIVE` con recepciones vacías. */
struct iwl_rx_transfer_desc {
    uint16_t rbid;
    uint16_t reserved[3];
    uint64_t addr;
} __attribute__((packed));

#define IWL_RX_BD_SIZE_GEN2          8u
#define IWL_RX_BD_SIZE_GEN3          16u
#define IWL_RX_CD_SIZE_GEN2          4u
#define IWL_RX_CD_SIZE_GEN3          32u

struct iwl_rx_completion_desc {
    uint32_t reserved1;
    uint16_t rbid;
    uint8_t status;
    uint8_t reserved2[25];
} __attribute__((packed));

typedef char iwl_rx_transfer_desc_sz[
    sizeof(struct iwl_rx_transfer_desc) == IWL_RX_BD_SIZE_GEN3 ? 1 : -1];
typedef char iwl_rx_completion_desc_sz[
    sizeof(struct iwl_rx_completion_desc) == IWL_RX_CD_SIZE_GEN3 ? 1 : -1];
typedef char iwl_rx_completion_rbid_off[
    offsetof(struct iwl_rx_completion_desc, rbid) == 4 ? 1 : -1];
#define IWL_TFH_TFD_SIZE             256
#define IWL_TFH_NUM_TBS              25
#define IWL_NUM_OF_TBS               20
#define IWL_GEN1_TFD_SIZE            128
#define IWL_8000_NUM_QUEUES          31
#define IWL_8000_TFD_RING_N          256
#define IWL_MVM_TX_FIFO_CMD          7
#define SCD_WIN_SIZE                 64
#define SCD_FRAME_LIMIT              64
#define SCD_QUEUE_STTS_REG_POS_TXF   0
#define SCD_QUEUE_STTS_REG_POS_ACTIVE 3
#define SCD_QUEUE_STTS_REG_POS_WSL   4
#define SCD_QUEUE_STTS_REG_POS_SCD_ACT_EN 19
#define SCD_QUEUE_STTS_REG_MSK       0x017f0000u
#define SCD_CONTEXT_MEM_LOWER_BOUND  0x600u
#define SCD_TRANS_TBL_OFFSET_QUEUE(q) \
    (((0x7e0u + ((unsigned)(q) * 2u)) & 0xfffcu))
#define SCD_BASE                     0xa02c00u
#define SCD_SRAM_BASE_ADDR           (SCD_BASE + 0x0u)
#define SCD_DRAM_BASE_ADDR           (SCD_BASE + 0x8u)
#define SCD_TXFACT                   (SCD_BASE + 0x10u)
#define SCD_CHAINEXT_EN              (SCD_BASE + 0x244u)
#define SCD_EN_CTRL                  (SCD_BASE + 0x254u)
#define SCD_QUEUE_WRPTR(q)           (SCD_BASE + 0x18u + (unsigned)(q) * 4u)
#define SCD_QUEUE_RDPTR(q)           (SCD_BASE + 0x68u + (unsigned)(q) * 4u)
#define SCD_QUEUE_STATUS_BITS(q)     (SCD_BASE + 0x10cu + (unsigned)(q) * 4u)
#define SCD_CONTEXT_QUEUE_OFFSET(q)  (SCD_CONTEXT_MEM_LOWER_BOUND + ((unsigned)(q) * 8u))
#define FH_TCSR_CHNL_NUM             8
#define FH_TCSR_TX_CONFIG_REG_VAL_DMA_CREDIT_ENABLE 0x00000008u
#define FH_TX_CHICKEN_BITS_REG       (FH_MEM_LOWER_BOUND + 0xe98u)
#define FH_TX_CHICKEN_BITS_SCD_AUTO_RETRY_EN 0x00000002u
#define FH_MEM_CBBC_QUEUE(chnl)      (FH_MEM_LOWER_BOUND + 0x9d0u + 4u * (unsigned)(chnl))
#define IWL_CTXT_INFO_TFD_FORMAT_LONG 0x0100u
#define IWL_CTXT_INFO_RB_CB_SIZE_32   0x0050u
#define IWL_CTXT_INFO_RB_SIZE_4K      0x0800u
#define TFD_QUEUE_CB_SIZE_32          2
#define SCD_QUEUE_CFG                 0x1d
#define SCD_QUEUE_CONFIG_CMD          0x17
#define TLC_MNG_CONFIG_CMD            0x0f
#define RLC_CONFIG_CMD                0x08

/* Linux fw/api/tx.h — tamaño cabecera MAC en palabras dentro de offload_assist. */
#define TX_CMD_OFFLD_MH_SIZE          8
#define IWL_SCD_QUEUE_ADD             0
/* Linux iwl-trans.h: IWL_MAX_TID_COUNT=8 es el índice lógico mgmt en MVM.
 * En SCD v3 el payload lleva IWL_MGMT_TID=15 (iwl_mvm_tvqm_enable_txq convierte
 * 8→15 antes de iwl_trans_txq_alloc). Confundir ambos rompe el ADD en placa. */
#define IWL_MAX_TID_COUNT             8
#define IWL_TID_NON_QOS               0
#define IWL_MGMT_TID                  15
#define IWL_MGMT_QUEUE_SIZE           16
/* Linux iwl-fh.h: tabla BC gen2 = TFD_QUEUE_SIZE_MAX + TFD_QUEUE_SIZE_BC_DUP. */
#define TFD_QUEUE_SIZE_MAX            256
#define TFD_QUEUE_SIZE_BC_DUP         64
#define TFD_QUEUE_BC_SIZE             (TFD_QUEUE_SIZE_MAX + TFD_QUEUE_SIZE_BC_DUP)
#define IWL_SCD_BC_TBL_BYTES          (TFD_QUEUE_BC_SIZE * (unsigned)sizeof(uint16_t))
#define IWL_SCD_DMA_ALIGN             256
#define IWL_FIRST_TB_SIZE             20
#define IWL_FIRST_TB_SIZE_ALIGN       64
#define IWL_CMD_VER_MAX               256
/* Cabecera de comando + tx_cmd + 802.11/SNAP sobre una trama Ethernet de MTU:
 * con 512 no cabía ni la mitad de un paquete de datos. */
#define IWL_MGMT_TX_SLOT_SIZE         2048
#define TX_QUEUE_CFG_ENABLE_QUEUE     (1u << 0)

static inline uint32_t tfd_queue_cb_size(unsigned qsize)
{
    unsigned l2 = 0;
    unsigned s = qsize;

    while (s > 1u) {
        s >>= 1;
        l2++;
    }
    return l2 >= 3u ? l2 - 3u : 0u;
}

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
#define CSR_INT_BIT_FH_TX                     (1u << 27)
#define CSR_INT_BIT_FH_RX                     (1u << 31)
#define CSR_FH_INT_STATUS                     (CSR_BASE + 0x010)
#define CSR_FH_INT_BIT_TX_CHNL0               (1u << 0)
#define CSR_FH_INT_BIT_TX_CHNL1               (1u << 1)
#define CSR_FH_INT_TX_MASK                    (CSR_FH_INT_BIT_TX_CHNL0 | CSR_FH_INT_BIT_TX_CHNL1)
#define CSR_DRAM_INT_TBL_REG                  (CSR_BASE + 0x0A0)
#define CSR_DRAM_INT_TBL_ENABLE               (1u << 31)
#define CSR_DRAM_INIT_TBL_WRITE_POINTER       (1u << 28)
#define CSR_DRAM_INIT_TBL_WRAP_CHECK          (1u << 27)
#define IWL_ICT_SIZE                          4096u
#define IWL_ICT_SHIFT                         12

/* FH legado (familia 8000): carga por canal de servicio + RX de un anillo. */
#define FH_MEM_LOWER_BOUND                    0x1000u
#define FH_MEM_TB_MAX_LENGTH                  0x00020000u
#define FH_SRVC_CHNL                          9
#define FH_SRVC_LOWER_BOUND                   (FH_MEM_LOWER_BOUND + 0x9C8u)
#define FH_SRVC_CHNL_SRAM_ADDR_REG(ch)        (FH_SRVC_LOWER_BOUND + ((unsigned)(ch) - 9u) * 4u)
#define FH_TFDIB_LOWER_BOUND                  (FH_MEM_LOWER_BOUND + 0x900u)
#define FH_TFDIB_CTRL0_REG(ch)                (FH_TFDIB_LOWER_BOUND + 8u * (unsigned)(ch))
#define FH_TFDIB_CTRL1_REG(ch)                (FH_TFDIB_LOWER_BOUND + 8u * (unsigned)(ch) + 4u)
#define FH_MEM_TFDIB_REG1_ADDR_BITSHIFT       28
#define FH_TCSR_LOWER_BOUND                   (FH_MEM_LOWER_BOUND + 0xD00u)
#define FH_TCSR_CHNL_TX_CONFIG_REG(ch)        (FH_TCSR_LOWER_BOUND + 0x20u * (unsigned)(ch))
#define FH_TCSR_CHNL_TX_BUF_STS_REG(ch)       (FH_TCSR_LOWER_BOUND + 0x20u * (unsigned)(ch) + 8u)
#define FH_TCSR_TX_CONFIG_REG_VAL_DMA_CHNL_PAUSE      0u
#define FH_TCSR_TX_CONFIG_REG_VAL_DMA_CHNL_ENABLE     0x80000000u
#define FH_TCSR_TX_CONFIG_REG_VAL_DMA_CREDIT_DISABLE  0u
#define FH_TCSR_TX_CONFIG_REG_VAL_CIRQ_HOST_ENDTFD    0x00100000u
#define FH_TCSR_CHNL_TX_BUF_STS_REG_POS_TB_NUM        20
#define FH_TCSR_CHNL_TX_BUF_STS_REG_POS_TB_IDX        12
#define FH_TCSR_CHNL_TX_BUF_STS_REG_VAL_TFDB_VALID    0x00000003u
#define FH_TSSR_TX_STATUS_REG                 (FH_MEM_LOWER_BOUND + 0xEA0u + 0x010u)
#define FH_TSSR_TX_STATUS_REG_MSK_CHNL_IDLE(ch) ((1u << (unsigned)(ch)) << 16)
#define FH_KW_MEM_ADDR_REG                    (FH_MEM_LOWER_BOUND + 0x97Cu)
#define FH_RSCSR_CHNL0_STTS_WPTR_REG          (FH_MEM_LOWER_BOUND + 0xBC0u)
#define FH_RSCSR_CHNL0_RBDCB_BASE_REG         (FH_MEM_LOWER_BOUND + 0xBC4u)
#define FH_RSCSR_CHNL0_WPTR                   (FH_MEM_LOWER_BOUND + 0xBC8u)
#define FH_RSCSR_CHNL0_RDPTR                  (FH_MEM_LOWER_BOUND + 0xBCCu)
#define FH_MEM_RCSR_CHNL0_CONFIG_REG          (FH_MEM_LOWER_BOUND + 0xC00u)
#define FH_MEM_RCSR_CHNL0_RBDCB_WPTR          (FH_MEM_LOWER_BOUND + 0xC08u)
#define FH_MEM_RCSR_CHNL0_FLUSH_RB_REQ        (FH_MEM_LOWER_BOUND + 0xC10u)
#define FH_RCSR_RX_CONFIG_CHNL_EN_ENABLE_VAL  0x80000000u
#define FH_RCSR_CHNL0_RX_IGNORE_RXF_EMPTY     0x00000004u
#define FH_RCSR_CHNL0_RX_CONFIG_IRQ_DEST_INT_HOST_VAL 0x00001000u
#define FH_RCSR_RX_CONFIG_REG_VAL_RB_SIZE_4K  0u
#define FH_RCSR_RX_CONFIG_REG_IRQ_RBTH_POS    4
#define FH_RCSR_RX_CONFIG_RBDCB_SIZE_POS      20
#define RX_RB_TIMEOUT                         0x11u
#define IWL_8000_RX_N                         32
#define IWL_8000_RX_LOG                       5
#define FH_UCODE_LOAD_STATUS                  0x1AF0u
#define RELEASE_CPU_RESET                     0x300Cu
#define RELEASE_CPU_RESET_BIT                 (1u << 24)
#define WFPM_GP2                              0xA030B4u
#define LMPM_CHICK                            0xA01FF8u
#define LMPM_CHICK_EXTENDED_ADDR_SPACE        (1u << 0)
#define IWL_FW_MEM_EXTENDED_START             0x40000u
#define IWL_FW_MEM_EXTENDED_END               0x57FFFu
#define IWL_8000_PLAN_MAX                     64
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

static inline uint64_t iwl_cpu_to_le64(uint64_t v)
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
#define DEBUG_GROUP                0xf
/* fw/api/debug.h: TFD idle; DMA real, no dirección 0. */
#define INVALID_WR_PTR_CMD         0x6

#define INIT_EXTENDED_CFG_CMD      0x03
/* Linux fw/api/nvm-reg.h: PNVM_INIT_COMPLETE_NTFY, grupo 0xc. */
#define PNVM_INIT_COMPLETE_NTFY    0xFE
/* Linux fw/pnvm.h: MVM_UCODE_PNVM_TIMEOUT = HZ/4. */
#define IWL_PNVM_TIMEOUT_MS        250
/* Linux fw/api/alive.h: iwl_alive_ntf_v5; sku_id detrás de umac (v5=128, v6=144). */
#define IWL_ALIVE_NTFY_V5_LEN      128
#define IWL_ALIVE_SKU_OFF          116
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

/* Linux fw/api/phy-ctxt.h — bandas PHY del firmware. */
#define PHY_BAND_5                 0
#define PHY_BAND_24                1
#define PHY_BAND_6                 2
#define IWL_PHY_CHANNEL_MODE20     0
#define IWL_LMAC_24G_INDEX         0u
#define IWL_LMAC_5G_INDEX          1u

/* iwl-nvm-parse.c — orden de canales en el perfil regulatorio NVM. */
#define IWL_NVM_NUM_CHANNELS       39
#define IWL_NVM_NUM_CHANNELS_EXT   51
#define IWL_NVM_NUM_CHANNELS_UHB   110
#define NUM_2GHZ_CHANNELS          14
#define NUM_5GHZ_CHANNELS          37

#define IWL_SCAN_CHANNEL_FLAG_ENABLE_CHAN_ORDER (1u << 5)

#define IWL_UCODE_TLV_PHY_SKU                  23
#define IWL_UCODE_TLV_ENABLED_CAPABILITIES       30
#define IWL_UCODE_TLV_N_SCAN                     31
#define IWL_UCODE_TLV_CMD_VERSIONS               48

/* Linux `enum iwl_ucode_tlv_capa` — bit 12 = DQA_SUPPORT (file.h). */
#define IWL_UCODE_TLV_CAPA_DQA_SUPPORT           12
#define IWL_UCODE_TLV_CAPA_FRAGMENTED_PNVM_IMG   32
#define IWL_UCODE_TLV_CAPA_BINDING_CDB_SUPPORT   39
#define IWL_UCODE_TLV_CAPA_CDB_SUPPORT           40
#define IWL_UCODE_TLV_CAPA_TLC_OFFLOAD           43
#define IWL_UCODE_TLV_CAPA_SESSION_PROT_CMD      54
#define IWL_FW_CAPA_SETS                         4
#define IWL_UCODE_TLV_HW_TYPE                    58
#define IWL_UCODE_TLV_PNVM_VERSION               62
#define IPC_DRAM_MAP_ENTRY_NUM_MAX               64
#define UNFRAGMENTED_PNVM_PAYLOADS_NUMBER        2

#define BINDING_CONTEXT_CMD                      0x2b
#define TIME_EVENT_CMD                           0x29
#define SESSION_PROTECTION_CMD                   0x05
#define SESSION_PROTECTION_NOTIF                 0xfb
#define SESSION_PROTECT_CONF_ASSOC               0
#define ADD_STA_KEY                              0x17

#define TE_BSS_STA_AGGRESSIVE_ASSOC              0
#define TE_V2_FRAG_NONE                          0
#define TE_V2_NOTIF_HOST_EVENT_START             (1u << 0)
#define TE_V2_NOTIF_HOST_EVENT_END               (1u << 1)
#define TE_V2_START_IMMEDIATELY                  (1u << 11)

#define STA_KEY_FLG_CCM                          (2u << 0)
#define STA_KEY_FLG_WEP_KEY_MAP                  (1u << 3)
#define STA_KEY_FLG_KEYID_POS                    8
#define STA_KEY_FLG_KEYID_MSK                    (3u << STA_KEY_FLG_KEYID_POS)
#define STA_KEY_NOT_VALID                        (1u << 11)
/* Clave de grupo. Linux mvm/sta.c la pone en toda GTK; sin ella el firmware
 * la registra como otra clave de pares. */
#define STA_KEY_MULTICAST                        (1u << 14)
#define STA_KEY_MFP                              (1u << 15)

#define IWL_MVM_TE_SESSION_PROTECTION_MAX_TIME_MS 600u
#define IWL_MVM_TE_ASSOC_MAX_DELAY_MS             500u
/* Linux mvm.h: MSEC_TO_TU; mac80211.c usa 900 ms para session protection. */
#define MSEC_TO_TU(_msec)                        (((uint32_t)(_msec) * 1000u) / 1024u)
#define IWL_MVM_SESSION_PROTECTION_ASSOC_MS        900u
#define POWER_TABLE_CMD                          0x77
#define MAC_PM_POWER_TABLE                       0xa9
#define FW_CTXT_INVALID                          0xffffffffu
#define MAX_MACS_IN_BINDING                      3
#define IWL_BINDING_CMD_SIZE_V1                  24
#define POWER_KEEP_ALIVE_PERIOD_SEC              25

struct iwl_binding_cmd_v1 {
    uint32_t id_and_color;
    uint32_t action;
    uint32_t macs[MAX_MACS_IN_BINDING];
    uint32_t phy;
} __attribute__((packed));

struct iwl_time_event_cmd {
    uint32_t id_and_color;
    uint32_t action;
    uint32_t id;
    uint32_t apply_time;
    uint32_t max_delay;
    uint32_t depends_on;
    uint32_t interval;
    uint32_t duration;
    uint8_t repeat;
    uint8_t max_frags;
    uint16_t policy;
} __attribute__((packed));

struct iwl_mvm_session_prot_cmd {
    uint32_t id_and_color;
    uint32_t action;
    uint32_t conf_id;
    uint32_t duration_tu;
    uint32_t repetition_count;
    uint32_t interval;
} __attribute__((packed));

struct iwl_mvm_add_sta_key_common {
    uint8_t sta_id;
    uint8_t key_offset;
    uint16_t key_flags;
    uint8_t key[32];
    uint8_t rx_secur_seq_cnt[16];
} __attribute__((packed));

struct iwl_mvm_add_sta_key_cmd {
    struct iwl_mvm_add_sta_key_common common;
    uint64_t rx_mic_key;
    uint64_t tx_mic_key;
    uint64_t transmit_seq_cnt;
} __attribute__((packed));

struct iwl_binding_cmd {
    uint32_t id_and_color;
    uint32_t action;
    uint32_t macs[MAX_MACS_IN_BINDING];
    uint32_t phy;
    uint32_t lmac_id;
} __attribute__((packed));

struct iwl_device_power_cmd {
    uint16_t flags;
    uint16_t reserved;
} __attribute__((packed));

struct iwl_mac_power_cmd {
    uint32_t id_and_color;
    uint16_t flags;
    uint16_t keep_alive_seconds;
    uint32_t rx_data_timeout;
    uint32_t tx_data_timeout;
    uint32_t rx_data_timeout_uapsd;
    uint32_t tx_data_timeout_uapsd;
    uint8_t lprx_rssi_threshold;
    uint8_t skip_dtim_periods;
    uint16_t snooze_interval;
    uint16_t snooze_window;
    uint8_t snooze_step;
    uint8_t qndp_tid;
    uint8_t uapsd_ac_flags;
    uint8_t uapsd_max_sp;
    uint8_t heavy_tx_thld_packets;
    uint8_t heavy_rx_thld_packets;
    uint8_t heavy_tx_thld_percentage;
    uint8_t heavy_rx_thld_percentage;
    uint8_t limited_ps_threshold;
    uint8_t reserved;
} __attribute__((packed));

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
#define SCAN_LB_LMAC_IDX           0
#define SCAN_HB_LMAC_IDX           1

/* Linux mvm/scan.c — dwell/adwell/priority de scan UMAC unassoc. */
#define IWL_SCAN_DWELL_ACTIVE                  10
#define IWL_SCAN_DWELL_PASSIVE                 110
#define IWL_SCAN_ADWELL_DEFAULT_LB_N_APS       2
#define IWL_SCAN_ADWELL_DEFAULT_HB_N_APS       8
#define IWL_SCAN_ADWELL_DEFAULT_N_APS_SOCIAL   10
#define IWL_SCAN_ADWELL_MAX_BUDGET_FULL_SCAN   300
#define IWL_SCAN_ADWELL_N_APS_GO_FRIENDLY      10
#define IWL_SCAN_ADWELL_N_APS_SOCIAL_CHS       2
#define IWL_SCAN_PRIORITY_EXT_6                6
#define IWL_MAX_SCHED_SCAN_PLANS   2
#define PROBE_OPTION_MAX           20
#define SCAN_SHORT_SSID_MAX_SIZE   8
#define SCAN_BSSID_MAX_SIZE        16
#define SCAN_NUM_BAND_PROBE_DATA_V_2 3
#define IWL_RX_DESC_SIZE_V1        48u

#define ADD_STA                    0x18
#define ADD_STA_SUCCESS            0x1
#define IWL_ADD_STA_STATUS_MASK    0xffu
#define LQ_CMD                     0x4e
#define TX_CMD                     0x1c
#define IWL_TX_FLAGS_CMD_RATE      (1u << 0)
#define IWL_TX_FLAGS_ENCRYPT_DIS   (1u << 1)
#define IWL_TX_FLAGS_HIGH_PRI      (1u << 2)
#define RATE_MCS_LEGACY_OFDM_MSK  (1u << 8)
#define RATE_MCS_ANT_A_MSK         (1u << 14)
#define RATE_LEGACY_OFDM_6M       0u
#define RATE_LEGACY_PLCP_6M       0x0du

struct iwl_dram_sec_info {
    uint32_t pn_low;
    uint16_t pn_high;
    uint16_t aux_info;
} __attribute__((packed));

/* AX200 / device_family < AX210: TX_CMD_API_S_VER_7/9. */
struct iwl_tx_cmd_gen2 {
    uint16_t len;
    uint16_t offload_assist;
    uint32_t flags;
    struct iwl_dram_sec_info dram_info;
    uint32_t rate_n_flags;
    uint8_t hdr[];
} __attribute__((packed));

/* AX210+: TX_CMD_API_S_VER_8/10. */
struct iwl_tx_cmd_gen3 {
    uint16_t len;
    uint16_t flags;
    uint32_t offload_assist;
    struct iwl_dram_sec_info dram_info;
    uint32_t rate_n_flags;
    uint8_t reserved[8];
    uint8_t hdr[];
} __attribute__((packed));

#define FW_CTXT_ID_POS             0
#define FW_CTXT_COLOR_POS          8
#define FW_CMD_ID_AND_COLOR(id, color) \
    (((uint32_t)(id) << FW_CTXT_ID_POS) | ((uint32_t)(color) << FW_CTXT_COLOR_POS))
#define FW_CTXT_ACTION_ADD         1u
#define FW_CTXT_ACTION_MODIFY      2u
#define FW_CTXT_ACTION_REMOVE      3u

#define IWL_STA_LINK               0u
#define IWL_STA_GENERAL_PURPOSE    1u
/* Linux mvm/sta.c: el AP de un vif STA ocupa sta_id 0. ADD_STA v12 no crea aux. */
#define IWL_MVM_AP_STA_ID          0u

#define STA_FLG_CLASS_AUTH         (1u << 14)
#define STA_FLG_CLASS_ASSOC        (1u << 15)
#define STA_FLG_FAT_EN_20MHZ       (0u << 26)
#define STA_FLG_FAT_EN_40MHZ       (1u << 26)
#define STA_FLG_FAT_EN_MSK         (3u << 26)
#define STA_FLG_MIMO_EN_SISO       (0u << 28)
#define STA_FLG_MIMO_EN_MSK        (3u << 28)

/* Linux fw/api/sta.h ADD_STA_CMD_API_S_VER_10 (48 B). */
struct iwl_mvm_add_sta_cmd {
    uint8_t add_modify;
    uint8_t awake_acs;
    uint16_t tid_disable_tx;
    uint32_t mac_id_n_color;
    uint8_t addr[6];
    uint16_t reserved2;
    uint8_t sta_id;
    uint8_t modify_mask;
    uint16_t reserved3;
    uint32_t station_flags;
    uint32_t station_flags_msk;
    uint8_t add_immediate_ba_tid;
    uint8_t remove_immediate_ba_tid;
    uint16_t add_immediate_ba_ssn;
    uint16_t sleep_tx_count;
    uint8_t sleep_state_flags;
    uint8_t station_type;
    uint16_t assoc_id;
    uint16_t beamform_flags;
    uint32_t tfd_queue_msk;
    uint16_t rx_ba_window;
    uint8_t sp_length;
    uint8_t uapsd_acs;
} __attribute__((packed));

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
#define WLAN_EID_TIM               5
#define WLAN_EID_SUPP_RATES        1
#define WLAN_EID_HT_OPERATION       61
#define WLAN_EID_RSN               48
#define WLAN_CAPABILITY_ESS        1u
#define WLAN_CAPABILITY_PRIVACY    (1u << 4)
#define IEEE80211_FTYPE_MGMT      0x0000u
#define IEEE80211_STYPE_ASSOC_REQ  0x0000u
#define IEEE80211_STYPE_ASSOC_RESP 0x0010u
#define IEEE80211_STYPE_BEACON     0x0080u
#define IEEE80211_STYPE_AUTH       0x00b0u
#define IEEE80211_FC_STYPE_MASK    0x00fcu

/* Linux ieee80211_ht_operation: el primer byte es primary_chan. */
struct ieee80211_ht_operation {
    uint8_t primary_chan;
    uint8_t ht_param;
    uint16_t operation_mode;
    uint16_t stbc_param;
    uint8_t basic_set[16];
} __attribute__((packed));

typedef char ieee80211_ht_operation_sz[sizeof(struct ieee80211_ht_operation) == 22 ? 1 : -1];
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

/* Linux iwl-context-info-gen3.h: descriptor de chunks PNVM (capa 32). */
struct iwl_prph_scrath_mem_desc_addr_array {
    uint64_t mem_descs[IPC_DRAM_MAP_ENTRY_NUM_MAX];
} __attribute__((packed));

struct iwl_pnvm_chunk {
    const uint8_t *data;
    uint32_t len;
};

struct iwl_pnvm_image {
    uint32_t version;
    unsigned n_chunks;
    struct iwl_pnvm_chunk chunks[IPC_DRAM_MAP_ENTRY_NUM_MAX];
};

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

struct iwl_tfd_tb {
    uint32_t lo;
    uint16_t hi_n_len;
} __attribute__((packed));

struct iwl_tfd {
    uint8_t __reserved1[3];
    uint8_t num_tbs;
    struct iwl_tfd_tb tbs[IWL_NUM_OF_TBS];
    uint32_t __pad;
} __attribute__((packed));
typedef char iwl_gen1_tfd_size_check[sizeof(struct iwl_tfd) == IWL_GEN1_TFD_SIZE ? 1 : -1];

struct iwl_tfh_tfd {
    uint64_t addr;
    uint16_t num_bytes;
    uint8_t reserved[6];
} __attribute__((packed));

struct iwl_tfh_tb {
    uint16_t tb_len;
    uint64_t addr;
} __attribute__((packed));

struct iwl_tfh_tfd_gen2 {
    uint16_t num_tbs;
    struct iwl_tfh_tb tbs[IWL_TFH_NUM_TBS];
    uint32_t pad;
} __attribute__((packed));

/* Linux queue/tx.c: iwl_txq_set_tfd_invalid_gen2 — TB0 = invalid_tx_cmd. */
static inline void iwl_txq_set_tfd_invalid_gen2(struct iwl_tfh_tfd_gen2 *tfd,
                                                uint64_t dma, uint16_t size)
{
    tfd->num_tbs = 0;
    tfd->tbs[0].addr = dma;
    tfd->tbs[0].tb_len = size;
    tfd->num_tbs = 1;
}

struct iwl_tx_queue_cfg_cmd {
    uint8_t sta_id;
    uint8_t tid;
    uint16_t flags;
    uint32_t cb_size;
    uint64_t byte_cnt_addr;
    uint64_t tfdq_addr;
} __attribute__((packed));

struct iwl_tx_queue_cfg_rsp {
    uint16_t queue_number;
    uint16_t flags;
    uint16_t write_pointer;
    uint16_t reserved;
} __attribute__((packed));

/* Linux fw/api/datapath.h — queue_alloc_cmd_ver==3 (cc-a0-77, so-a0-gf-a0). */
struct iwl_scd_queue_cfg_cmd {
    uint32_t operation;
    union {
        struct {
            uint32_t sta_mask;
            uint8_t tid;
            uint8_t reserved[3];
            uint32_t flags;
            uint32_t cb_size;
            uint64_t bc_dram_addr;
            uint64_t tfdq_dram_addr;
        } __attribute__((packed)) add;
    } u;
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

/* Linux fw/api/cmdhdr.h — cabecera corta TFD (4 B), no confundir con wide HCMD. */
struct iwl_cmd_header {
    uint8_t cmd;
    uint8_t group_id;
    uint16_t sequence;
} __attribute__((packed));

struct iwl_cmd_header_wide {
    uint8_t cmd;
    uint8_t group_id;
    uint16_t sequence;
    uint16_t length;
    uint8_t reserved;
    uint8_t version;
} __attribute__((packed));

/* Linux pcie/trans.c: iwl_pcie_alloc_invalid_tx_cmd. */
static inline void iwl_invalid_tx_cmd_init(struct iwl_cmd_header_wide *hdr)
{
    hdr->cmd = INVALID_WR_PTR_CMD;
    hdr->group_id = DEBUG_GROUP;
    hdr->sequence = iwl_cpu_to_le16(0xffff);
    hdr->length = iwl_cpu_to_le16(0);
    hdr->reserved = 0;
    hdr->version = 0;
}

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

/* Linux fw/api/rx.h iwl_rx_phy_info (REPLY_RX_PHY_CMD = 0xc0).
 * `channel` va a offset 22; phy_flags BIT(0) = 2.4 GHz. */
#define RX_RES_PHY_FLAGS_BAND_24       1u
#define IWL_RX_INFO_PHY_CNT            8
#define IWL_RX_INFO_ENERGY_ANT_ABC_IDX 1

struct iwl_rx_phy_info {
    uint8_t non_cfg_phy_cnt;
    uint8_t cfg_phy_cnt;
    uint8_t stat_id;
    uint8_t reserved1;
    uint32_t system_timestamp;
    uint64_t timestamp;
    uint32_t beacon_time_stamp;
    uint16_t phy_flags;
    uint16_t channel;
    uint32_t non_cfg_phy[IWL_RX_INFO_PHY_CNT];
} __attribute__((packed));

typedef char iwl_rx_phy_channel_off[
    offsetof(struct iwl_rx_phy_info, channel) == 22 ? 1 : -1];

/* Prefijo DW2–DW6 + v1 de iwl_rx_mpdu_desc (AX200 / device_family < AX210).
 * Linux IWL_RX_DESC_SIZE_V1 = offsetofend(..., v1) = 48. */
struct iwl_rx_mpdu_desc_v1 {
    uint32_t rss_hash;
    uint32_t filter_match;
    uint32_t rate_n_flags;
    uint8_t energy_a;
    uint8_t energy_b;
    uint8_t channel;
    uint8_t mac_context;
    uint32_t gp2_on_air_rise;
    uint64_t tsf_on_air_rise;
} __attribute__((packed));

/* v3 de iwl_rx_mpdu_desc (AX210 / AX211). Mismo prefijo de 20 B que v1, pero
 * la cola mide 36 en vez de 28: el descriptor completo son 56 B, no 48.
 * Linux fw/api/rx.h, IWL_RX_DESC_SIZE_V3 = offsetofend(..., v3). */
struct iwl_rx_mpdu_desc_v3 {
    uint32_t filter_match;
    uint32_t rss_hash;
    uint32_t partial_hash;
    uint16_t raw_xsum;
    uint16_t reserved_xsum;
    uint32_t rate_n_flags;
    uint8_t energy_a;
    uint8_t energy_b;
    uint8_t channel;
    uint8_t mac_context;
    uint32_t gp2_on_air_rise;
    uint64_t tsf_on_air_rise;
} __attribute__((packed));

struct iwl_rx_mpdu_desc {
    uint16_t mpdu_len;
    uint8_t mac_flags1;
    uint8_t mac_flags2;
    uint8_t amsdu_info;
    uint16_t phy_info;
    uint8_t mac_phy_idx;
    uint32_t dw4;
    uint32_t status;
    uint32_t reorder_data;
    union {
        struct iwl_rx_mpdu_desc_v1 v1;
        struct iwl_rx_mpdu_desc_v3 v3;
    };
} __attribute__((packed));

/* Tamaño del descriptor que precede a la MPDU, por generación. Saltar 4 B
 * (`iwl_rx_mpdu_res_start`) en AX211 deja la trama 802.11 desplazada 52 B. */
#define IWL_RX_DESC_SIZE_V3        56u

typedef char iwl_rx_mpdu_desc_v3_sz[
    sizeof(struct iwl_rx_mpdu_desc) == IWL_RX_DESC_SIZE_V3 ? 1 : -1];
typedef char iwl_rx_mpdu_desc_v1_off[
    offsetof(struct iwl_rx_mpdu_desc, v1) == 20 ? 1 : -1];
typedef char iwl_rx_mpdu_v1_channel_off[
    offsetof(struct iwl_rx_mpdu_desc, v1.channel) == 34 ? 1 : -1];
typedef char iwl_rx_mpdu_v3_channel_off[
    offsetof(struct iwl_rx_mpdu_desc, v3.channel) == 42 ? 1 : -1];

/* Bits de `status` (DW5). Linux fw/api/rx.h. */
#define IWL_RX_MPDU_STATUS_CRC_OK      (1u << 0)
#define IWL_RX_MPDU_STATUS_OVERRUN_OK  (1u << 1)
#define IWL_RX_MPDU_STATUS_KEY_VALID   (1u << 3)
#define IWL_RX_MPDU_STATUS_ICV_OK      (1u << 5)
#define IWL_RX_MPDU_STATUS_MIC_OK      (1u << 6)
#define IWL_RX_MPDU_STATUS_SEC_MASK    0x0700u
#define IWL_RX_MPDU_STATUS_SEC_NONE    0x0000u
#define IWL_RX_MPDU_STATUS_SEC_CCM     0x0200u
#define IWL_RX_MPDU_STATUS_DECRYPTED   (1u << 11)

/* fw/api/rx.h — TSF del descriptor no es válido; usar cuerpo 802.11 @24. */
#define IWL_RX_MPDU_PHY_TSF_OVERLOAD   (1u << 8)

#define ETH_P_EAPOL                    0x888eu
/* MTU Ethernet que anuncia smoltcp: la conversión y la cola TX tienen que
 * admitirlo entero, no recortarlo a 450 bytes. */
#define IWL_MAX_ETH_FRAME              1514u

static inline uint8_t iwl_rx_phy_info_channel(const uint8_t *data, int len)
{
    const struct iwl_rx_phy_info *phy;

    if (!data || len < (int)(offsetof(struct iwl_rx_phy_info, channel) + 2))
        return 0;
    phy = (const struct iwl_rx_phy_info *)data;
    return (uint8_t)phy->channel;
}

static inline uint16_t iwl_rx_phy_info_flags(const uint8_t *data, int len)
{
    const struct iwl_rx_phy_info *phy;

    if (!data || len < (int)(offsetof(struct iwl_rx_phy_info, phy_flags) + 2))
        return 0;
    phy = (const struct iwl_rx_phy_info *)data;
    return phy->phy_flags;
}

static inline int iwl_rx_phy_info_energy(const uint8_t *data, int len,
                                         uint8_t *a, uint8_t *b)
{
    const struct iwl_rx_phy_info *phy;
    uint32_t energy;
    size_t need = offsetof(struct iwl_rx_phy_info, non_cfg_phy) +
                  (size_t)(IWL_RX_INFO_ENERGY_ANT_ABC_IDX + 1) * sizeof(uint32_t);

    if (!data || !a || !b || len < (int)need)
        return -1;
    phy = (const struct iwl_rx_phy_info *)data;
    energy = phy->non_cfg_phy[IWL_RX_INFO_ENERGY_ANT_ABC_IDX];
    *a = (uint8_t)energy;
    *b = (uint8_t)(energy >> 8);
    return 0;
}

static inline uint8_t iwl_rx_mpdu_v1_channel(const uint8_t *data, int len)
{
    const struct iwl_rx_mpdu_desc *desc;

    if (!data || len < (int)IWL_RX_DESC_SIZE_V1)
        return 0;
    desc = (const struct iwl_rx_mpdu_desc *)data;
    return desc->v1.channel;
}

static inline uint8_t iwl_rx_mpdu_v3_channel(const uint8_t *data, int len)
{
    const struct iwl_rx_mpdu_desc *desc;

    if (!data || len < (int)IWL_RX_DESC_SIZE_V3)
        return 0;
    desc = (const struct iwl_rx_mpdu_desc *)data;
    return desc->v3.channel;
}

static inline void iwl_rx_mpdu_v3_energy(const uint8_t *data, int len,
                                         uint8_t *a, uint8_t *b)
{
    const struct iwl_rx_mpdu_desc *desc;

    if (!a || !b)
        return;
    *a = 0;
    *b = 0;
    if (!data || len < (int)IWL_RX_DESC_SIZE_V3)
        return;
    desc = (const struct iwl_rx_mpdu_desc *)data;
    *a = desc->v3.energy_a;
    *b = desc->v3.energy_b;
}

static inline uint32_t iwl_rx_mpdu_status(const uint8_t *data, int len)
{
    const struct iwl_rx_mpdu_desc *desc;

    if (!data || len < (int)(offsetof(struct iwl_rx_mpdu_desc, status) + 4))
        return 0;
    desc = (const struct iwl_rx_mpdu_desc *)data;
    return desc->status;
}

static inline void iwl_rx_mpdu_v1_energy(const uint8_t *data, int len,
                                          uint8_t *a, uint8_t *b)
{
    const struct iwl_rx_mpdu_desc *desc;

    if (!a || !b)
        return;
    *a = 0;
    *b = 0;
    if (!data || len < (int)IWL_RX_DESC_SIZE_V1)
        return;
    desc = (const struct iwl_rx_mpdu_desc *)data;
    *a = desc->v1.energy_a;
    *b = desc->v1.energy_b;
}

static inline uint64_t iwl_le64_to_cpu(uint64_t v)
{
    return v;
}

static inline uint32_t iwl_le32_to_cpu(uint32_t v)
{
    return v;
}

static inline uint16_t iwl_le16_to_cpu(uint16_t v)
{
    return v;
}

static inline uint64_t iwl_beacon_tsf_from_frame(const uint8_t *frame, int flen)
{
    if (!frame || flen < 32)
        return 0;
    return (uint64_t)frame[24] |
           ((uint64_t)frame[25] << 8) |
           ((uint64_t)frame[26] << 16) |
           ((uint64_t)frame[27] << 24) |
           ((uint64_t)frame[28] << 32) |
           ((uint64_t)frame[29] << 40) |
           ((uint64_t)frame[30] << 48) |
           ((uint64_t)frame[31] << 56);
}

/* Linux mvm/rxmq.c: GP2 siempre del descriptor; TSF salvo TSF_OVERLOAD. */
static inline void iwl_rx_mpdu_beacon_sync(const uint8_t *data, int len, int gen3,
                                           const uint8_t *frame, int flen,
                                           uint64_t *tsf, uint32_t *gp2)
{
    const struct iwl_rx_mpdu_desc *desc;
    uint16_t phy_info;

    if (!tsf || !gp2)
        return;
    *tsf = 0;
    *gp2 = 0;
    if (!data)
        return;
    if (gen3) {
        if (len < (int)IWL_RX_DESC_SIZE_V3)
            return;
        desc = (const struct iwl_rx_mpdu_desc *)data;
        phy_info = iwl_le16_to_cpu(desc->phy_info);
        *gp2 = iwl_le32_to_cpu(desc->v3.gp2_on_air_rise);
        if (phy_info & IWL_RX_MPDU_PHY_TSF_OVERLOAD)
            *tsf = iwl_beacon_tsf_from_frame(frame, flen);
        else
            *tsf = iwl_le64_to_cpu(desc->v3.tsf_on_air_rise);
    } else {
        if (len < (int)IWL_RX_DESC_SIZE_V1)
            return;
        desc = (const struct iwl_rx_mpdu_desc *)data;
        phy_info = iwl_le16_to_cpu(desc->phy_info);
        *gp2 = iwl_le32_to_cpu(desc->v1.gp2_on_air_rise);
        if (phy_info & IWL_RX_MPDU_PHY_TSF_OVERLOAD)
            *tsf = iwl_beacon_tsf_from_frame(frame, flen);
        else
            *tsf = iwl_le64_to_cpu(desc->v1.tsf_on_air_rise);
    }
}

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

/* Linux fw/api/phy-ctxt.h CHANNEL_CONFIG_API_S_VER_2 + PHY_CONTEXT_CMD v3/v4. */
struct iwl_fw_channel_info {
    uint32_t channel;
    uint8_t band;
    uint8_t width;
    uint8_t ctrl_pos;
    uint8_t reserved;
} __attribute__((packed));

struct iwl_rlc_properties {
    uint32_t rx_chain_info;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_sad_properties {
    uint32_t chain_a_sad_mode;
    uint32_t chain_b_sad_mode;
    uint32_t mac_id;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_rlc_config_cmd {
    uint32_t phy_id;
    struct iwl_rlc_properties rlc;
    struct iwl_sad_properties sad;
    uint8_t flags;
    uint8_t reserved[3];
} __attribute__((packed));

struct iwl_phy_context_cmd {
    uint32_t id_and_color;
    uint32_t action;
    struct iwl_fw_channel_info ci;
    uint32_t lmac_id;
    uint32_t rxchain_info;
    uint32_t dsp_cfg_flags;
    uint32_t reserved;
} __attribute__((packed));

struct iwl_phy_context_cmd_v1 {
    uint32_t id_and_color;
    uint32_t action;
    uint32_t apply_time;
    uint32_t tx_param_color;
    struct {
        uint8_t band;
        uint8_t channel;
        uint8_t width;
        uint8_t ctrl_pos;
    } ci;
    uint32_t txchain_info;
    uint32_t rxchain_info;
    uint32_t acquisition_data;
    uint32_t dsp_cfg_flags;
} __attribute__((packed));

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
    struct iwl_scan_probe_segment band_data[SCAN_NUM_BAND_PROBE_DATA_V_2];
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
#define IWL_UMAC_SCAN_GEN_FLAGS_V2_ADAPTIVE_DWELL (1u << 7)
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

#define IWL_FW_AC_NUM              4u
#define IWL_FW_MAC_TYPE_BSS_STA     5u
#define IWL_MAC_FILTER_IN_CONTROL_AND_MGMT (1u << 1)
#define IWL_MAC_FILTER_ACCEPT_GRP  (1u << 2)
#define IWL_MAC_FILTER_IN_BEACON   (1u << 6)

#define TX_STATUS_MSK              0x000000ffu
#define TX_STATUS_SUCCESS            0x01u
#define TX_STATUS_FAIL_LONG_LIMIT    0x83u
#define IWL_MVM_TX_RESP_V3_STATUS_OFF 36u
#define IWL_MVM_TX_RESP_STATUS_OFF     40u
#define IWL_MVM_TX_RESP_MIN_PAY        42u

struct agg_tx_status {
    uint16_t status;
    uint16_t sequence;
} __attribute__((packed));

struct iwl_mvm_session_prot_notif {
    uint32_t mac_id;
    uint32_t status;
    uint32_t start;
    uint32_t conf_id;
} __attribute__((packed));

struct iwl_ac_qos {
    uint16_t cw_min;
    uint16_t cw_max;
    uint8_t aifsn;
    uint8_t fifos_mask;
    uint16_t edca_txop;
} __attribute__((packed));

struct iwl_mac_data_sta {
    uint32_t is_assoc;
    uint32_t dtim_time;
    uint64_t dtim_tsf;
    uint32_t bi;
    uint32_t reserved1;
    uint32_t dtim_interval;
    uint32_t data_policy;
    uint32_t listen_interval;
    uint32_t assoc_id;
    uint32_t assoc_beacon_arrive_time;
} __attribute__((packed));

struct iwl_mac_data_p2p_sta {
    struct iwl_mac_data_sta sta;
    uint32_t ctwin;
} __attribute__((packed));

union iwl_mac_ctx_data {
    struct iwl_mac_data_sta sta;
    struct iwl_mac_data_p2p_sta p2p_sta;
};

struct iwl_mac_ctx_cmd {
    uint32_t id_and_color;
    uint32_t action;
    uint32_t mac_type;
    uint32_t tsf_id;
    uint8_t node_addr[6];
    uint16_t reserved_for_node_addr;
    uint8_t bssid_addr[6];
    uint16_t reserved_for_bssid_addr;
    uint32_t cck_rates;
    uint32_t ofdm_rates;
    uint32_t protection_flags;
    uint32_t cck_short_preamble;
    uint32_t short_slot;
    uint32_t filter_flags;
    uint32_t qos_flags;
    struct iwl_ac_qos ac[IWL_FW_AC_NUM + 1];
    union iwl_mac_ctx_data u;
} __attribute__((packed));

static inline void iwl_mvm_mac_qos_defaults(struct iwl_ac_qos ac[IWL_FW_AC_NUM + 1])
{
    unsigned i;

    for (i = 0; i <= IWL_FW_AC_NUM; i++) {
        ac[i].cw_min = 0x0fu;
        ac[i].cw_max = 0x3fu;
        ac[i].aifsn = 1u;
        ac[i].fifos_mask = 0u;
        ac[i].edca_txop = 0u;
    }
}

int iwl_fw_parse_tlv(struct iwl_ax211_priv *iwl, const uint8_t *fw, unsigned long fw_len);
int iwl_trans_gen2_start(struct iwl_ax211_priv *iwl);
int iwl_trans_gen3_start(struct iwl_ax211_priv *iwl);
int iwl_trans_8000_start(struct iwl_ax211_priv *iwl);
void iwl_trans_8000_drain(struct iwl_ax211_priv *iwl);
int iwl_trans_8000_fw_alive(struct iwl_ax211_priv *iwl);
int iwl_trans_8000_alloc_hcmd(struct iwl_ax211_priv *iwl);

/* Plan de carga FH 8000: comprobable en host, sin NIC. */
struct iwl_8000_chunk {
    uint32_t dst;
    uint32_t len;
    uint32_t off;
    uint8_t cpu;
    uint8_t sec;
};

struct iwl_8000_sec_status {
    uint8_t cpu;
    uint8_t sec;
    uint32_t load_status;
};

struct iwl_8000_load_plan {
    uint32_t n_chunks;
    uint32_t n_status;
    uint32_t cpu1_secs;
    uint32_t cpu2_secs;
    uint32_t cpu1_final;
    uint32_t cpu2_final;
    uint32_t uses_context_info;
    struct iwl_8000_chunk chunks[IWL_8000_PLAN_MAX];
    struct iwl_8000_sec_status status[IWL_FW_RT_MAX];
};

struct iwl_8000_fh_prog {
    uint32_t tcsr_cfg_reg;
    uint32_t tcsr_pause;
    uint32_t sram_reg;
    uint32_t sram_val;
    uint32_t tfdib0_reg;
    uint32_t tfdib0_val;
    uint32_t tfdib1_reg;
    uint32_t tfdib1_val;
    uint32_t buf_sts_reg;
    uint32_t buf_sts_val;
    uint32_t tcsr_run;
};

int iwl_8000_plan_load(const struct iwl_fw_image *fw, struct iwl_8000_load_plan *plan);
int iwl_8000_plan_load_ex(const struct iwl_fw_image *fw, uint32_t chunk_sz,
                          struct iwl_8000_load_plan *plan);
void iwl_8000_fh_program(uint32_t dst, uint64_t dma, uint32_t byte_cnt,
                         struct iwl_8000_fh_prog *out);
void iwl_trans_poll(struct iwl_ax211_priv *iwl);
void iwl_trans_txq_drain_mgmt(struct iwl_ax211_priv *iwl);
int iwl_trans_txq_alloc_data(struct iwl_ax211_priv *iwl, uint8_t sta_id, uint8_t tid);
void iwl_trans_txq_drain_data(struct iwl_ax211_priv *iwl);
int iwl_trans_wait_mgmt_tx_resp(struct iwl_ax211_priv *iwl, unsigned iters);
uint32_t iwl_mvm_tx_rate_n_flags(struct iwl_ax211_priv *iwl);
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
/* Linux iwl_write_umac_prph(UREG_DOORBELL_TO_ISR6, BIT(20)). */
int iwl_trans_pnvm_publish(struct iwl_ax211_priv *iwl);
void iwl_trans_pnvm_doorbell(struct iwl_ax211_priv *iwl);
int iwl_fw_pnvm_select(struct iwl_ax211_priv *iwl, struct iwl_pnvm_image *out);
int iwl_mvm_run_init(struct iwl_ax211_priv *iwl);
int iwl_mvm_up_minimal(struct iwl_ax211_priv *iwl);
int iwl_mvm_phy_ctxt_changed(struct iwl_ax211_priv *iwl, uint8_t channel);
int iwl_mvm_binding_update(struct iwl_ax211_priv *iwl);
int iwl_mvm_binding_send(struct iwl_ax211_priv *iwl, uint32_t action);
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
/* Tabla NVM (legacy/ext/uhb) según nvm_n_channels; expuesta para hostcheck. */
const uint8_t *iwl_mvm_nvm_chan_table(unsigned nvm_n, unsigned *table_n);
uint8_t iwl_mvm_phy_band_from_channel_idx(unsigned ch_idx, unsigned nvm_n);
unsigned iwl_mvm_collect_scan_channels(struct iwl_ax211_priv *iwl,
                                       uint8_t *ch, uint8_t *band,
                                       uint8_t *passive, unsigned max,
                                       int *origen);
int iwl_fw_notif_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd);
uint8_t iwl_mvm_valid_tx_ant(struct iwl_ax211_priv *iwl);
uint8_t iwl_mvm_valid_rx_ant(struct iwl_ax211_priv *iwl);
void iwl_mvm_fill_probe_req(struct iwl_ax211_priv *iwl, struct iwl_scan_probe_params_v4 *probe);
int iwl_mvm_rate_init_ap_sta(struct iwl_ax211_priv *iwl);
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
void iwl_mvm_rx_mlme_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len);
/* Clasifica un beacon/probe response: SSID, canal, capacidades y RSN.
 * Devuelve la longitud del SSID o -1 si la trama no sirve. */
int iwl_mvm_parse_bss(const uint8_t *frame, int len, struct iwl_ax211_bss *out);
/* El BSS con ese SSID exacto y mejor RSSI, o NULL. */
const struct iwl_ax211_bss *iwl_mvm_pick_bss(struct iwl_ax211_priv *iwl,
                                             const char *ssid);
int iwl_mvm_connect_open(struct iwl_ax211_priv *iwl, const char *ssid);
int iwl_mvm_connect_wpa2(struct iwl_ax211_priv *iwl, const char *ssid, const uint8_t psk[32]);
int iwl_mvm_install_key(struct iwl_ax211_priv *iwl, const uint8_t key[16], int key_idx);
int iwl_mvm_install_gtk(struct iwl_ax211_priv *iwl, const uint8_t key[16], int key_idx,
                        const uint8_t rsc[8]);
int iwl_mvm_rsn_ie(const struct iwl_ax211_priv *iwl, uint8_t *out, int max);
int iwl_80211_hdrlen(uint16_t fc);
int iwl_rx_crypto_ok(uint32_t status);
int iwl_mvm_rx_to_eth(const struct iwl_ax211_priv *iwl, const uint8_t *frame, int flen,
                      uint32_t status, uint8_t *out, int outmax);
int iwl_mvm_eth_to_80211(const struct iwl_ax211_priv *iwl, const uint8_t *eth, int len,
                         uint8_t *out, int outmax);

int iwl_mvm_tx_8023(struct iwl_ax211_priv *iwl, const uint8_t *buf, int len);
int iwl_mvm_rx_8023(struct iwl_ax211_priv *iwl, uint8_t *buf, int buflen);

static inline unsigned iwl_mvm_add_sta_cmd_size(struct iwl_ax211_priv *iwl)
{
    if (iwl_fw_cmd_ver(iwl, LEGACY_GROUP, ADD_STA) >= 12)
        return (unsigned)sizeof(struct iwl_mvm_add_sta_cmd);
    return 24u;
}

#endif
