#ifndef IWL_AX211_H
#define IWL_AX211_H

#include <stdint.h>
#include <stddef.h>
#include "iwl_internal.h"

#define IWL_AX211_MAX_SCAN 32
#define IWL_AX211_SSID_MAX 32

struct iwl_ax211_tfd {
    uint64_t addr;
    uint16_t num_bytes;
    uint8_t reserved[6];
} __attribute__((packed));

struct lx_pci_dev;

struct iwl_ax211_priv {
    struct lx_pci_dev *pdev;
    volatile uint32_t *mmio;
    uint16_t device_id;
    int gen3;
    uint8_t mac[6];
    int alive;
    int probed;
    int associated;
    char ssid[IWL_AX211_SSID_MAX + 1];
    uint8_t bssid[6];
    uint8_t channel;

    struct iwl_fw_image fw;
    const uint8_t *iml;
    unsigned long iml_len;
    const uint8_t *pnvm_data;
    unsigned long pnvm_len;
    /* Mapa DMA de las secciones ya subidas, para no reenviarlas al reiniciar. */
    struct iwl_context_info_dram dram_cache;
    const uint8_t *dram_fw_src;
    int dram_cached;
    /* PNVM ya copiado a DMA. */
    void *pnvm_cpu;
    uint64_t pnvm_dma;

    uint64_t scratch_dma;
    uint64_t info_dma;
    uint64_t ctxt_dma;
    uint64_t mtr_dma;
    uint64_t mcr_dma;
    uint64_t cr_head;
    uint64_t cr_tail;
    uint64_t tr_head;
    uint64_t tr_tail;
    void *mtr_cpu;
    void *mcr_cpu;
    /* Buffers del context-info, reutilizados al reiniciar el transporte. */
    void *scratch_cpu;
    void *info_cpu;
    void *ctxt_cpu;
    void *iml_cpu;
    uint64_t iml_dma;
    unsigned long iml_cpu_len;

    void *rx_bd_cpu;
    void *used_bd_cpu;
    void *rx_page_cpu;
    volatile uint16_t *rb_stts;
    uint64_t rx_bd_dma;
    uint64_t used_bd_dma;
    uint64_t rx_page_dma;
    uint64_t rb_stts_dma;
    uint16_t rx_read;
    uint16_t rx_write;
    uint16_t cmd_write;
    uint16_t cmd_qid;

    struct iwl_ax211_tfd tfd[256];
    uint16_t mtr_write;
    uint16_t mcr_read;
    uint16_t mcr_write;
    uint16_t cmd_seq;

    struct iwl_ax211_bss scan[IWL_AX211_MAX_SCAN];
    int scan_count;
    int scan_active;
    int scan_complete;
    uint32_t scan_uid;
    int scan_end;
    uint8_t nvm_ready;
    uint32_t nvm_n_channels;
    uint32_t nvm_chan_flags[IWL_NUM_CHANNELS];
    int scan_cfg_sent;
    int init_complete;
    int radio_ready;
    int mvm_up_done;
    int phy_ctxt_added;
    uint8_t scan_rx_ant;
    uint32_t phy_sku;
    uint8_t fw_valid_tx_ant;
    uint8_t fw_valid_rx_ant;
    uint8_t valid_tx_ant;
    uint8_t valid_rx_ant;
    uint8_t lar_enabled;
    uint8_t mcc_done;
    /* Perfil regulatorio realmente aplicado (no solo «MCC contestó»). */
    uint8_t lar_regdom_set;
    uint16_t mcc_applied;
    uint32_t mcc_status;
    /* Origen de la última lista de canales: IWL_CHAN_SRC_*. */
    uint8_t chan_src;
    uint8_t scan_passive_only;
    uint8_t n_scan_channels;
    struct iwl_fw_cmd_version cmd_ver[64];
    unsigned cmd_ver_count;
    uint32_t fw_capa[IWL_FW_CAPA_SETS];
    int cmd_pending;
    uint16_t cmd_pending_seq;
    uint8_t cmd_pending_group;
    uint8_t cmd_pending_id;
    /* Propiedad de slots de la cola de comandos (R4). El anillo es FIFO
     * estricto: [cmd_read, cmd_write) son los slots en vuelo y ninguno se
     * reutiliza hasta que el FW responde o se reinicia el transporte. */
    uint8_t cmd_slot_state[IWL_CMD_QUEUE_SIZE];
    uint8_t cmd_slot_group[IWL_CMD_QUEUE_SIZE];
    uint8_t cmd_slot_id[IWL_CMD_QUEUE_SIZE];
    uint16_t cmd_slot_seq[IWL_CMD_QUEUE_SIZE];
    uint16_t cmd_read;
    uint16_t cmd_poisoned;
    uint32_t cmd_backpressure;
    uint32_t cmd_reentry_reject;
    uint32_t cmd_recover;
    uint8_t cmd_needs_recover;
    /* Guardia de reentrada: envío/RX/poll tienen un único propietario. */
    uint8_t in_trans;
    int cmd_status;
    int cmd_fw_err;
    uint8_t cmd_resp[512];
    uint16_t cmd_resp_len;
    /* Longitud anunciada por el FW antes de recortar a cmd_resp[]. */
    uint16_t cmd_resp_wire_len;
    uint8_t cmd_resp_trunc;
    /* Paquetes RX descartados por anunciar más bytes de los recibidos. */
    uint32_t rx_trunc_drop;
    uint8_t last_rx_channel;
    int8_t last_rx_rssi;
    uint8_t last_rx_band24;

    uint8_t rxq[8][2048];
    int rxq_head;
    int rxq_tail;

    char phase[48];
};

extern struct iwl_ax211_priv g_iwl;

void lx_iwlwifi_set_alive(int alive);
void lx_iwlwifi_set_phase(const char *phase);

int iwl_ax211_register(void);
int iwl_ax211_start_firmware(void);
int iwl_ax211_probed(void);
void iwl_ax211_poll(void);
int iwl_ax211_alive(void);
const char *iwl_ax211_phase(void);

int iwl_ax211_scan(struct iwl_ax211_bss *out, int max, int *count);
int iwl_ax211_get_scan_results(struct iwl_ax211_bss *out, int max, int *count);
int iwl_ax211_connect_open(const char *ssid);
int iwl_ax211_connect_wpa2(const char *ssid, const uint8_t psk[32]);
int iwl_ax211_install_key(const uint8_t key[16], int key_idx);
int iwl_ax211_connected(void);
int iwl_ax211_rx(uint8_t *buf, int buflen);
int iwl_ax211_tx(const uint8_t *buf, int len);
int iwl_ax211_mac(uint8_t mac[6]);
int iwl_ax211_bssid(uint8_t bssid[6]);

void iwl_ax211_deliver_rx(const uint8_t *data, int len);
void iwl_ax211_add_bss(const struct iwl_ax211_bss *bss);

int iwl_fw_parse_pnvm(struct iwl_ax211_priv *iwl, const uint8_t *pnvm, unsigned long pnvm_len);
int iwl_fw_upload_sections(struct iwl_ax211_priv *iwl, struct iwl_context_info_dram *dram);
int iwl_trans_send_cmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                       const void *payload, uint16_t pay_len);
int iwl_trans_send_cmd_async(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                             const void *payload, uint16_t pay_len);
int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t pay_len, int wait_ms);

#endif
