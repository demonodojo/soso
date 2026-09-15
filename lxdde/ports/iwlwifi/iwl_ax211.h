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
    int family;
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
    /* Fichero PNVM crudo (TLV); la sección se elige tras ALIVE por sku_id. */
    uint8_t *pnvm_file;
    unsigned long pnvm_file_len;
    uint32_t hw_rev;
    uint32_t hw_rf_id;
    int pnvm_published;
    unsigned pnvm_n_chunks;
    struct iwl_pnvm_dma_chunk {
        void *cpu;
        uint64_t dma;
        uint32_t len;
    } pnvm_chunks[IPC_DRAM_MAP_ENTRY_NUM_MAX];
    void *pnvm_desc_cpu;
    uint64_t pnvm_desc_dma;
    /* Modo continuo (sin capa 32): un solo blob DMA. */
    void *pnvm_cont_cpu;
    uint64_t pnvm_cont_dma;
    unsigned long pnvm_cont_len;
    /* Mapa DMA de las secciones ya subidas, para no reenviarlas al reiniciar. */
    struct iwl_context_info_dram dram_cache;
    const uint8_t *dram_fw_src;
    int dram_cached;

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
    void *hcmd_first_tb_cpu;
    uint64_t hcmd_first_tb_dma;
    void *scd_bc_cpu;
    uint64_t scd_bc_dma;
    uint32_t scd_base_addr;
    uint8_t tx_8000_ready;
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

    uint16_t mgmt_txq_id;
    uint16_t mgmt_txq_write;
    /* Consumidor de la cola TX: lo avanza la respuesta del firmware. Sin él,
     * `mgmt_txq_write` daba la vuelta cada 16 tramas y pisaba TFDs en vuelo. */
    uint16_t mgmt_txq_read;
    uint16_t data_txq_id;
    uint16_t data_txq_write;
    uint16_t data_txq_read;
    uint32_t tx_full_drop;
    uint8_t mgmt_txq_ready;
    uint8_t data_txq_ready;
    void *mgmt_tfd_cpu;
    uint64_t mgmt_tfd_dma;
    void *mgmt_first_tb_cpu;
    uint64_t mgmt_first_tb_dma;
    void *mgmt_body_cpu;
    uint64_t mgmt_body_dma;
    void *mgmt_bc_cpu;
    uint64_t mgmt_bc_dma;
    void *data_tfd_cpu;
    uint64_t data_tfd_dma;
    void *data_first_tb_cpu;
    uint64_t data_first_tb_dma;
    void *data_body_cpu;
    uint64_t data_body_dma;
    void *data_bc_cpu;
    uint64_t data_bc_dma;
    void *invalid_tx_cmd_cpu;
    uint64_t invalid_tx_cmd_dma;
    uint16_t invalid_tx_cmd_size;

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
    int pnvm_complete;
    uint32_t sku_id[3];
    int radio_ready;
    int mvm_up_done;
    int phy_ctxt_added;
    int mac_ctxt_added;
    int binding_added;
    uint8_t phy_channel;
    uint8_t phy_band;
    uint8_t scan_mac_id;
    uint8_t ap_sta_id;
    uint8_t mlme_auth_ok;
    uint8_t mlme_assoc_ok;
    uint8_t last_mgmt_tx_status;
    uint8_t auth_ctl_filter;
    uint8_t dtim_period;
    uint16_t beacon_int;
    uint16_t assoc_id;
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
    struct iwl_fw_cmd_version cmd_ver[IWL_CMD_VER_MAX];
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
    /* Descriptores completados con un VID que no designa ningún buffer. */
    uint32_t rx_vid_drop;
    /* MPDUs de datos descartadas: cifrado, direcciones o encapsulado. */
    uint32_t rx_data_drop;
    /* MPDUs de datos convertidas a Ethernet y entregadas. */
    uint32_t rx_data_ok;
    uint8_t last_rx_channel;
    int8_t last_rx_rssi;
    uint8_t last_rx_band24;

    uint8_t rxq[8][2048];
    int rxq_head;
    int rxq_tail;

    /* Cola propia para EAPOL. Si el supplicant y smoltcp comparten la de datos,
     * el primero que lee se lleva M1/M3 y la autenticación se queda colgada. */
    uint8_t eapolq[4][512];
    int eapolq_head;
    int eapolq_tail;

    /* Claves CCMP ya en el firmware: mientras valga 0 se transmite en claro y
     * se aceptan datos sin proteger (sólo el 4-way). */
    uint8_t keys_installed;
    /* Enlace utilizable para IP. Asociada no es autorizada. */
    uint8_t authorized;

    char phase[48];
};

extern struct iwl_ax211_priv g_iwl;

void lx_iwlwifi_set_alive(int alive);
void lx_iwlwifi_set_phase(const char *phase);

int iwl_ax211_register(void);
int iwl_ax211_start_firmware(void);
int iwl_ax211_probed(void);
int iwl_ax211_id_supported(uint16_t device_id);
int iwl_ax211_family_from_id(uint16_t device_id);
const char *iwl_ax211_family_name(int family);
const char *iwl_ax211_unclaimed_family(uint16_t device_id);
void iwl_ax211_log_unclaimed(uint16_t device_id);
void iwl_ax211_log_missing_devices(void);
void iwl_ax211_poll(void);
int iwl_ax211_alive(void);
const char *iwl_ax211_phase(void);

int iwl_ax211_scan(struct iwl_ax211_bss *out, int max, int *count);
int iwl_ax211_get_scan_results(struct iwl_ax211_bss *out, int max, int *count);
int iwl_ax211_connect_open(const char *ssid);
int iwl_ax211_connect_wpa2(const char *ssid, const uint8_t psk[32]);
int iwl_ax211_install_key(const uint8_t key[16], int key_idx);
int iwl_ax211_install_gtk(const uint8_t key[16], int key_idx, const uint8_t rsc[8]);
int iwl_ax211_connected(void);
int iwl_ax211_authorized(void);
void iwl_ax211_set_authorized(int authorized);
int iwl_ax211_rsn_ie(uint8_t *out, int max);
int iwl_ax211_rx(uint8_t *buf, int buflen);
int iwl_ax211_rx_eapol(uint8_t *buf, int buflen);
int iwl_ax211_tx(const uint8_t *buf, int len);
int iwl_ax211_mac(uint8_t mac[6]);
int iwl_ax211_bssid(uint8_t bssid[6]);

void iwl_ax211_deliver_rx(const uint8_t *data, int len);
void iwl_ax211_deliver_eapol(const uint8_t *data, int len);
void iwl_ax211_add_bss(const struct iwl_ax211_bss *bss);

int iwl_fw_parse_pnvm(struct iwl_ax211_priv *iwl, const uint8_t *pnvm, unsigned long pnvm_len);
int iwl_fw_upload_sections(struct iwl_ax211_priv *iwl, struct iwl_context_info_dram *dram);
int iwl_trans_send_cmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                       const void *payload, uint16_t pay_len);
int iwl_trans_send_cmd_async(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                             const void *payload, uint16_t pay_len);
int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t pay_len, int wait_ms);
int iwl_trans_txq_alloc_mgmt(struct iwl_ax211_priv *iwl, uint8_t sta_id);
int iwl_trans_txq_alloc_data(struct iwl_ax211_priv *iwl, uint8_t sta_id, uint8_t tid);
void iwl_trans_txq_drain_mgmt(struct iwl_ax211_priv *iwl);
void iwl_trans_txq_drain_data(struct iwl_ax211_priv *iwl);
int iwl_trans_wait_mgmt_tx_resp(struct iwl_ax211_priv *iwl, unsigned iters);
int iwl_trans_tx(struct iwl_ax211_priv *iwl, uint16_t txq_id,
                 const void *payload, uint16_t pay_len);
unsigned iwl_trans_tx_space(const struct iwl_ax211_priv *iwl);
void iwl_trans_tx_reclaim(struct iwl_ax211_priv *iwl, uint16_t seq);

#endif
