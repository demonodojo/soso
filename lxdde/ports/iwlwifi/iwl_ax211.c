/*
 * Driver Intel AX211 (8086:7f70) y AX200 (8086:2723) para soso/lxdde.
 * Transporte PCIe gen2/gen3 + firmware TLV + scan/assoc/TX mínimos.
 */
#include "lx_emul.h"
#include "iwl_ax211.h"
#include "iwl_internal.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);
extern char *strncpy(char *dst, const char *src, unsigned long n);
extern int strncmp(const char *a, const char *b, unsigned long n);

struct iwl_ax211_priv g_iwl;

static void iwl_set_phase(struct iwl_ax211_priv *iwl, const char *p)
{
    strncpy(iwl->phase, p, sizeof(iwl->phase) - 1);
    iwl->phase[sizeof(iwl->phase) - 1] = '\0';
    lx_iwlwifi_set_phase(p);
}

static uint32_t iwl_read32(struct iwl_ax211_priv *iwl, uint32_t off)
{
    return iwl->mmio[off / 4];
}

static void mac_from_bdf(struct iwl_ax211_priv *iwl)
{
    if (!iwl->pdev)
        return;
    uint32_t bdf = lx_pci_bdf(iwl->pdev);
    iwl->mac[0] = 0x02;
    iwl->mac[1] = 0x00;
    iwl->mac[2] = 0x00;
    iwl->mac[3] = (uint8_t)(bdf >> 8);
    iwl->mac[4] = (uint8_t)(bdf & 0xff);
    iwl->mac[5] = 0;
}

static int iwl_try_ucode(struct iwl_ax211_priv *iwl, const char *name)
{
    const unsigned char *data = 0;
    unsigned long len = 0;
    if (lx_request_firmware(name, &data, &len) != 0)
        return -1;
    return iwl_fw_parse_tlv(iwl, data, len);
}

static int iwl_load_firmware_files(struct iwl_ax211_priv *iwl)
{
    const unsigned char *pnvm = 0;
    unsigned long pnvm_len = 0;

    if (!iwl->gen3) {
        static const char *const ax200[] = {
            "iwlwifi-cc-a0-77.ucode",
            "iwlwifi-cc-a0-74.ucode",
            "iwlwifi-cc-a0-73.ucode",
            "iwlwifi-cc-a0-72.ucode",
            "iwlwifi-cc-a0-66.ucode",
            0
        };
        for (int i = 0; ax200[i]; i++) {
            if (iwl_try_ucode(iwl, ax200[i]) == 0)
                return 0;
        }
        return -1;
    }

    if (iwl_try_ucode(iwl, "iwlwifi-so-a0-gf-a0-89.ucode") != 0 &&
        iwl_try_ucode(iwl, "iwlwifi-so-a0-gf-a0-77.ucode") != 0)
        return -1;

    if (lx_request_firmware("iwlwifi-so-a0-gf-a0.pnvm", &pnvm, &pnvm_len) == 0)
        iwl_fw_parse_pnvm(iwl, pnvm, pnvm_len);
    return 0;
}

static int iwl_ax211_probe(struct lx_pci_dev *pdev, const struct lx_pci_device_id *id)
{
    struct iwl_ax211_priv *iwl = &g_iwl;
    (void)id;
    memset(iwl, 0, sizeof(*iwl));
    iwl->pdev = pdev;
    iwl->device_id = lx_pci_device_id(pdev);
    iwl->gen3 = iwl->device_id != IWL_PCI_AX200;
    iwl->cmd_qid = IWL_MVM_DQA_CMD_QUEUE;

    if (lx_pci_enable_device(pdev) != 0)
        return -1;
    lx_pci_set_master(pdev);

    void *mmio = lx_pci_iomap(pdev, 0, 0x10000);
    if (!mmio)
        return -1;
    iwl->mmio = (volatile uint32_t *)mmio;
    lx_pci_set_drvdata(pdev, iwl);

    uint32_t rev = iwl_read32(iwl, CSR_HW_REV);
    lx_printk("iwlwifi: probe id=0x%x gen%s rev=0x%x bdf=0x%x\n",
              iwl->device_id, iwl->gen3 ? "3" : "2", rev, lx_pci_bdf(pdev));

    mac_from_bdf(iwl);
    iwl->probed = 1;
    iwl_set_phase(iwl, "probe");
    return 0;
}

static void iwl_ax211_remove(struct lx_pci_dev *pdev)
{
    (void)pdev;
    g_iwl.alive = 0;
    g_iwl.probed = 0;
    g_iwl.associated = 0;
    lx_iwlwifi_set_alive(0);
    iwl_set_phase(&g_iwl, "removed");
}

static struct lx_pci_device_id iwl_ax211_ids[] = {
    { 0x8086u, 0x7f70u, 0, 0, 0, 0, 0 },
    { 0x8086u, 0x51f0u, 0, 0, 0, 0, 0 },
    { 0x8086u, 0x54f0u, 0, 0, 0, 0, 0 },
    { 0x8086u, IWL_PCI_AX200, 0, 0, 0, 0, 0 },
    { 0, 0, 0, 0, 0, 0, 0 },
};

static int iwl_driver_registered;

int iwl_ax211_register(void)
{
    if (iwl_driver_registered)
        return 0;
    int r = lx_pci_register_driver("iwlwifi", iwl_ax211_ids, iwl_ax211_probe, iwl_ax211_remove);
    if (r == 0)
        iwl_driver_registered = 1;
    return r;
}

int iwl_ax211_probed(void)
{
    return g_iwl.probed;
}

int iwl_ax211_start_firmware(void)
{
    struct iwl_ax211_priv *iwl = &g_iwl;
    if (!iwl->probed || !iwl->mmio)
        return -1;
    if (iwl->alive)
        return 0;

    iwl_set_phase(iwl, "fw_load");
    if (iwl_load_firmware_files(iwl) != 0) {
        lx_printk("iwlwifi: firmware no encontrado\n");
        return -1;
    }

    iwl_set_phase(iwl, "fw_start");
    if ((iwl->gen3 ? iwl_trans_gen3_start(iwl) : iwl_trans_gen2_start(iwl)) != 0) {
        lx_printk("iwlwifi: arranque firmware falló\n");
        return -1;
    }
    iwl_set_phase(iwl, "alive");
    if (iwl_mvm_run_init(iwl) != 0) {
        lx_printk("iwlwifi: init MVM incompleto\n");
        return -1;
    }
    if (iwl_mvm_up_minimal(iwl) != 0) {
        lx_printk("iwlwifi: up MVM incompleto\n");
        return -1;
    }
    iwl_set_phase(iwl, "ready");
    return 0;
}

int iwl_ax211_alive(void)
{
    return g_iwl.alive;
}

const char *iwl_ax211_phase(void)
{
    return g_iwl.phase;
}

void iwl_ax211_poll(void)
{
    iwl_trans_poll(&g_iwl);
}

static int iwl_ax211_copy_scan(struct iwl_ax211_bss *out, int max, int *count)
{
    struct iwl_ax211_priv *iwl = &g_iwl;
    int n = iwl->scan_count;

    if (n > max)
        n = max;
    if (out && n > 0)
        memcpy(out, iwl->scan, (size_t)n * sizeof(struct iwl_ax211_bss));
    if (count)
        *count = n;
    return 0;
}

int iwl_ax211_scan(struct iwl_ax211_bss *out, int max, int *count)
{
    struct iwl_ax211_priv *iwl = &g_iwl;
    int rc;

    if (!iwl->alive)
        return -1;

    if (!out && !count) {
        iwl_set_phase(iwl, "scan");
        rc = iwl_mvm_scan(iwl);
        iwl_set_phase(iwl, rc == 0 ? "scan_done" : "scan_fail");
        return rc;
    }

    iwl_set_phase(iwl, "scan");
    rc = iwl_mvm_scan(iwl);
    iwl_set_phase(iwl, rc == 0 ? "scan_done" : "scan_fail");
    if (rc != 0 && iwl->scan_count == 0)
        return rc;
    return iwl_ax211_copy_scan(out, max, count);
}

int iwl_ax211_get_scan_results(struct iwl_ax211_bss *out, int max, int *count)
{
    if (!g_iwl.alive)
        return -1;
    return iwl_ax211_copy_scan(out, max, count);
}

int iwl_ax211_connect_open(const char *ssid)
{
    return iwl_mvm_connect_open(&g_iwl, ssid);
}

int iwl_ax211_connect_wpa2(const char *ssid, const uint8_t psk[32])
{
    return iwl_mvm_connect_wpa2(&g_iwl, ssid, psk);
}

int iwl_ax211_install_key(const uint8_t key[16], int key_idx)
{
    return iwl_mvm_install_key(&g_iwl, key, key_idx);
}

int iwl_ax211_connected(void)
{
    return g_iwl.associated;
}

int iwl_ax211_mac(uint8_t mac[6])
{
    if (!mac)
        return -1;
    memcpy(mac, g_iwl.mac, 6);
    return 0;
}

int iwl_ax211_bssid(uint8_t bssid[6])
{
    if (!bssid || !g_iwl.associated)
        return -1;
    memcpy(bssid, g_iwl.bssid, 6);
    return 0;
}

int iwl_ax211_rx(uint8_t *buf, int buflen)
{
    return iwl_mvm_rx_8023(&g_iwl, buf, buflen);
}

int iwl_ax211_tx(const uint8_t *buf, int len)
{
    return iwl_mvm_tx_8023(&g_iwl, buf, len);
}

void iwl_ax211_deliver_rx(const uint8_t *data, int len)
{
    struct iwl_ax211_priv *iwl = &g_iwl;
    if (!data || len <= 0 || len > 2040)
        return;
    int idx = iwl->rxq_head % 8;
    iwl->rxq[idx][0] = (uint8_t)(len & 0xff);
    iwl->rxq[idx][1] = (uint8_t)((len >> 8) & 0xff);
    memcpy(&iwl->rxq[idx][2], data, (size_t)len);
    iwl->rxq_head = (iwl->rxq_head + 1) % 8;
}

void iwl_ax211_add_bss(const struct iwl_ax211_bss *bss)
{
    struct iwl_ax211_priv *iwl = &g_iwl;
    if (!bss || iwl->scan_count >= IWL_AX211_MAX_SCAN)
        return;
    iwl->scan[iwl->scan_count++] = *bss;
}
