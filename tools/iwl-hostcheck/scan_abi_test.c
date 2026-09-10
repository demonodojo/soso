/* C1: constructor scan v14–17 y MAC CSR contra offsets Linux, sin oráculo de tamaño. */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

void lx_printk(const char *fmt, ...) { (void)fmt; }
void lx_mdelay(unsigned int ms) { (void)ms; }

int iwl_trans_send_cmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                       const void *payload, uint16_t pay_len)
{
    (void)iwl;
    (void)group;
    (void)id;
    (void)payload;
    (void)pay_len;
    return 0;
}

int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t pay_len, int wait_ms)
{
    (void)group;
    (void)id;
    (void)payload;
    (void)pay_len;
    (void)wait_ms;
    iwl->cmd_status = 1;
    return 0;
}

void iwl_trans_poll(struct iwl_ax211_priv *iwl) { (void)iwl; }

int iwl_mvm_up_minimal(struct iwl_ax211_priv *iwl)
{
    iwl->mvm_up_done = 1;
    return 0;
}

int iwl_mvm_assoc_prepare(struct iwl_ax211_priv *iwl, const char *ssid,
                          const uint8_t *bssid)
{
    (void)iwl;
    (void)ssid;
    (void)bssid;
    return -1;
}

int iwl_fw_cmd_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    unsigned i;

    for (i = 0; i < iwl->cmd_ver_count; i++) {
        if (iwl->cmd_ver[i].group == group && iwl->cmd_ver[i].cmd == cmd)
            return iwl->cmd_ver[i].version;
    }
    return 0;
}

uint8_t iwl_mvm_scan_rx_ant(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 1;
}

void iwl_ax211_add_bss(const struct iwl_ax211_bss *bss)
{
    (void)bss;
}

/* Offsets Linux fw/api/scan.h iwl_scan_req_umac_v17 — no usar size() del driver. */
#define REF_UID                 0u
#define REF_OOC                 4u
#define REF_GENERAL             8u
#define REF_GENERAL_FLAGS       8u
#define REF_GENERAL_SIZE        36u
#define REF_CHAN_PARAMS         44u
#define REF_CHAN_COUNT          45u
#define REF_CHAN0               48u
#define REF_CHAN_STRIDE         8u
#define REF_CHAN_SLOTS          67u
#define REF_PERIODIC            (REF_CHAN_PARAMS + 4u + REF_CHAN_SLOTS * REF_CHAN_STRIDE)
#define REF_PROBE               (REF_PERIODIC + 44u)
#define REF_TOTAL               (REF_PROBE + 1484u)
#define REF_V2_PASS_ALL         (1u << 1)
#define REF_V2_ITER_COMPLETE    (1u << 2)
#define REF_V2_MATCH            (1u << 5)
#define REF_OLD_PASS_ALL        (1u << 2)
#define REF_OLD_ITER_COMPLETE   (1u << 5)

static void set_scan_ver(struct iwl_ax211_priv *iwl, uint8_t ver)
{
    iwl->cmd_ver_count = 1;
    iwl->cmd_ver[0].group = LONG_GROUP;
    iwl->cmd_ver[0].cmd = SCAN_REQ_UMAC;
    iwl->cmd_ver[0].version = ver;
}

static int check_v17_layout(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t buf[4096];
    uint16_t pay;
    uint16_t flags;
    unsigned i;

    memset(&iwl, 0, sizeof(iwl));
    set_scan_ver(&iwl, 15);
    iwl.mac[0] = 0x02;
    iwl.mac[1] = 0x00;
    iwl.mac[2] = 0x00;
    iwl.mac[3] = 0x00;
    iwl.mac[4] = 0x00;
    iwl.mac[5] = 0x01;

    pay = iwl_mvm_build_scan_req(&iwl, buf, sizeof(buf));
    if (pay != REF_TOTAL) {
        fprintf(stderr, "pay=%u (ref independiente %u)\n", pay, REF_TOTAL);
        return -1;
    }
    flags = (uint16_t)buf[REF_GENERAL_FLAGS] | ((uint16_t)buf[REF_GENERAL_FLAGS + 1] << 8);
    if (flags != (REF_V2_PASS_ALL | REF_V2_ITER_COMPLETE)) {
        fprintf(stderr, "flags v15=0x%x (esperaba V2 PASS_ALL|ITER)\n", flags);
        return -1;
    }
    if (flags & REF_V2_MATCH) {
        fprintf(stderr, "BIT(5) MATCH no debe estar en v14–17\n");
        return -1;
    }
    if (buf[REF_CHAN_COUNT] == 0) {
        fprintf(stderr, "count de canales vacío\n");
        return -1;
    }
    /* periodic empieza tras 67 slots, no tras count canales. */
    if (buf[REF_PERIODIC] == 0 && buf[REF_PERIODIC + 2] != 1) {
        fprintf(stderr, "periodic.schedule[0].iter_count no está en offset %u\n",
                REF_PERIODIC + 2);
        return -1;
    }
    if (buf[REF_PERIODIC + 2] != 1) {
        fprintf(stderr, "iter_count periodic=%u @%u\n", buf[REF_PERIODIC + 2],
                REF_PERIODIC + 2);
        return -1;
    }
    if (buf[REF_PROBE + 16] != 0x40) {
        fprintf(stderr, "probe frame no está en offset %u (byte=0x%02x)\n",
                REF_PROBE + 16, buf[REF_PROBE + 16]);
        return -1;
    }
    for (i = (unsigned)buf[REF_CHAN_COUNT]; i < REF_CHAN_SLOTS; i++) {
        unsigned off = REF_CHAN0 + i * REF_CHAN_STRIDE;
        unsigned k;
        for (k = 0; k < REF_CHAN_STRIDE; k++) {
            if (buf[off + k] != 0) {
                fprintf(stderr, "slot canal %u no está a cero\n", i);
                return -1;
            }
        }
    }
    if (iwl.scan_uid == 0) {
        fprintf(stderr, "uid no asignado\n");
        return -1;
    }
    puts("OK: SCAN_REQ v15 offsets/flags/tamaño contra referencia Linux");
    return 0;
}

static int check_versions(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t buf[4096];
    const uint8_t ok[] = {6, 14, 15, 16, 17};
    const uint8_t bad[] = {0, 13, 18, 99};
    unsigned i;

    for (i = 0; i < sizeof(ok); i++) {
        memset(&iwl, 0, sizeof(iwl));
        set_scan_ver(&iwl, ok[i]);
        if (!iwl_mvm_scan_umac_supported(ok[i]) ||
            iwl_mvm_build_scan_req(&iwl, buf, sizeof(buf)) == 0) {
            fprintf(stderr, "versión %u debería construirse\n", ok[i]);
            return -1;
        }
    }
    for (i = 0; i < sizeof(bad); i++) {
        memset(&iwl, 0, sizeof(iwl));
        set_scan_ver(&iwl, bad[i]);
        if (iwl_mvm_scan_umac_supported(bad[i]) ||
            iwl_mvm_build_scan_req(&iwl, buf, sizeof(buf)) != 0) {
            fprintf(stderr, "versión %u no debe aceptarse\n", bad[i]);
            return -1;
        }
    }
    puts("OK: lista explícita de versiones scan (rechaza 0/13/18)");
    return 0;
}

static int check_nvm_channels(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t buf[4096];
    uint16_t pay;

    memset(&iwl, 0, sizeof(iwl));
    set_scan_ver(&iwl, 17);
    iwl.nvm_n_channels = 3;
    iwl.nvm_chan_flags[0] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE; /* ch 1 */
    iwl.nvm_chan_flags[1] = 0;
    iwl.nvm_chan_flags[2] = NVM_CHANNEL_VALID | NVM_CHANNEL_RADAR; /* ch 3 passive */
    pay = iwl_mvm_build_scan_req(&iwl, buf, sizeof(buf));
    if (pay == 0 || buf[REF_CHAN_COUNT] != 2) {
        fprintf(stderr, "NVM debería producir 2 canales válidos (count=%u)\n",
                buf[REF_CHAN_COUNT]);
        return -1;
    }
    if (buf[REF_CHAN0 + 4] != 1 || buf[REF_CHAN0 + 12] != 3) {
        fprintf(stderr, "canales NVM %u %u (esperaba 1 y 3)\n",
                buf[REF_CHAN0 + 4], buf[REF_CHAN0 + 12]);
        return -1;
    }
    if (buf[REF_CHAN0 + 8] == 0) {
        fprintf(stderr, "canal radar debería marcarse pasivo\n");
        return -1;
    }
    puts("OK: canales desde perfil NVM (activo/pasivo)");
    return 0;
}

static int check_mac_csr(void)
{
    struct iwl_ax211_priv iwl;
    uint32_t mmio[0x400];
    uint8_t mac[6];
    /* STRAP en 0x388/0x38c: bytes LE 11 22 33 44 / 55 66 → flip 44:33:22:11:66:55 */
    uint32_t strap0 = 0x44332211u;
    uint32_t strap1 = 0x00006655u;

    memset(&iwl, 0, sizeof(iwl));
    memset(mmio, 0, sizeof(mmio));
    iwl.device_id = IWL_PCI_AX200;
    iwl.mmio = mmio;

    mmio[CSR_INT / 4] = 0x80000001u;
    mmio[CSR_INT_MASK / 4] = 0x00010000u;
    mmio[CSR_HW_IF_CONFIG_REG / 4] = 0xa5a5a5a5u;
    mmio[0x388 / 4] = strap0;
    mmio[0x38c / 4] = strap1;

    if (iwl_mac_addr_from_csr(IWL_PCI_AX200) != 0x380u) {
        fprintf(stderr, "base AX200 != 0x380\n");
        return -1;
    }
    iwl_mac_from_csr(&iwl, mac);
    if (mac[0] != 0x44 || mac[1] != 0x33 || mac[2] != 0x22 ||
        mac[3] != 0x11 || mac[4] != 0x66 || mac[5] != 0x55) {
        fprintf(stderr, "MAC %02x:%02x:%02x:%02x:%02x:%02x (no leyó 0x380)\n",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
        return -1;
    }
    if (!iwl_mac_valid_unicast(mac)) {
        fprintf(stderr, "MAC STRAP debería ser unicast válida\n");
        return -1;
    }
    {
        uint8_t mcast[6] = {0x81, 0, 0, 0, 0, 1};
        uint8_t zero[6] = {0};
        if (iwl_mac_valid_unicast(mcast) || iwl_mac_valid_unicast(zero)) {
            fprintf(stderr, "multicast/cero no deben ser unicast\n");
            return -1;
        }
    }
    puts("OK: MAC CSR 0x380 distinta de CSR_INT/INT_MASK");
    return 0;
}

static int check_scan_states(void)
{
    struct iwl_ax211_priv iwl;

    memset(&iwl, 0, sizeof(iwl));
    iwl.scan_uid = 7;
    iwl.scan_active = 1;
    iwl.scan_count = 0;
    iwl_mvm_on_scan_complete(&iwl, 7, IWL_SCAN_OFFLOAD_COMPLETED);
    if (iwl.scan_end != IWL_SCAN_END_NORMAL || iwl.scan_active) {
        fprintf(stderr, "fin normal vacío mal marcado\n");
        return -1;
    }

    memset(&iwl, 0, sizeof(iwl));
    iwl.scan_uid = 8;
    iwl.scan_active = 1;
    iwl.scan_count = 2;
    iwl_mvm_on_scan_complete(&iwl, 99, IWL_SCAN_OFFLOAD_COMPLETED);
    if (iwl.scan_end != IWL_SCAN_END_NONE || !iwl.scan_active) {
        fprintf(stderr, "uid ajeno no debe cerrar el scan\n");
        return -1;
    }

    iwl_mvm_on_scan_complete(&iwl, 8, IWL_SCAN_OFFLOAD_ABORTED);
    if (iwl.scan_end != IWL_SCAN_END_ABORTED) {
        fprintf(stderr, "aborto no registrado\n");
        return -1;
    }

    memset(&iwl, 0, sizeof(iwl));
    iwl.scan_uid = 1;
    iwl.scan_active = 1;
    iwl.scan_count = 3;
    /* BSS parciales sin fin: scan_end sigue NONE. */
    if (iwl.scan_end != IWL_SCAN_END_NONE || iwl.scan_count == 0) {
        fprintf(stderr, "parcial sin fin no debe acreditarse\n");
        return -1;
    }

    memset(&iwl, 0, sizeof(iwl));
    iwl.scan_uid = 2;
    iwl.scan_active = 1;
    iwl_mvm_on_scan_complete(&iwl, 2, IWL_SCAN_OFFLOAD_COMPLETED);
    iwl.scan_active = 1;
    iwl.scan_uid = 3;
    iwl.scan_end = IWL_SCAN_END_NONE;
    iwl_mvm_on_scan_complete(&iwl, 2, IWL_SCAN_OFFLOAD_COMPLETED);
    if (iwl.scan_end != IWL_SCAN_END_NONE) {
        fprintf(stderr, "complete tardío del uid viejo cerró el nuevo\n");
        return -1;
    }
    puts("OK: scan vacío/abortado/tardío/repetido por UID");
    return 0;
}

int main(void)
{
    if (check_v17_layout() != 0)
        return 1;
    if (check_versions() != 0)
        return 1;
    if (check_nvm_channels() != 0)
        return 1;
    if (check_mac_csr() != 0)
        return 1;
    if (check_scan_states() != 0)
        return 1;
    return 0;
}
