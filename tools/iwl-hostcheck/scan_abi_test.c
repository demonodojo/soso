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
    /* Igual que iwl_fw.c: grp 0 → LONG_GROUP (DEF_ID). */
    uint8_t grp = group;

    if (group == LEGACY_GROUP)
        grp = LONG_GROUP;
    for (i = 0; i < iwl->cmd_ver_count; i++) {
        if (iwl->cmd_ver[i].group == grp && iwl->cmd_ver[i].cmd == cmd)
            return iwl->cmd_ver[i].version;
    }
    return 0;
}

int iwl_fw_notif_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    unsigned i;

    for (i = 0; i < iwl->cmd_ver_count; i++) {
        if (iwl->cmd_ver[i].group == group && iwl->cmd_ver[i].cmd == cmd)
            return iwl->cmd_ver[i].notif_version;
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
#define REF_PERIODIC            584u
#define REF_PROBE               596u
#define REF_PROBE_FRAME         (REF_PROBE + 20u)
#define REF_SCHED1_ITER         (REF_PERIODIC + 6u)
#define REF_TOTAL               1940u
#define REF_V2_PASS_ALL         (1u << 1)
#define REF_V2_ITER_COMPLETE    (1u << 2)
#define REF_V2_ADAPTIVE_DWELL   (1u << 7)
#define REF_V2_FORCE_PASSIVE    (1u << 11)
#define REF_V2_MATCH            (1u << 5)
#define REF_NAPS0               (REF_CHAN_PARAMS + 2u)
#define REF_NAPS1               (REF_CHAN_PARAMS + 3u)
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
    /* Perfil con canales activos: sin él la lista es el fallback pasivo y el
     * scan se declara FORCE_PASSIVE (R6). */
    iwl.nvm_n_channels = 4;
    iwl.nvm_chan_flags[0] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
    iwl.nvm_chan_flags[1] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
    iwl.nvm_chan_flags[2] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
    iwl.nvm_chan_flags[3] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
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
    if (buf[REF_OOC] != 6 || buf[REF_OOC + 1] != 0 || buf[REF_OOC + 2] != 0 ||
        buf[REF_OOC + 3] != 0) {
        fprintf(stderr, "ooc_priority LE (esperaba EXT_6=6)\n");
        return -1;
    }
    flags = (uint16_t)buf[REF_GENERAL_FLAGS] | ((uint16_t)buf[REF_GENERAL_FLAGS + 1] << 8);
    if (flags != (REF_V2_PASS_ALL | REF_V2_ITER_COMPLETE | REF_V2_ADAPTIVE_DWELL |
                  REF_V2_FORCE_PASSIVE)) {
        fprintf(stderr, "flags v15=0x%x (esperaba PASS_ALL|ITER|ADWELL|FORCE_PASSIVE=0x%x)\n",
                flags,
                REF_V2_PASS_ALL | REF_V2_ITER_COMPLETE | REF_V2_ADAPTIVE_DWELL |
                    REF_V2_FORCE_PASSIVE);
        return -1;
    }
    if (buf[REF_NAPS0] != 10 || buf[REF_NAPS1] != 2) {
        fprintf(stderr, "n_aps_override=%u/%u (esperaba 10/2)\n",
                buf[REF_NAPS0], buf[REF_NAPS1]);
        return -1;
    }
    if (buf[REF_GENERAL + 4] != 10 || buf[REF_GENERAL + 5] != 10) {
        fprintf(stderr, "active_dwell=%u/%u (esperaba 10/10)\n",
                buf[REF_GENERAL + 4], buf[REF_GENERAL + 5]);
        return -1;
    }
    if (buf[REF_GENERAL + 6] != 2 || buf[REF_GENERAL + 7] != 8 ||
        buf[REF_GENERAL + 8] != 10) {
        fprintf(stderr, "adwell 2g/5g/social=%u/%u/%u (esperaba 2/8/10)\n",
                buf[REF_GENERAL + 6], buf[REF_GENERAL + 7], buf[REF_GENERAL + 8]);
        return -1;
    }
    if (buf[REF_GENERAL + 10] != 44 || buf[REF_GENERAL + 11] != 1) {
        fprintf(stderr, "adwell_max_budget=0x%02x%02x (esperaba 300 LE)\n",
                buf[REF_GENERAL + 10], buf[REF_GENERAL + 11]);
        return -1;
    }
    if (buf[REF_GENERAL + 28] != 6 || buf[REF_GENERAL + 29] != 0 ||
        buf[REF_GENERAL + 30] != 0 || buf[REF_GENERAL + 31] != 0) {
        fprintf(stderr, "scan_priority LE (esperaba EXT_6=6)\n");
        return -1;
    }
    if (buf[REF_GENERAL + 32] != 110 || buf[REF_GENERAL + 33] != 110) {
        fprintf(stderr, "passive_dwell=%u/%u (esperaba 110/110)\n",
                buf[REF_GENERAL + 32], buf[REF_GENERAL + 33]);
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
    if (buf[REF_SCHED1_ITER] != 0) {
        fprintf(stderr, "schedule[1].iter_count=%u (esperaba 0)\n",
                buf[REF_SCHED1_ITER]);
        return -1;
    }
    if (buf[REF_PROBE_FRAME] != 0x40) {
        fprintf(stderr, "probe frame no está en offset %u (byte=0x%02x)\n",
                REF_PROBE_FRAME, buf[REF_PROBE_FRAME]);
        return -1;
    }
    for (i = 0; i < (unsigned)buf[REF_CHAN_COUNT]; i++) {
        uint32_t cf = (uint32_t)buf[REF_CHAN0 + i * REF_CHAN_STRIDE] |
                      ((uint32_t)buf[REF_CHAN0 + i * REF_CHAN_STRIDE + 1] << 8) |
                      ((uint32_t)buf[REF_CHAN0 + i * REF_CHAN_STRIDE + 2] << 16) |
                      ((uint32_t)buf[REF_CHAN0 + i * REF_CHAN_STRIDE + 3] << 24);
        if (cf & IWL_UHB_CHAN_CFG_FLAG_FORCE_PASSIVE) {
            fprintf(stderr, "canal %u lleva bit 26 FORCE_PASSIVE (0x%08x)\n", i, cf);
            return -1;
        }
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
        iwl.nvm_n_channels = 1;
        iwl.nvm_chan_flags[0] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
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

static int check_ext_nvm_table(void)
{
    unsigned table_n = 0;
    const uint8_t *tbl = iwl_mvm_nvm_chan_table(51, &table_n);
    static const uint8_t expect[] = {
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
        36, 40, 44, 48, 52, 56, 60, 64, 68, 72, 76, 80, 84, 88, 92,
        96, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144,
        149, 153, 157, 161, 165, 169, 173, 177, 181,
    };
    unsigned i;

    if (table_n != 51) {
        fprintf(stderr, "tabla ext n=%u (esperaba 51)\n", table_n);
        return -1;
    }
    for (i = 0; i < table_n; i++) {
        if (tbl[i] != expect[i]) {
            fprintf(stderr, "ext[%u]=%u (esperaba %u)\n", i, tbl[i], expect[i]);
            return -1;
        }
    }
    if (tbl[22] != 68 || tbl[29] != 96) {
        fprintf(stderr, "ext idx22=%u idx29=%u (esperaba 68 y 96)\n",
                tbl[22], tbl[29]);
        return -1;
    }
    for (i = 0; i < table_n; i++) {
        if (tbl[i] >= 183) {
            fprintf(stderr, "tabla ext no debe incluir ch %u (idx %u)\n", tbl[i], i);
            return -1;
        }
    }
    if (iwl_mvm_phy_band_from_channel_idx(0, 51) != PHY_BAND_24 ||
        iwl_mvm_phy_band_from_channel_idx(13, 51) != PHY_BAND_24 ||
        iwl_mvm_phy_band_from_channel_idx(14, 51) != PHY_BAND_5 ||
        iwl_mvm_phy_band_from_channel_idx(50, 51) != PHY_BAND_5) {
        fprintf(stderr, "banda por índice NVM incorrecta en tabla ext\n");
        return -1;
    }
    puts("OK: iwl_ext_nvm_channels (51 entradas, 68/96, sin 183+)");
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
    /* v14–17: pasivo global (FORCE_PASSIVE en general), no bit 26 por canal. */
    {
        uint16_t gflags = (uint16_t)buf[REF_GENERAL_FLAGS] |
                          ((uint16_t)buf[REF_GENERAL_FLAGS + 1] << 8);
        unsigned c;

        if (!(gflags & REF_V2_FORCE_PASSIVE)) {
            fprintf(stderr, "scan sin SSIDs debe llevar FORCE_PASSIVE global (0x%x)\n",
                    gflags);
            return -1;
        }
        for (c = 0; c < (unsigned)buf[REF_CHAN_COUNT]; c++) {
            uint32_t cf = (uint32_t)buf[REF_CHAN0 + c * REF_CHAN_STRIDE] |
                          ((uint32_t)buf[REF_CHAN0 + c * REF_CHAN_STRIDE + 1] << 8) |
                          ((uint32_t)buf[REF_CHAN0 + c * REF_CHAN_STRIDE + 2] << 16) |
                          ((uint32_t)buf[REF_CHAN0 + c * REF_CHAN_STRIDE + 3] << 24);
            if (cf & IWL_UHB_CHAN_CFG_FLAG_FORCE_PASSIVE) {
                fprintf(stderr, "canal %u no debe llevar bit 26 (0x%08x)\n", c, cf);
                return -1;
            }
            if (cf & 1u) {
                fprintf(stderr, "bit 0 (SSID directo) usado como pasivo en canal %u\n", c);
                return -1;
            }
        }
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

/* Linux phy-ctxt.h: PHY_BAND_24=1, PHY_BAND_5=0 — no invertir 2.4/5 GHz. */
static int check_phy_bands(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t buf[4096];
    uint16_t pay;
    uint32_t flags0;
    unsigned off36;

    memset(&iwl, 0, sizeof(iwl));
    set_scan_ver(&iwl, 17);
    iwl.nvm_n_channels = 15;
    iwl.nvm_chan_flags[0] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;  /* ch 1 */
    iwl.nvm_chan_flags[14] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE; /* ch 36 */
    pay = iwl_mvm_build_scan_req(&iwl, buf, sizeof(buf));
    if (pay == 0 || buf[REF_CHAN_COUNT] != 2) {
        fprintf(stderr, "bandas: count=%u (esperaba 2)\n", buf[REF_CHAN_COUNT]);
        return -1;
    }
    if (buf[REF_CHAN_PARAMS] != IWL_SCAN_CHANNEL_FLAG_ENABLE_CHAN_ORDER) {
        fprintf(stderr, "channel_params.flags=0x%02x (esperaba ENABLE_CHAN_ORDER=0x%x)\n",
                buf[REF_CHAN_PARAMS], IWL_SCAN_CHANNEL_FLAG_ENABLE_CHAN_ORDER);
        return -1;
    }
    if (buf[REF_CHAN0 + 4] != 1) {
        fprintf(stderr, "primer canal=%u (esperaba 1)\n", buf[REF_CHAN0 + 4]);
        return -1;
    }
    flags0 = (uint32_t)buf[REF_CHAN0] | ((uint32_t)buf[REF_CHAN0 + 1] << 8) |
             ((uint32_t)buf[REF_CHAN0 + 2] << 16) | ((uint32_t)buf[REF_CHAN0 + 3] << 24);
    if (((flags0 >> IWL_CHAN_CFG_FLAGS_BAND_POS) & 3u) != PHY_BAND_24) {
        fprintf(stderr, "ch1 banda en flags=0x%x (esperaba PHY_BAND_24=%u)\n",
                (unsigned)((flags0 >> IWL_CHAN_CFG_FLAGS_BAND_POS) & 3u), PHY_BAND_24);
        return -1;
    }
    off36 = REF_CHAN0 + REF_CHAN_STRIDE;
    if (buf[off36 + 4] != 36) {
        fprintf(stderr, "segundo canal=%u (esperaba 36)\n", buf[off36 + 4]);
        return -1;
    }
    flags0 = (uint32_t)buf[off36] | ((uint32_t)buf[off36 + 1] << 8) |
             ((uint32_t)buf[off36 + 2] << 16) | ((uint32_t)buf[off36 + 3] << 24);
    if (((flags0 >> IWL_CHAN_CFG_FLAGS_BAND_POS) & 3u) != PHY_BAND_5) {
        fprintf(stderr, "ch36 banda en flags=0x%x (esperaba PHY_BAND_5=%u)\n",
                (unsigned)((flags0 >> IWL_CHAN_CFG_FLAGS_BAND_POS) & 3u), PHY_BAND_5);
        return -1;
    }
    puts("OK: bandas SCAN_REQ PHY_BAND_24/5 y ENABLE_CHAN_ORDER");
    return 0;
}

static int check_scan_cfg_bcast_ax200(void)
{
    struct iwl_ax211_priv iwl;

    memset(&iwl, 0, sizeof(iwl));
    /* cc-a0-77: ADD_STA v12 en LONG_GROUP (grp=1), no en MAC_CONF (grp=3). */
    iwl.cmd_ver_count = 2;
    iwl.cmd_ver[0].group = LONG_GROUP;
    iwl.cmd_ver[0].cmd = SCAN_CFG_CMD;
    iwl.cmd_ver[0].version = 5;
    iwl.cmd_ver[1].group = LONG_GROUP;
    iwl.cmd_ver[1].cmd = ADD_STA;
    iwl.cmd_ver[1].version = 12;
    iwl.fw_valid_tx_ant = 0x3;
    iwl.fw_valid_rx_ant = 0x3;

    if (iwl_fw_cmd_ver(&iwl, LEGACY_GROUP, ADD_STA) < 12) {
        fprintf(stderr, "ADD_STA v12 vía LEGACY→LONG debería ser API nueva\n");
        return -1;
    }
    if (iwl_mvm_send_scan_cfg(&iwl) != 0) {
        fprintf(stderr, "iwl_mvm_send_scan_cfg falló con ADD_STA v12\n");
        return -1;
    }
    if (!iwl.scan_cfg_sent) {
        fprintf(stderr, "scan_cfg_sent no marcado\n");
        return -1;
    }
    puts("OK: SCAN_CFG v5 bcast=0 con ADD_STA v12 vía LONG_GROUP (política Linux)");
    return 0;
}

int main(void)
{
    if (check_scan_cfg_bcast_ax200() != 0)
        return 1;
    if (check_v17_layout() != 0)
        return 1;
    if (check_versions() != 0)
        return 1;
    if (check_ext_nvm_table() != 0)
        return 1;
    if (check_nvm_channels() != 0)
        return 1;
    if (check_phy_bands() != 0)
        return 1;
    if (check_mac_csr() != 0)
        return 1;
    if (check_scan_states() != 0)
        return 1;
    return 0;
}
