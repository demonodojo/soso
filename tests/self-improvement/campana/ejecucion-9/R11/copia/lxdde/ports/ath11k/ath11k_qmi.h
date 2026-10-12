/* QMI (WLFW) de ath11k — formato del cable.
 *
 * Constantes tomadas de `drivers/net/wireless/ath/ath11k/qmi.h` y de
 * `include/linux/soc/qcom/qmi.h` en el árbol Linux pinneado.
 */
#ifndef ATH11K_QMI_H
#define ATH11K_QMI_H

#include <stddef.h>
#include <stdint.h>

/* Cabecera QMI: type(1) txn_id(2) msg_id(2) msg_len(2) — `struct qmi_header`. */
#define QMI_HDR_LEN 7

#define QMI_REQUEST     0
#define QMI_RESPONSE    2
#define QMI_INDICATION  4

/* TLV estándar de resultado en toda respuesta (`qmi_response_type_v01`). */
#define QMI_TLV_RESULT  0x02
#define QMI_RESULT_SUCCESS 0
#define QMI_RESULT_FAILURE 1

/* Servicio WLFW (qmi.h:18). */
#define ATH11K_QMI_WLFW_SERVICE_ID_V01   0x45
#define ATH11K_QMI_WLFW_SERVICE_VERS_V01 0x01
/* Instancia del servicio para QCA6390/WCN6855 (qmi.h:21). */
#define ATH11K_QMI_WLFW_SERVICE_INS_ID_V01_QCA6390 0x01

/* IDs de mensaje (qmi.h). */
#define QMI_WLANFW_IND_REGISTER_REQ_V01 0x0020
#define QMI_WLANFW_HOST_CAP_REQ_V01     0x0034
#define QMI_WLANFW_WLAN_MODE_REQ_V01    0x0022
#define QMI_WLANFW_WLAN_CFG_REQ_V01     0x0023
#define QMI_WLANFW_CAP_REQ_V01          0x0024
#define QMI_WLANFW_BDF_DOWNLOAD_REQ_V01 0x0025
#define QMI_WLANFW_M3_INFO_REQ_V01      0x003C
#define QMI_WLANFW_RESPOND_MEM_REQ_V01  0x0036

/* Indicaciones que manda el firmware. */
#define QMI_WLFW_FW_READY_IND_V01     0x0021
#define QMI_WLFW_REQUEST_MEM_IND_V01  0x0035
#define QMI_WLFW_FW_MEM_READY_IND_V01 0x0037
#define QMI_WLFW_FW_INIT_DONE_IND_V01 0x0038

/* Identificador del cliente host (qmi.h:196). */
#define QMI_WLANFW_CLIENT_ID 0x4b4e454c

/* Bits de `nm_modem` (qmi.c:19). */
#define SLEEP_CLOCK_SELECT_INTERNAL_BIT 0x02
#define HOST_CSTATE_BIT                 0x04
#define PLATFORM_CAP_PCIE_GLOBAL_RESET  0x08
#define PLATFORM_CAP_PCIE_PME_D3COLD    0x10

/* Segmento máximo de BDF por mensaje (qmi.h:38). */
#define QMI_WLANFW_MAX_DATA_SIZE_V01 6144

/* Tipo de blob en BDF_DOWNLOAD (`enum ath11k_bdf_type`, qmi.h:54). */
#define ATH11K_QMI_BDF_TYPE_BIN   0
#define ATH11K_QMI_BDF_TYPE_ELF   1
#define ATH11K_QMI_BDF_TYPE_REGDB 4

/* Índice de calibración por defecto (`QMI_WLANFW_CAL_TEMP_IDX_0_V01`). */
#define QMI_WLANFW_CAL_TEMP_IDX_0_V01 0

/* Modos de `WLAN_MODE` (enum ath11k_driver_mode). */
#define ATH11K_FIRMWARE_MODE_OFF     0
#define ATH11K_FIRMWARE_MODE_NORMAL  1

struct qmi_writer {
    unsigned char *buf;
    unsigned int cap;
    unsigned int len;
    /* Pegajoso: una vez desbordado, `finish` falla y no se envía nada a
     * medias. */
    int error;
};

struct qmi_msg_hdr {
    unsigned char type;
    uint16_t txn;
    uint16_t msg_id;
    uint16_t msg_len;
};

void ath11k_qmi_init(struct qmi_writer *w, unsigned char *buf, unsigned int cap);
void ath11k_qmi_put_u8(struct qmi_writer *w, unsigned char type, unsigned char v);
void ath11k_qmi_put_u32(struct qmi_writer *w, unsigned char type, uint32_t v);
void ath11k_qmi_put_u64(struct qmi_writer *w, unsigned char type, uint64_t v);
void ath11k_qmi_put_bytes(struct qmi_writer *w, unsigned char type,
                          const unsigned char *datos, unsigned int n);
int ath11k_qmi_finish(struct qmi_writer *w, unsigned char msg_type, uint16_t txn,
                      uint16_t msg_id);

int ath11k_qmi_parse_hdr(const unsigned char *buf, unsigned int len,
                         struct qmi_msg_hdr *out);
const unsigned char *ath11k_qmi_find_tlv(const unsigned char *buf, unsigned int len,
                                         unsigned char type, unsigned int *len_out);
int ath11k_qmi_parse_result(const unsigned char *buf, unsigned int len,
                            uint16_t *result, uint16_t *error);

int ath11k_qmi_build_ind_register(unsigned char *buf, unsigned int cap, uint16_t txn);
int ath11k_qmi_build_host_cap(unsigned char *buf, unsigned int cap, uint16_t txn,
                              unsigned char mem_cfg_mode, unsigned char cal_done);
int ath11k_qmi_build_cap_req(unsigned char *buf, unsigned int cap, uint16_t txn);
int ath11k_qmi_build_wlan_mode(unsigned char *buf, unsigned int cap, uint16_t txn,
                               uint32_t mode, int hw_debug);
int ath11k_qmi_build_bdf_download(unsigned char *buf, unsigned int cap, uint16_t txn,
                                  uint32_t total_size, uint32_t seg_id,
                                  const unsigned char *datos, unsigned int n,
                                  int end, unsigned char bdf_type);
int ath11k_qmi_build_m3_info(unsigned char *buf, unsigned int cap, uint16_t txn,
                             uint64_t addr, uint32_t size);

#endif /* ATH11K_QMI_H */
